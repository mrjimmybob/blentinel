//! The `tls` check (M5-01, D-77): the certificate a TLS server presents.
//!
//! The probe makes a real TLS handshake (rustls, as for its hub) and reads
//! the server's certificate. It reports the days until it expires; the hub's
//! thresholds decide when that is a Warning (proposal 0006 §1.5). The check
//! itself fails when the certificate has expired, or when a client wouldn't
//! accept it (an authority nobody knows, another name) unless the check says
//! not to verify that.
//!
//! The authorities trusted are Mozilla's, compiled in
//! ([`crate::tls::public_roots`]): never the machine's certificate store (FP-2).

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use common::checks::{Metric, metric};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use super::{Outcome, TCP_TIMEOUT};
use crate::tls::{provider, public_roots};

/// How long the TLS handshake may take once the port is open.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

const SECONDS_PER_DAY: f64 = 86_400.0;

/// Decides whether a certificate is one a client would accept.
pub type Trust = Arc<WebPkiServerVerifier>;

/// Trust in certificates issued under `roots`.
fn trust_in(roots: RootCertStore) -> Result<Trust, rustls::client::VerifierBuilderError> {
    WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider()).build()
}

/// What browsers trust: Mozilla's list of authorities, compiled in.
pub fn public_trust() -> Trust {
    // Building it fails only without any authority, and the list is compiled in.
    trust_in(public_roots()).expect("the compiled-in list of authorities isn't empty")
}

/// Lets every handshake finish, so the certificate can be read whatever is
/// wrong with it, and keeps why `trust` would have refused it. The server
/// must still prove it holds the certificate's key.
#[derive(Debug)]
struct Recording {
    trust: Trust,
    refused: Mutex<Option<rustls::Error>>,
}

impl ServerCertVerifier for Recording {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let verdict = self.trust.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        );
        if let Err(e) = verdict {
            *self.refused.lock().unwrap_or_else(PoisonError::into_inner) = Some(e);
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.trust.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.trust.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.trust.supported_verify_schemes()
    }
}

/// Connect to `host:port`, read its certificate and judge it.
pub async fn check(host: &str, port: u16, verify: bool, trust: &Trust) -> Outcome {
    let Ok(name) = ServerName::try_from(host.to_string()) else {
        return Outcome::down("The host isn't a name or an IP address");
    };
    let recording = Arc::new(Recording {
        trust: Arc::clone(trust),
        refused: Mutex::new(None),
    });
    let config = match ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
    {
        Ok(builder) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::clone(&recording) as Arc<dyn ServerCertVerifier>)
            .with_no_client_auth(),
        Err(e) => return Outcome::down(format!("TLS setup failed: {e}")),
    };

    let start = Instant::now();
    let socket = match tokio::time::timeout(TCP_TIMEOUT, TcpStream::connect((host, port))).await {
        Ok(Ok(socket)) => socket,
        Ok(Err(e)) => return Outcome::down(format!("Connection failed: {e}")),
        Err(_) => return Outcome::down("Connection timeout"),
    };
    let connector = TlsConnector::from(Arc::new(config));
    let tls = match tokio::time::timeout(HANDSHAKE_TIMEOUT, connector.connect(name, socket)).await {
        Ok(Ok(tls)) => tls,
        Ok(Err(e)) => return Outcome::down(format!("TLS handshake failed: {e}")),
        Err(_) => return Outcome::down("TLS handshake timeout"),
    };
    let elapsed = start.elapsed();

    let (_, connection) = tls.get_ref();
    let Some(certificate) = connection.peer_certificates().and_then(|c| c.first()) else {
        return Outcome::down("The server sent no certificate");
    };
    let refused = recording
        .refused
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .filter(|_| verify);
    judge(certificate.as_ref(), refused.as_ref(), host, Utc::now()).with_latency(elapsed)
}

/// What a certificate is worth at `now`: Down if it has expired or was
/// `refused`, else Up; with the days it has left either way.
fn judge(
    certificate_der: &[u8],
    refused: Option<&rustls::Error>,
    host: &str,
    now: DateTime<Utc>,
) -> Outcome {
    let Ok((_, certificate)) = x509_parser::parse_x509_certificate(certificate_der) else {
        return Outcome::down("The server's certificate can't be read");
    };
    let Some(expires) = DateTime::from_timestamp(certificate.validity().not_after.timestamp(), 0)
    else {
        return Outcome::down("The certificate's expiry date is out of range");
    };
    let seconds_left = (expires - now).num_seconds();
    let days_left = seconds_left as f64 / SECONDS_PER_DAY;
    let metrics = vec![Metric::new(metric::DAYS_LEFT, days_left, metric::DAYS)];
    let date = expires.format("%Y-%m-%d");

    if seconds_left <= 0 {
        let ago = days(-days_left);
        return Outcome::down(format!("Certificate expired {ago} ago ({date})"))
            .with_metrics(metrics);
    }
    if let Some(error) = refused {
        return Outcome::down(format!(
            "Certificate not trusted: {} (expires {date})",
            why_refused(error, host)
        ))
        .with_metrics(metrics);
    }
    let issuer = certificate
        .issuer()
        .iter_common_name()
        .chain(certificate.issuer().iter_organization())
        .find_map(|name| name.as_str().ok())
        .map(|name| format!(", issued by {name}"))
        .unwrap_or_default();
    Outcome::up(format!(
        "Certificate expires in {} ({date}){issuer}",
        days(days_left)
    ))
    .with_metrics(metrics)
}

/// `0 days`, `1 day`, `43 days`: whole days, rounded down.
fn days(days: f64) -> String {
    match days.floor() as i64 {
        1 => "1 day".to_string(),
        n => format!("{n} days"),
    }
}

/// Why a client wouldn't accept the certificate, for people.
fn why_refused(error: &rustls::Error, host: &str) -> String {
    match error {
        rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer) => {
            "its issuer isn't a public authority (self-signed, or a private one?)".to_string()
        }
        rustls::Error::InvalidCertificate(
            CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
        ) => format!("it isn't for {host}"),
        rustls::Error::InvalidCertificate(
            CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. },
        ) => "it isn't valid yet".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::Health;
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};

    /// The address the test servers listen on, and the name in their certificates.
    const HOST: &str = "127.0.0.1";

    /// A private authority and trust in it.
    struct Authority {
        certificate: rcgen::Certificate,
        key: KeyPair,
    }

    impl Authority {
        fn new() -> Self {
            let key = KeyPair::generate().unwrap();
            let mut params = CertificateParams::new(Vec::new()).unwrap();
            params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
            params
                .distinguished_name
                .push(rcgen::DnType::CommonName, "Test Authority");
            Self {
                certificate: params.self_signed(&key).unwrap(),
                key,
            }
        }

        fn trust(&self) -> Trust {
            let mut roots = RootCertStore::empty();
            roots.add(self.certificate.der().clone()).unwrap();
            trust_in(roots).unwrap()
        }

        /// A certificate for `name` that expires on `expires` (year, month, day) at 00:00 UTC.
        fn issue(&self, name: &str, expires: (i32, u8, u8)) -> (CertificateDer<'static>, KeyPair) {
            let key = KeyPair::generate().unwrap();
            let mut params = CertificateParams::new(vec![name.to_string()]).unwrap();
            params.not_after = rcgen::date_time_ymd(expires.0, expires.1, expires.2);
            let certificate = params
                .signed_by(&key, &self.certificate, &self.key)
                .unwrap();
            (certificate.der().clone(), key)
        }
    }

    /// A TLS server with this certificate; returns its port.
    async fn server(certificate: CertificateDer<'static>, key: KeyPair) -> u16 {
        let key = rustls::pki_types::PrivateKeyDer::try_from(key.serialize_der()).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], key)
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind((HOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let _ = acceptor.accept(socket).await;
                });
            }
        });
        port
    }

    fn at(date: &str) -> DateTime<Utc> {
        format!("{date}T00:00:00Z").parse().unwrap()
    }

    fn days_left(outcome: &Outcome) -> f64 {
        outcome.metrics[0].value
    }

    #[tokio::test]
    async fn a_trusted_certificate_is_up_with_its_days_left() {
        let authority = Authority::new();
        let (certificate, key) = authority.issue(HOST, (2099, 1, 1));
        let port = server(certificate, key).await;

        let outcome = check(HOST, port, true, &authority.trust()).await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert!(
            outcome
                .message
                .contains("(2099-01-01), issued by Test Authority")
        );
        assert_eq!(outcome.metrics[0].key, metric::DAYS_LEFT);
        assert!(days_left(&outcome) > 365.0);
        assert!(outcome.latency_ms.is_some());
    }

    #[tokio::test]
    async fn an_untrusted_certificate_is_down_unless_the_check_does_not_verify() {
        let authority = Authority::new();
        let (certificate, key) = authority.issue(HOST, (2099, 1, 1));
        let port = server(certificate, key).await;
        let public = public_trust();

        let refused = check(HOST, port, true, &public).await;
        assert_eq!(refused.status, Health::Down);
        assert!(
            refused.message.contains("isn't a public authority"),
            "{}",
            refused.message
        );
        assert!(days_left(&refused) > 365.0, "the days are reported anyway");

        let accepted = check(HOST, port, false, &public).await;
        assert_eq!(accepted.status, Health::Up, "{}", accepted.message);
    }

    #[tokio::test]
    async fn a_certificate_for_another_name_is_refused() {
        let authority = Authority::new();
        let (certificate, key) = authority.issue("other.example", (2099, 1, 1));
        let port = server(certificate, key).await;

        let outcome = check(HOST, port, true, &authority.trust()).await;
        assert_eq!(outcome.status, Health::Down);
        assert!(
            outcome.message.contains("it isn't for 127.0.0.1"),
            "{}",
            outcome.message
        );
    }

    #[tokio::test]
    async fn a_port_that_is_closed_or_does_not_speak_tls_is_down() {
        let listener = tokio::net::TcpListener::bind((HOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let trust = public_trust();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                use tokio::io::AsyncWriteExt;
                let _ = socket.write_all(b"220 mail.example ESMTP\r\n").await;
            }
        });
        let plain = check(HOST, port, true, &trust).await;
        assert_eq!(plain.status, Health::Down);
        assert!(
            plain.message.starts_with("TLS handshake"),
            "{}",
            plain.message
        );
        assert!(plain.metrics.is_empty());

        let free = tokio::net::TcpListener::bind((HOST, 0)).await.unwrap();
        let closed = free.local_addr().unwrap().port();
        drop(free);
        let outcome = check(HOST, closed, true, &trust).await;
        assert_eq!(outcome.status, Health::Down);
        assert!(
            outcome.message.starts_with("Connection"),
            "{}",
            outcome.message
        );
    }

    #[test]
    fn days_left_are_counted_to_the_expiry_date() {
        let authority = Authority::new();
        let (certificate, _) = authority.issue(HOST, (2030, 1, 31));

        let soon = judge(&certificate, None, HOST, at("2030-01-21"));
        assert_eq!(soon.status, Health::Up);
        assert_eq!(days_left(&soon), 10.0);
        assert!(
            soon.message
                .starts_with("Certificate expires in 10 days (2030-01-31)")
        );

        let tomorrow = judge(
            &certificate,
            None,
            HOST,
            at("2030-01-29") + chrono::Duration::hours(12),
        );
        assert_eq!(days_left(&tomorrow), 1.5);
        assert!(
            tomorrow.message.contains("in 1 day ("),
            "{}",
            tomorrow.message
        );
    }

    #[test]
    fn an_expired_certificate_is_down_whether_verified_or_not() {
        let authority = Authority::new();
        let (certificate, _) = authority.issue(HOST, (2030, 1, 31));

        let expired = judge(&certificate, None, HOST, at("2030-02-03"));
        assert_eq!(expired.status, Health::Down);
        assert_eq!(
            expired.message,
            "Certificate expired 3 days ago (2030-01-31)"
        );
        assert_eq!(days_left(&expired), -3.0);

        let unreadable = judge(b"not a certificate", None, HOST, at("2030-01-01"));
        assert_eq!(unreadable.status, Health::Down);
        assert!(unreadable.metrics.is_empty());
    }
}
