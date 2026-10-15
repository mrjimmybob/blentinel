//! Report delivery with an offline buffer (FP-4), bounded so it can't grow forever.
//!
//! Rules:
//! - A report is stored **only while it is undelivered**. Once the hub has it
//!   (200, or 409 "already have it"), it is deleted.
//! - Delivery is **in order**, oldest first. While a backlog exists, new
//!   reports join the back of the queue instead of jumping ahead: the hub only
//!   accepts reports newer than the last one it stored (replay protection).
//! - A report the hub **permanently rejects** (e.g. 403 unknown probe, 422 too
//!   old) is dropped and logged, never retried. Only transient failures
//!   (network error, timeout, 5xx, 408, 429) keep a report queued.
//! - When the hub says, in a signed answer, that this probe is **revoked**,
//!   delivery stops; the caller decides what happens next (D-48).
//! - The buffer is capped by age ([`MAX_BUFFERED_REPORT_AGE_SECS`], the same
//!   limit the hub enforces) and by count ([`MAX_BUFFERED_REPORTS`]).

use async_trait::async_trait;
use common::ack::ReportAck;
use common::protocol::MAX_BUFFERED_REPORT_AGE_SECS;
use tracing::{error, info, warn};

use crate::storage::BlackBox;

/// Hard cap on buffered reports (~2 weeks at a 60 s interval, well past the age limit).
pub const MAX_BUFFERED_REPORTS: i64 = 20_000;

/// Most queued reports sent per cycle, so a long backlog can't stall monitoring.
pub const FLUSH_BATCH: i64 = 500;

/// What happened to one delivery attempt.
#[derive(Debug, Clone, PartialEq)]
pub enum Delivery {
    /// The hub has the report (stored now, or already had it), with its
    /// signed answer when it sent one.
    Delivered(Option<Box<ReportAck>>),
    /// The hub revoked this probe (a signed answer): stop sending.
    Revoked,
    /// The hub refused it for good; resending can't help.
    Rejected(String),
    /// Temporary failure; try again later.
    Retry(String),
}

/// Something that can deliver a sealed report to the hub.
#[async_trait]
pub trait ReportSender: Send + Sync {
    async fn deliver(&self, sealed_report: &[u8]) -> Delivery;
}

/// Counts from one [`Outbox::submit`] call, for logging and tests.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SubmitSummary {
    pub delivered: usize,
    pub rejected: usize,
    pub pruned: u64,
    pub queued: i64,
    /// The hub's answer to the newest delivered report.
    pub last_ack: Option<Box<ReportAck>>,
    /// The hub revoked this probe; nothing more was sent.
    pub revoked: bool,
    /// Why the last report that didn't get through didn't.
    pub problem: Option<String>,
}

pub struct Outbox {
    store: BlackBox,
}

impl Outbox {
    pub fn new(store: BlackBox) -> Self {
        Self { store }
    }

    /// Forget every queued report.
    pub async fn clear(&self) -> anyhow::Result<()> {
        self.store.clear().await
    }

    /// How many reports wait for the hub.
    pub async fn queued(&self) -> anyhow::Result<i64> {
        self.store.count().await
    }

    /// The hub switched keys (D-61): keep the queued reports for it.
    pub async fn set_hub_key(&self, hub_public_key: &str) -> anyhow::Result<()> {
        self.store.set_hub_key(hub_public_key).await
    }

    /// Deliver `sealed_report`, buffering it if the hub is unreachable, and
    /// flush any backlog in order.
    pub async fn submit(
        &self,
        sender: &dyn ReportSender,
        sealed_report: Vec<u8>,
    ) -> anyhow::Result<SubmitSummary> {
        let mut summary = SubmitSummary {
            pruned: self
                .store
                .prune(MAX_BUFFERED_REPORT_AGE_SECS, MAX_BUFFERED_REPORTS)
                .await?,
            ..Default::default()
        };
        if summary.pruned > 0 {
            warn!(
                dropped = summary.pruned,
                "Dropped buffered reports beyond the age/size limit"
            );
        }

        if self.store.count().await? == 0 {
            // Fast path: nothing waiting, send directly. Nothing is written to disk
            // unless delivery fails transiently.
            match sender.deliver(&sealed_report).await {
                Delivery::Delivered(ack) => {
                    summary.delivered += 1;
                    summary.last_ack = ack.or(summary.last_ack.take());
                }
                Delivery::Revoked => summary.revoked = true,
                Delivery::Rejected(reason) => {
                    summary.rejected += 1;
                    error!(%reason, "Hub rejected the report permanently; it will not be retried");
                    summary.problem = Some(format!("The hub refused a report: {reason}"));
                }
                Delivery::Retry(reason) => {
                    warn!(%reason, "Hub unreachable, buffering report");
                    summary.problem = Some(format!("Can't reach the hub: {reason}"));
                    self.store.queue_report(&sealed_report).await?;
                }
            }
        } else {
            // Backlog exists: preserve order by joining the back of the queue.
            self.store.queue_report(&sealed_report).await?;
            self.flush(sender, &mut summary).await?;
        }

        summary.queued = self.store.count().await?;
        Ok(summary)
    }

    /// Send up to [`FLUSH_BATCH`] queued reports, oldest first; stop at the
    /// first transient failure so order is kept.
    async fn flush(
        &self,
        sender: &dyn ReportSender,
        summary: &mut SubmitSummary,
    ) -> anyhow::Result<()> {
        for (id, payload) in self.store.oldest(FLUSH_BATCH).await? {
            match sender.deliver(&payload).await {
                Delivery::Delivered(ack) => {
                    self.store.delete_report(id).await?;
                    summary.delivered += 1;
                    summary.last_ack = ack.or(summary.last_ack.take());
                }
                Delivery::Revoked => {
                    self.store.delete_report(id).await?;
                    summary.revoked = true;
                    break;
                }
                Delivery::Rejected(reason) => {
                    self.store.delete_report(id).await?;
                    summary.rejected += 1;
                    error!(%reason, "Hub rejected a buffered report permanently; dropped");
                    summary.problem = Some(format!("The hub refused a report: {reason}"));
                }
                Delivery::Retry(reason) => {
                    warn!(%reason, "Hub unreachable while flushing buffered reports; will retry");
                    summary.problem = Some(format!("Can't reach the hub: {reason}"));
                    break;
                }
            }
        }
        if summary.delivered > 0 {
            info!(delivered = summary.delivered, "Delivered buffered reports");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// Scripted sender: returns the given outcomes in order (then `Delivered`),
    /// and records every payload it was asked to send.
    #[derive(Default)]
    struct FakeHub {
        script: Mutex<VecDeque<Delivery>>,
        sent: Mutex<Vec<Vec<u8>>>,
    }

    impl FakeHub {
        fn scripted(outcomes: impl IntoIterator<Item = Delivery>) -> Self {
            Self {
                script: Mutex::new(outcomes.into_iter().collect()),
                ..Default::default()
            }
        }
        fn sent(&self) -> Vec<Vec<u8>> {
            self.sent.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ReportSender for FakeHub {
        async fn deliver(&self, sealed_report: &[u8]) -> Delivery {
            self.sent.lock().unwrap().push(sealed_report.to_vec());
            self.script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Delivery::Delivered(None))
        }
    }

    fn down() -> Delivery {
        Delivery::Retry("connection refused".into())
    }

    async fn outbox() -> (Outbox, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = BlackBox::open(&dir.path().join("blackbox.db"), "probe", "hub")
            .await
            .unwrap();
        (Outbox::new(store), dir)
    }

    #[tokio::test]
    async fn a_revocation_stops_delivery_and_leaves_the_rest_queued() {
        let (outbox, _dir) = outbox().await;
        let hub = FakeHub::scripted([down(), down(), Delivery::Revoked]);
        outbox.submit(&hub, b"r1".to_vec()).await.unwrap();
        outbox.submit(&hub, b"r2".to_vec()).await.unwrap();
        let summary = outbox.submit(&hub, b"r3".to_vec()).await.unwrap();

        assert!(summary.revoked);
        assert_eq!(summary.delivered, 0);
        assert_eq!(summary.queued, 2, "r1 dropped, r2 and r3 not sent");
        assert_eq!(hub.sent().len(), 3, "r1, r2 (down), r1 (revoked)");
    }

    #[tokio::test]
    async fn a_failed_delivery_says_why_and_a_good_one_says_nothing() {
        let (outbox, _dir) = outbox().await;
        let hub = FakeHub::scripted([down(), Delivery::Rejected("too old".into())]);

        let summary = outbox.submit(&hub, b"r1".to_vec()).await.unwrap();
        assert_eq!(
            summary.problem.as_deref(),
            Some("Can't reach the hub: connection refused")
        );
        assert_eq!(outbox.queued().await.unwrap(), 1);

        let summary = outbox.submit(&hub, b"r2".to_vec()).await.unwrap();
        assert_eq!(
            summary.problem.as_deref(),
            Some("The hub refused a report: too old")
        );
        let summary = outbox.submit(&hub, b"r3".to_vec()).await.unwrap();
        assert_eq!(summary.problem, None);
        assert_eq!(outbox.queued().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn delivered_report_is_never_stored() {
        let (outbox, _dir) = outbox().await;
        let hub = FakeHub::default();

        let summary = outbox.submit(&hub, b"r1".to_vec()).await.unwrap();

        assert_eq!(summary.delivered, 1);
        assert_eq!(summary.queued, 0);
    }

    #[tokio::test]
    async fn unreachable_hub_buffers_the_report() {
        let (outbox, _dir) = outbox().await;
        let hub = FakeHub::scripted([down()]);

        let summary = outbox.submit(&hub, b"r1".to_vec()).await.unwrap();

        assert_eq!(summary.queued, 1);
    }

    #[tokio::test]
    async fn backlog_is_delivered_oldest_first_then_deleted() {
        let (outbox, _dir) = outbox().await;
        let hub = FakeHub::scripted([down(), down()]);
        outbox.submit(&hub, b"r1".to_vec()).await.unwrap(); // hub down: buffered
        outbox.submit(&hub, b"r2".to_vec()).await.unwrap(); // still down: r1 fails, r2 queued behind

        let summary = outbox.submit(&hub, b"r3".to_vec()).await.unwrap(); // hub back

        assert_eq!(summary.delivered, 3);
        assert_eq!(summary.queued, 0, "delivered reports must be deleted");
        let sent = hub.sent();
        assert_eq!(
            &sent[sent.len() - 3..],
            [b"r1".to_vec(), b"r2".to_vec(), b"r3".to_vec()]
        );
    }

    #[tokio::test]
    async fn new_report_never_jumps_ahead_of_the_backlog() {
        let (outbox, _dir) = outbox().await;
        let hub = FakeHub::scripted([down(), down()]);
        outbox.submit(&hub, b"r1".to_vec()).await.unwrap();

        let summary = outbox.submit(&hub, b"r2".to_vec()).await.unwrap();

        // The one attempt this cycle was for r1 (oldest), not r2.
        assert_eq!(hub.sent().last().unwrap(), b"r1");
        assert_eq!(summary.queued, 2);
    }

    #[tokio::test]
    async fn permanently_rejected_report_is_dropped_not_buffered() {
        // e.g. the probe isn't registered yet (403): buffering would grow forever.
        let (outbox, _dir) = outbox().await;
        let hub = FakeHub::scripted([Delivery::Rejected("403".into())]);

        let summary = outbox.submit(&hub, b"r1".to_vec()).await.unwrap();

        assert_eq!(summary.rejected, 1);
        assert_eq!(summary.queued, 0);
    }

    #[tokio::test]
    async fn rejected_buffered_report_does_not_block_the_rest() {
        let (outbox, _dir) = outbox().await;
        let hub = FakeHub::scripted([down(), down(), Delivery::Rejected("422".into())]);
        outbox.submit(&hub, b"r1".to_vec()).await.unwrap();
        outbox.submit(&hub, b"r2".to_vec()).await.unwrap();

        let summary = outbox.submit(&hub, b"r3".to_vec()).await.unwrap();

        assert_eq!(summary.rejected, 1, "r1 dropped");
        assert_eq!(summary.delivered, 2, "r2 and r3 still delivered");
        assert_eq!(summary.queued, 0);
    }

    #[tokio::test]
    async fn reports_older_than_the_hub_accepts_are_pruned() {
        let (outbox, _dir) = outbox().await;
        outbox
            .store
            .queue_report_created_at(b"ancient", "2000-01-01 00:00:00")
            .await
            .unwrap();
        let hub = FakeHub::scripted([down()]);

        let summary = outbox.submit(&hub, b"fresh".to_vec()).await.unwrap();

        assert_eq!(summary.pruned, 1);
        assert_eq!(summary.queued, 1, "only the fresh report is buffered");
    }

    #[tokio::test]
    async fn buffer_is_capped_by_count_dropping_the_oldest() {
        let (outbox, _dir) = outbox().await;
        for i in 0..5 {
            outbox
                .store
                .queue_report(format!("r{i}").as_bytes())
                .await
                .unwrap();
        }

        let dropped = outbox
            .store
            .prune(MAX_BUFFERED_REPORT_AGE_SECS, 3)
            .await
            .unwrap();

        assert_eq!(dropped, 2);
        let left: Vec<Vec<u8>> = outbox
            .store
            .oldest(10)
            .await
            .unwrap()
            .into_iter()
            .map(|(_, p)| p)
            .collect();
        assert_eq!(left, [b"r2".to_vec(), b"r3".to_vec(), b"r4".to_vec()]);
    }
}
