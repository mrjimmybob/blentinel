//! One alert, whatever channel carries it (D-66, proposal 0005 §2.7): what
//! happened, where (company › site › probe › check), when, why, and a link.
//! Each channel lays out the same [`Notification`] its own way.

use chrono::{DateTime, Utc};

use crate::model::AlertKind;

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
}

impl Notification {
    /// The headline word: `DOWN`, `WARNING`, `RECOVERED`, `EXPIRED`, `TEST`.
    pub fn word(&self) -> &'static str {
        match self.test {
            true => "TEST",
            false => self.kind.word(),
        }
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
        match &self.check {
            Some(check) => format!(
                "[BLENTINEL][{}][{}] {} {}",
                self.company_id,
                self.site_id,
                check.name,
                self.word()
            ),
            None => format!(
                "[BLENTINEL][{}] PROBE {} {}",
                self.company_id,
                self.probe_name,
                self.word()
            ),
        }
    }

    /// The link to the probe's page, if the hub's address is set.
    pub fn link(&self, hub_url: &str) -> Option<String> {
        let base = hub_url.trim().trim_end_matches('/');
        (!base.is_empty()).then(|| format!("{base}/probes/{}", self.probe_id))
    }

    /// The alert as plain text (e-mail, Telegram), with `link` if any.
    pub fn text(&self, link: Option<&str>) -> String {
        let mut lines = vec![
            format!("{} {}", self.about(), self.word()),
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
}
