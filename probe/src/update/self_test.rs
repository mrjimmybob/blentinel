//! `probe --self-test`: what a downloaded version must pass before the
//! running one makes way for it (proposal 0012 §5.4). It runs from the
//! probe's folder like the probe itself, reads its files and changes nothing.
//!
//! Prints [`binary::self_test_banner`] first, then exits `0` if this binary
//! can run the probe there, [`binary::SELF_TEST_HUB_UNREACHABLE`] if all
//! that failed is reaching the hub (try again later), `1` otherwise.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};

use super::{VERSION, binary};
use crate::{enrollment, hub_link::HubLink, identity, storage};

/// How long the hub has to answer.
const HUB_TIMEOUT: Duration = Duration::from_secs(20);

pub async fn run() -> i32 {
    println!("{}", binary::self_test_banner(VERSION));
    let dir = crate::config::get_base_dir();
    match check(&dir).await {
        Ok(()) => {
            println!("self-test: ok");
            0
        }
        Err((code, e)) => {
            println!("self-test: {e:#}");
            code
        }
    }
}

async fn check(dir: &Path) -> Result<(), (i32, anyhow::Error)> {
    let broken = |e: anyhow::Error| (1, e);
    identity::probe_id_in(dir)
        .map_err(broken)?
        .ok_or_else(|| broken(anyhow!("no identity in {}", dir.display())))?;
    let enrolled = enrollment::load(dir)
        .map_err(broken)?
        .ok_or_else(|| broken(anyhow!("not enrolled")))?;
    let link = HubLink::new(&enrolled).map_err(broken)?;
    storage::BlackBox::pending_at(&dir.join(storage::FILE_NAME))
        .await
        .context("can't read the report buffer")
        .map_err(broken)?;
    tokio::time::timeout(HUB_TIMEOUT, link.transport.hub_date())
        .await
        .map_err(|_| {
            anyhow!(
                "no answer from {} in {} s",
                enrolled.hub_url,
                HUB_TIMEOUT.as_secs()
            )
        })
        .and_then(|reached| reached.with_context(|| format!("can't reach {}", enrolled.hub_url)))
        .map_err(|e| (binary::SELF_TEST_HUB_UNREACHABLE, e))?;
    Ok(())
}
