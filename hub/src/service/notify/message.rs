//! One alert, whatever channel carries it (D-66, proposal 0005 §2.7): what
//! happened, where (company › site › probe › check), when, why, how long it
//! lasted (a recovery), and a link to its incident (D-67). Each channel lays
//! out the same [`Notification`] its own way.

use chrono::{DateTime, Utc};

use crate::model::{AlertKind, duration_text};

/// The incident an alert belongs to (D-67).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IncidentRef {
    pub id: i64,
    pub opened_at: DateTime<Utc>,
    /// It was escalated, as an alert of this kind: its recovery goes to the
    /// escalation's channels too (D-69).
    pub escalated: Option<AlertKind>,
}

/// An alert about an incident nobody acknowledged (D-69): to these channels,
/// not the routing rules'.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nudge {
    /// The `count`th reminder.
    Reminder { count: u32, channels: Vec<i64> },
    /// Not acknowledged for `after_minutes`.
    Escalation {
        after_minutes: u32,
        channels: Vec<i64>,
    },
}

impl Nudge {
    pub fn channels(&self) -> &[i64] {
        match self {
            Nudge::Reminder { channels, .. } | Nudge::Escalation { channels, .. } => channels,
        }
    }
}

/// The check an alert is about (`None` in a notification: the probe itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRef {
    pub id: String,
    pub name: String,
    /// `ping`, `disk`, …
    pub kind: String,
    pub target: String,
}

/// An alert, before a channel formats it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub kind: AlertKind,
    pub at: DateTime<Utc>,
    pub company_id: String,
    pub company_name: String,
    pub site_id: String,
    pub site_name: String,
    pub probe_id: String,
    pub probe_name: String,
    pub hostname: String,
    pub check: Option<CheckRef>,
    /// Why: a threshold's reason, the probe's message, how long it's silent.
    pub detail: String,
    /// "Send test": not a real alert.
    pub test: bool,
    /// Its incident; `None` for a test.
    pub incident: Option<IncidentRef>,
    /// A reminder or an escalation; `None`: the alert itself.
    pub nudge: Option<Nudge>,
}

impl Notification {
    /// What happened: `DOWN`, `WARNING`, `RECOVERED`, `EXPIRED`, `TEST`; a
    /// reminder `STILL DOWN`; an escalation `DOWN (escalated: not
    /// acknowledged for 1 h)`.
    pub fn headline(&self) -> String {
        let word = self.kind.word();
        match (&self.nudge, self.test) {
            (_, true) => "TEST".into(),
            (None, false) => word.into(),
            (Some(Nudge::Reminder { .. }), false) => format!("STILL {word}"),
            (Some(Nudge::Escalation { after_minutes, .. }), false) => format!(
                "{word} (escalated: not acknowledged for {})",
                duration_text(i64::from(*after_minutes) * 60)
            ),
        }
    }

    /// For a reminder or an escalation: how long it has lasted, and which
    /// reminder this is.
    pub fn open_for(&self) -> Option<String> {
        let (nudge, incident) = (self.nudge.as_ref()?, self.incident?);
        let lasted = duration_text((self.at - incident.opened_at).num_seconds());
        Some(match nudge {
            Nudge::Reminder { count, .. } => {
                format!("for {lasted}, not acknowledged (reminder {count})")
            }
            Nudge::Escalation { .. } => format!("for {lasted}, not acknowledged"),
        })
    }

    /// What it is about: the check's name, or `probe <name>`.
    pub fn about(&self) -> String {
        match &self.check {
            Some(check) => check.name.clone(),
            None => format!("probe {}", self.probe_name),
        }
    }

    /// `ACME Ltd › Head office › hq-probe › NAS`.
    pub fn place(&self) -> String {
        let mut parts = vec![
            self.company_name.as_str(),
            self.site_name.as_str(),
            self.probe_name.as_str(),
        ];
        if let Some(check) = &self.check {
            parts.push(&check.name);
        }
        parts.retain(|p| !p.is_empty());
        parts.join(" › ")
    }

    /// The subject line: `[BLENTINEL][acme][hq] NAS DOWN`; it always ends
    /// with [`Notification::word`].
    pub fn subject(&self) -> String {
        let tag = self.tag();
        match &self.check {
            Some(check) => format!(
                "[{tag}][{}][{}] {} {}",
                self.company_id,
                self.site_id,
                check.name,
                self.headline()
            ),
            None => format!(
                "[{tag}][{}] PROBE {} {}",
                self.company_id,
                self.probe_name,
                self.headline()
            ),
        }
    }

    /// `BLENTINEL #12` for an alert of incident 12, the same on every alert
    /// about it (its first, changes, reminders, escalation, recovery), so a
    /// PSA or a mail client can keep them in one ticket or thread (D-72);
    /// `BLENTINEL` without one (a test).
    pub fn tag(&self) -> String {
        match self.incident {
            Some(incident) => format!("BLENTINEL #{}", incident.id),
            None => "BLENTINEL".into(),
        }
    }

    /// The incident's state after this alert: `closed` for a recovery,
    /// else `open`; `None` without one.
    pub fn incident_state(&self) -> Option<&'static str> {
        self.incident.map(|_| match self.kind {
            AlertKind::Recovered => "closed",
            _ => "open",
        })
    }

    /// The link to its incident (else the probe's page), if the hub's
    /// address is set.
    pub fn link(&self, hub_url: &str) -> Option<String> {
        let base = hub_url.trim().trim_end_matches('/');
        (!base.is_empty()).then(|| match self.incident {
            Some(incident) => format!("{base}/incidents/{}", incident.id),
            None => format!("{base}/probes/{}", self.probe_id),
        })
    }

    /// How long the outage lasted: a recovery's, e.g. `1 h 5 min`.
    pub fn lasted(&self) -> Option<String> {
        match (self.kind, self.incident) {
            (AlertKind::Recovered, Some(incident)) => {
                Some(duration_text((self.at - incident.opened_at).num_seconds()))
            }
            _ => None,
        }
    }

    /// The alert as plain text (e-mail, Telegram), with `link` if any.
    pub fn text(&self, link: Option<&str>) -> String {
        let mut lines = vec![
            format!("{} {}", self.about(), self.headline()),
            String::new(),
            format!("Where:  {}", self.place()),
        ];
        if let Some(check) = &self.check {
            lines.push(format!("Check:  {} {}", check.kind, check.target));
        }
        if !self.hostname.is_empty() {
            lines.push(format!("Host:   {}", self.hostname));
        }
        lines.push(format!(
            "When:   {}",
            self.at.format("%Y-%m-%d %H:%M:%S UTC")
        ));
        if let Some(lasted) = self.lasted() {
            lines.push(format!("Lasted: {lasted}"));
        }
        if let Some(open) = self.open_for() {
            lines.push(format!("Open:   {open}"));
        }
        if let (Some(incident), Some(state)) = (self.incident, self.incident_state()) {
            lines.push(format!("Incident: #{} ({state})", incident.id));
        }
        if !self.detail.is_empty() {
            lines.push(format!("Why:    {}", self.detail));
        }
        if let Some(link) = link {
            lines.push(String::new());
            lines.push(link.to_string());
        }
        if self.test {
            lines.push(String::new());
            lines.push("This is a test from the hub's alerts page: nothing is wrong.".into());
        }
        lines.join("\n")
    }

    /// A made-up alert for "Send test".
    pub fn sample(at: DateTime<Utc>) -> Self {
        Self {
            kind: AlertKind::Down,
            at,
            company_id: "example".into(),
            company_name: "Example Ltd".into(),
            site_id: "hq".into(),
            site_name: "Head office".into(),
            probe_id: "test".into(),
            probe_name: "test-probe".into(),
            hostname: String::new(),
            check: Some(CheckRef {
                id: "test".into(),
                name: "Test check".into(),
                kind: "ping".into(),
                target: "192.0.2.1".into(),
            }),
            detail: "a test alert".into(),
            test: true,
            incident: None,
            nudge: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn down() -> Notification {
        Notification {
            test: false,
            hostname: "hq-pc".into(),
            probe_id: "p1".into(),
            ..Notification::sample(chrono::DateTime::from_timestamp(1_791_000_000, 0).unwrap())
        }
    }

    #[test]
    fn the_subject_ends_with_what_happened() {
        assert_eq!(down().subject(), "[BLENTINEL][example][hq] Test check DOWN");
        let expired = Notification {
            kind: AlertKind::ProbeExpired,
            check: None,
            ..down()
        };
        assert_eq!(
            expired.subject(),
            "[BLENTINEL][example] PROBE test-probe EXPIRED"
        );
        assert_eq!(
            Notification::sample(down().at).subject().rsplit(' ').next(),
            Some("TEST")
        );
    }

    #[test]
    fn the_text_says_what_where_when_why_and_links() {
        let link = down().link("https://hub.example.com/");
        assert_eq!(link.as_deref(), Some("https://hub.example.com/probes/p1"));
        assert_eq!(down().link(""), None);
        let text = down().text(link.as_deref());
        assert!(text.starts_with("Test check DOWN\n"), "{text}");
        assert!(text.contains("Where:  Example Ltd › Head office › test-probe › Test check"));
        assert!(text.contains("Check:  ping 192.0.2.1"));
        assert!(text.contains("Why:    a test alert"));
        assert!(text.ends_with("https://hub.example.com/probes/p1"));
    }

    #[test]
    fn an_alert_links_to_its_incident_and_a_recovery_says_how_long() {
        let incident = IncidentRef {
            id: 7,
            opened_at: down().at - chrono::Duration::minutes(65),
            escalated: None,
        };
        let down = Notification {
            incident: Some(incident),
            ..down()
        };
        assert_eq!(
            down.link("https://hub.example.com").as_deref(),
            Some("https://hub.example.com/incidents/7")
        );
        assert_eq!(down.lasted(), None, "only a recovery has lasted");
        // One tag on every alert of the incident, for a PSA's ticket (D-72).
        assert_eq!(
            down.subject(),
            "[BLENTINEL #7][example][hq] Test check DOWN"
        );
        assert_eq!(down.incident_state(), Some("open"));
        assert!(down.text(None).contains("Incident: #7 (open)"));

        let recovered = Notification {
            kind: AlertKind::Recovered,
            ..down
        };
        assert_eq!(recovered.lasted().as_deref(), Some("1 h 5 min"));
        assert_eq!(
            recovered.subject(),
            "[BLENTINEL #7][example][hq] Test check RECOVERED"
        );
        assert_eq!(recovered.incident_state(), Some("closed"));
        assert!(recovered.text(None).contains("Lasted: 1 h 5 min"));
    }
}
