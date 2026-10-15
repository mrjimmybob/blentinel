//! The probe's link to its hub: where it is, the key reports are sealed for,
//! the keys its answers are signed with, and the certificates it trusts, all
//! from `enrollment.json` (D-47, D-48), with the next ones while the hub
//! rotates them (D-61). The file
//! is read again every cycle, so enrolling again (a new code, or a moved hub)
//! applies while the probe runs (as config changes do, D-38).

use anyhow::Result;
use ed25519_dalek::SigningKey;
use tracing::{error, info};

use crate::enrollment::Enrollment;
use crate::transport::{HubSender, HubTransport};

pub struct HubLink {
    /// The enrollment this link was built from, to notice when it changes.
    enrollment: Enrollment,
    pub transport: HubTransport,
    pub hub_key: [u8; 32],
    /// Current, then next (D-61).
    pub hub_signing_keys: Vec<[u8; 32]>,
}

impl HubLink {
    /// Build the link from an enrollment. Needs no network: the enrollment
    /// already holds the hub's key and certificate pin.
    pub fn new(enrollment: &Enrollment) -> Result<Self> {
        Ok(Self {
            transport: HubTransport::new(&enrollment.hub_url, enrollment.tls_pins()?)?,
            hub_key: enrollment.hub_key()?,
            hub_signing_keys: enrollment.signing_keys()?,
            enrollment: enrollment.clone(),
        })
    }

    pub fn enrollment(&self) -> &Enrollment {
        &self.enrollment
    }

    /// Delivers reports signed by `probe_key` and checks the hub's answers.
    pub fn sender<'a>(&'a self, probe_key: &'a SigningKey) -> HubSender<'a> {
        HubSender {
            transport: &self.transport,
            probe_key,
            hub_signing_keys: &self.hub_signing_keys,
        }
    }

    /// Follow a changed enrollment. If it can't be used, keep the current
    /// link and log why.
    pub fn follow(&mut self, enrollment: &Enrollment) {
        if *enrollment == self.enrollment {
            return;
        }
        match Self::new(enrollment) {
            Ok(link) => {
                info!(
                    hub_url = %enrollment.hub_url,
                    company = %enrollment.company_id,
                    "Enrollment changed; now reporting to this hub"
                );
                *self = link;
            }
            Err(e) => error!(
                "Can't use the new enrollment, keeping the current hub ({}): {e:#}",
                self.enrollment.hub_url
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enrollment(url: &str, key: [u8; 32]) -> Enrollment {
        Enrollment {
            hub_url: url.into(),
            hub_public_key: hex::encode(key),
            hub_signing_key: hex::encode([3; 32]),
            tls_spki_sha256: None,
            next: None,
            probe_name: "pc".into(),
            company_id: "acme".into(),
            company_name: "ACME".into(),
            site_id: "hq".into(),
            site_name: "HQ".into(),
            enrolled_at: "2026-09-28T00:00:00Z".into(),
        }
    }

    #[test]
    fn a_new_enrollment_is_followed() {
        let mut link = HubLink::new(&enrollment("http://127.0.0.1:9", [1; 32])).unwrap();
        link.follow(&enrollment("https://hub.example.com:3443", [2; 32]));
        assert_eq!(link.hub_key, [2; 32]);
        assert_eq!(link.enrollment().hub_url, "https://hub.example.com:3443");
    }

    #[test]
    fn an_unusable_enrollment_keeps_the_current_link() {
        let mut link = HubLink::new(&enrollment("http://127.0.0.1:9", [1; 32])).unwrap();

        let mut bad_key = enrollment("http://127.0.0.1:9", [2; 32]);
        bad_key.hub_public_key = "not hex".into();
        link.follow(&bad_key);
        link.follow(&enrollment("hub.example.com:3000", [2; 32])); // no scheme

        assert_eq!(link.hub_key, [1; 32]);
        assert_eq!(link.enrollment().hub_url, "http://127.0.0.1:9");
    }
}
