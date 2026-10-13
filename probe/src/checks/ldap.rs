//! The `ldap` check (M5-16g, D-99): does a directory server answer in LDAP,
//! and is it fit to serve? Active Directory's domain controllers, OpenLDAP,
//! FreeIPA and the like.
//!
//! An open port doesn't say so. The probe asks the server for what every
//! LDAP server tells anyone: its root entry (the rootDSE), which names the
//! server and what it holds. It never logs in (it has no account) and reads
//! nothing of the directory itself. A domain controller also says there
//! whether it is synchronised with its domain: one that isn't is Down.
//!
//! Where the connection is encrypted (StartTLS, or LDAPS from the first
//! byte) the certificate is judged as the `tls` check judges one: a domain
//! controller's certificate that ran out stops every program that logs in
//! through LDAPS.
//!
//! The few messages are written and read here, byte by byte (BER): a
//! request to start TLS, the search for the root entry, and the goodbye.

use std::time::{Duration, Instant};

use common::checks::{Health, LdapSecurity};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use super::certificate::{Handshake, Trust};
use super::database::{broken, said};
use super::{Outcome, TCP_TIMEOUT};

/// How long the server has for the whole exchange once the port is open.
const TALK_TIMEOUT: Duration = Duration::from_secs(10);
/// The longest message the probe reads: a root entry is a few kilobytes.
const MAX_MESSAGE_BYTES: usize = 256 * 1024;
/// The most messages of one answer.
const MAX_MESSAGES: usize = 16;
/// What the probe asks of the root entry: who the server is, what it holds,
/// and a domain controller's state.
const ATTRIBUTES: [&str; 7] = [
    "dnsHostName",
    "defaultNamingContext",
    "namingContexts",
    "isSynchronized",
    "isGlobalCatalogReady",
    "vendorName",
    "vendorVersion",
];
/// The name of the request to start TLS (RFC 4511).
const STARTTLS: &str = "1.3.6.1.4.1.1466.20037";
/// The numbers the probe gives its two requests.
const STARTTLS_ID: u8 = 1;
const SEARCH_ID: u8 = 2;

// BER tags: the universal ones, and LDAP's operations.
const INTEGER: u8 = 0x02;
const OCTETS: u8 = 0x04;
const ENUMERATED: u8 = 0x0a;
const BOOLEAN: u8 = 0x01;
const SEQUENCE: u8 = 0x30;
const SET: u8 = 0x31;
const UNBIND: u8 = 0x42;
const SEARCH: u8 = 0x63;
const ENTRY: u8 = 0x64;
const DONE: u8 = 0x65;
const EXTENDED: u8 = 0x77;
const EXTENDED_ANSWER: u8 = 0x78;
/// In a search: "has this attribute"; in an extended request: its name.
const PRESENT: u8 = 0x87;
const REQUEST_NAME: u8 = 0x80;

/// Result codes that say the server can't serve now.
const BUSY: u32 = 51;
const UNAVAILABLE: u32 = 52;
const OTHER: u32 = 80;

/// Why a server isn't one that serves a directory now.
#[derive(Debug, PartialEq)]
enum Trouble {
    /// No answer in LDAP (something else listens there, or it hung up).
    Silent(String),
    /// It answers in LDAP, but: why it isn't fit.
    But(String),
}

impl From<String> for Trouble {
    fn from(why: String) -> Self {
        Trouble::Silent(why)
    }
}

fn not_ldap() -> String {
    "its answer isn't LDAP's".to_string()
}

/// One element: its tag, its length, its content.
fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut element = vec![tag];
    match content.len() {
        short if short < 0x80 => element.push(short as u8),
        byte if byte <= 0xff => element.extend([0x81, byte as u8]),
        long => element.extend([0x82, (long >> 8) as u8, long as u8]),
    }
    element.extend_from_slice(content);
    element
}

/// A message: its number and its operation.
fn message(id: u8, operation: Vec<u8>) -> Vec<u8> {
    tlv(SEQUENCE, &[tlv(INTEGER, &[id]), operation].concat())
}

/// "Start TLS on this connection."
fn starttls_request() -> Vec<u8> {
    message(
        STARTTLS_ID,
        tlv(EXTENDED, &tlv(REQUEST_NAME, STARTTLS.as_bytes())),
    )
}

/// "Your root entry, these attributes of it": the entry with the empty
/// name, itself only, whatever it is.
fn search_request() -> Vec<u8> {
    let wanted: Vec<u8> = ATTRIBUTES
        .iter()
        .flat_map(|name| tlv(OCTETS, name.as_bytes()))
        .collect();
    let search = [
        tlv(OCTETS, b""),
        // This entry only; aliases as they are; no limit of size or time.
        tlv(ENUMERATED, &[0]),
        tlv(ENUMERATED, &[0]),
        tlv(INTEGER, &[0]),
        tlv(INTEGER, &[0]),
        // Values too, not the attributes' names alone.
        tlv(BOOLEAN, &[0]),
        tlv(PRESENT, b"objectClass"),
        tlv(SEQUENCE, &wanted),
    ]
    .concat();
    message(SEARCH_ID, tlv(SEARCH, &search))
}

/// The goodbye: it has no answer.
fn unbind_request() -> Vec<u8> {
    message(SEARCH_ID + 1, tlv(UNBIND, &[]))
}

/// The next element of `bytes`: its tag, its content, and what follows it.
fn element(bytes: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = bytes.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (length, rest) = match first {
        short if short < 0x80 => (usize::from(short), rest),
        0x81..=0x84 => {
            let (digits, rest) = rest.split_at_checked(usize::from(first & 0x7f))?;
            (number(digits)? as usize, rest)
        }
        _ => return None,
    };
    let (content, rest) = rest.split_at_checked(length)?;
    Some((tag, content, rest))
}

/// A number of up to four bytes.
fn number(digits: &[u8]) -> Option<u32> {
    (1..=4).contains(&digits.len()).then(|| {
        digits
            .iter()
            .fold(0u32, |sum, digit| (sum << 8) | u32::from(*digit))
    })
}

/// An element with this tag, and what follows it.
fn tagged(bytes: &[u8], tag: u8) -> Option<(&[u8], &[u8])> {
    element(bytes)
        .filter(|(found, ..)| *found == tag)
        .map(|(_, content, rest)| (content, rest))
}

/// How a request ended: LDAP's result code and the server's words.
#[derive(Debug, Clone, Default, PartialEq)]
struct Verdict {
    code: u32,
    words: String,
}

impl Verdict {
    /// The code in words, with the server's own where it gave some.
    fn said(&self) -> String {
        let name = match self.code {
            0 => "success".to_string(),
            1 => "operations error".to_string(),
            2 => "protocol error".to_string(),
            7 => "authentication method not supported".to_string(),
            8 => "stronger authentication required".to_string(),
            13 => "an encrypted connection required".to_string(),
            48 => "inappropriate authentication".to_string(),
            49 => "invalid credentials".to_string(),
            50 => "insufficient access rights".to_string(),
            BUSY => "busy".to_string(),
            UNAVAILABLE => "unavailable".to_string(),
            53 => "unwilling to perform".to_string(),
            OTHER => "an internal error".to_string(),
            code => format!("result code {code}"),
        };
        match self.words.is_empty() {
            true => name,
            false => format!("{name}: {}", self.words),
        }
    }
}

/// What a server sent.
#[derive(Debug, PartialEq)]
enum Reply {
    /// An entry: its attributes, each with its values.
    Entry(Vec<(String, Vec<String>)>),
    /// The end of a search.
    Done(Verdict),
    /// The answer to an extended request, or a notice of the server's own.
    Extended(Verdict),
    /// Something the probe has no use for (a referral to another server).
    Other,
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .to_string()
}

fn verdict(result: &[u8]) -> Option<Verdict> {
    let (code, rest) = tagged(result, ENUMERATED)?;
    // The name it matched, then its words.
    let (_, rest) = tagged(rest, OCTETS)?;
    let (words, _) = tagged(rest, OCTETS)?;
    Some(Verdict {
        code: number(code)?,
        words: said(&text(words)),
    })
}

fn entry(body: &[u8]) -> Option<Vec<(String, Vec<String>)>> {
    let (_name, rest) = tagged(body, OCTETS)?;
    let (mut list, _) = tagged(rest, SEQUENCE)?;
    let mut attributes = Vec::new();
    while !list.is_empty() {
        let (attribute, rest) = tagged(list, SEQUENCE)?;
        list = rest;
        let (name, values) = tagged(attribute, OCTETS)?;
        let (mut set, _) = tagged(values, SET)?;
        let mut values = Vec::new();
        while !set.is_empty() {
            let (value, rest) = tagged(set, OCTETS)?;
            set = rest;
            values.push(text(value));
        }
        attributes.push((text(name), values));
    }
    Some(attributes)
}

/// A message's number and what it is, from the content of its envelope.
/// `None`: it isn't an LDAP message.
fn reply(body: &[u8]) -> Option<(u32, Reply)> {
    let (id, rest) = tagged(body, INTEGER)?;
    let (tag, operation, _) = element(rest)?;
    let reply = match tag {
        ENTRY => Reply::Entry(entry(operation)?),
        DONE => Reply::Done(verdict(operation)?),
        EXTENDED_ANSWER => Reply::Extended(verdict(operation)?),
        _ => Reply::Other,
    };
    Some((number(id)?, reply))
}

/// The next message: the content of its envelope.
async fn read_message<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Vec<u8>, String> {
    let mut head = [0u8; 2];
    stream.read_exact(&mut head).await.map_err(broken)?;
    if head[0] != SEQUENCE {
        return Err(not_ldap());
    }
    let length = match head[1] {
        short if short < 0x80 => usize::from(short),
        0x81..=0x84 => {
            let mut digits = [0u8; 4];
            let digits = &mut digits[..usize::from(head[1] & 0x7f)];
            stream.read_exact(digits).await.map_err(broken)?;
            number(digits).ok_or_else(not_ldap)? as usize
        }
        _ => return Err(not_ldap()),
    };
    if length > MAX_MESSAGE_BYTES {
        return Err(not_ldap());
    }
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).await.map_err(broken)?;
    Ok(body)
}

async fn send<S: AsyncWrite + Unpin>(stream: &mut S, request: &[u8]) -> Result<(), String> {
    stream.write_all(request).await.map_err(broken)?;
    stream.flush().await.map_err(broken)
}

/// Ask the server to start TLS on this connection: `Ok` when it agreed, and
/// TLS is next. Nothing is read beyond its answer, so nothing it might send
/// after can be taken for encrypted.
async fn ask_for_tls<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S) -> Result<(), Trouble> {
    send(stream, &starttls_request()).await?;
    let body = read_message(stream).await?;
    match reply(&body) {
        Some((_, Reply::Extended(verdict))) if verdict.code == 0 => Ok(()),
        Some((_, Reply::Extended(verdict) | Reply::Done(verdict))) => Err(Trouble::But(format!(
            "it doesn't start TLS (StartTLS: {})",
            verdict.said()
        ))),
        _ => Err(Trouble::Silent(not_ldap())),
    }
}

/// A server's root entry.
#[derive(Debug, Default, PartialEq)]
struct Root(Vec<(String, Vec<String>)>);

impl Root {
    /// An attribute's values; servers differ in how they write its name.
    fn all(&self, name: &str) -> &[String] {
        self.0
            .iter()
            .find(|(found, _)| found.eq_ignore_ascii_case(name))
            .map(|(_, values)| values.as_slice())
            .unwrap_or_default()
    }

    fn first(&self, name: &str) -> Option<&str> {
        self.all(name)
            .first()
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }

    fn is(&self, name: &str, value: &str) -> bool {
        self.first(name)
            .is_some_and(|found| found.eq_ignore_ascii_case(value))
    }
}

/// Ask for the root entry and say goodbye: the entry, if the server gave
/// one, and how the search ended.
async fn read_root<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
) -> Result<(Option<Root>, Verdict), Trouble> {
    send(stream, &search_request()).await?;
    let mut root = None;
    for _ in 0..MAX_MESSAGES {
        let body = read_message(stream).await?;
        match reply(&body).ok_or_else(not_ldap)? {
            (_, Reply::Entry(attributes)) => root = Some(Root(attributes)),
            (_, Reply::Done(verdict)) => {
                // Whether the goodbye arrives doesn't matter.
                let _ = send(stream, &unbind_request()).await;
                return Ok((root, verdict));
            }
            // Its own notice (message 0): it is hanging up.
            (0, Reply::Extended(verdict)) => {
                return Err(Trouble::But(format!(
                    "it closes the connection ({})",
                    verdict.said()
                )));
            }
            _ => {}
        }
    }
    Err(Trouble::Silent("its answer has no end".to_string()))
}

/// A domain's name from its entry's: `DC=corp,DC=example,DC=com` is
/// `corp.example.com`. Any other name stays as it is.
fn domain(context: &str) -> String {
    let labels: Option<Vec<&str>> = context
        .split(',')
        .map(|part| {
            let (kind, label) = part.trim().split_once('=')?;
            kind.trim()
                .eq_ignore_ascii_case("dc")
                .then_some(label.trim())
        })
        .collect();
    match labels {
        Some(labels) if !labels.is_empty() => labels.join("."),
        _ => context.to_string(),
    }
}

/// What the answer says of the server, for a message. `Err`: it says it
/// isn't fit to serve.
fn describe(root: Option<&Root>, done: &Verdict) -> Result<String, Trouble> {
    if [BUSY, UNAVAILABLE, OTHER].contains(&done.code) {
        return Err(Trouble::But(format!(
            "it can't serve now ({})",
            done.said()
        )));
    }
    // It answered in LDAP and keeps its root entry to itself: a server that
    // works, set up to tell nothing without a login.
    if done.code != 0 {
        return Ok(format!(
            "it tells nothing of itself without a login ({})",
            done.said()
        ));
    }
    let Some(root) = root.filter(|root| !root.0.is_empty()) else {
        return Ok("it tells nothing of itself without a login".to_string());
    };

    // A domain controller starting up, or one cut off from the others,
    // answers before it has its domain's data.
    if root.is("isSynchronized", "FALSE") {
        return Err(Trouble::But(
            "the domain controller says it isn't synchronised with its domain".to_string(),
        ));
    }
    let contexts = root.all("namingContexts");
    if let Some(name) = root.first("dnsHostName") {
        let mut words = match root.first("defaultNamingContext") {
            Some(context) if root.first("isSynchronized").is_some() => {
                format!("domain controller {name} of {}", domain(context))
            }
            Some(context) => format!("{name}, serves {}", domain(context)),
            None => name.to_string(),
        };
        if root.is("isGlobalCatalogReady", "TRUE") {
            words.push_str(", global catalog");
        }
        return Ok(words);
    }
    let vendor = [root.first("vendorName"), root.first("vendorVersion")]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    let serves = match contexts {
        [] => String::new(),
        [one] => format!("serves {one}"),
        [first, rest @ ..] => format!("serves {first} and {} more", rest.len()),
    };
    Ok([said(&vendor), serves]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", "))
}

/// What a server that serves said, and what its certificate is worth.
#[derive(Debug)]
struct Heard {
    words: String,
    certificate: Option<Outcome>,
}

/// The whole exchange on an open connection.
async fn talk(
    security: LdapSecurity,
    verify: bool,
    host: &str,
    trust: &Trust,
    mut stream: TcpStream,
) -> Result<Heard, Trouble> {
    if security == LdapSecurity::None {
        let (root, done) = read_root(&mut stream).await?;
        return Ok(Heard {
            words: describe(root.as_ref(), &done)?,
            certificate: None,
        });
    }
    let handshake = Handshake::new(trust)?;
    let mut tls = if security == LdapSecurity::Starttls {
        ask_for_tls(&mut stream).await?;
        handshake
            .connect(host, stream)
            .await
            .map_err(|why| Trouble::But(format!("TLS didn't start after StartTLS: {why}")))?
    } else {
        handshake.connect(host, stream).await?
    };
    let certificate = Some(handshake.certificate(&tls, verify, host));
    let (root, done) = read_root(&mut tls).await?;
    Ok(Heard {
        words: describe(root.as_ref(), &done)?,
        certificate,
    })
}

/// Ask the directory server at `host:port` for its root entry.
pub async fn check(
    host: &str,
    port: Option<u16>,
    security: LdapSecurity,
    verify: bool,
    trust: &Trust,
) -> Outcome {
    let port = port.unwrap_or(security.default_port());
    ask((security, verify), host, port, trust, TALK_TIMEOUT).await
}

async fn ask(
    (security, verify): (LdapSecurity, bool),
    host: &str,
    port: u16,
    trust: &Trust,
    timeout: Duration,
) -> Outcome {
    let start = Instant::now();
    let stream = match tokio::time::timeout(TCP_TIMEOUT, TcpStream::connect((host, port))).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => return Outcome::down(format!("Connection failed: {e}")),
        Err(_) => return Outcome::down("Connection timeout"),
    };
    let talked = tokio::time::timeout(timeout, talk(security, verify, host, trust, stream)).await;
    let heard = match talked {
        Ok(Ok(heard)) => heard,
        Ok(Err(Trouble::Silent(why))) => {
            return Outcome::down(format!("The port is open, but LDAP didn't answer: {why}"));
        }
        Ok(Err(Trouble::But(why))) => return Outcome::down(format!("LDAP answers, but {why}")),
        Err(_) => {
            return Outcome::down(format!(
                "The port is open, but LDAP didn't answer within {} s",
                timeout.as_secs_f32()
            ));
        }
    };
    let elapsed = start.elapsed();
    let how = match security {
        LdapSecurity::Starttls => " (StartTLS)",
        LdapSecurity::Tls => " (TLS)",
        LdapSecurity::None => "",
    };
    let ready = match heard.words.is_empty() {
        true => format!("LDAP ready{how}"),
        false => format!("LDAP ready{how}: {}", heard.words),
    };
    match heard.certificate {
        None => Outcome::up(ready),
        Some(certificate) if certificate.status == Health::Down => Outcome::down(format!(
            "LDAP answers, but its certificate isn't good: {}",
            certificate.message
        ))
        .with_metrics(certificate.metrics),
        Some(certificate) => Outcome::up(format!("{ready}. {}", certificate.message))
            .with_metrics(certificate.metrics),
    }
    .with_latency(elapsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::certificate::tests::{Authority, acceptor};
    use crate::checks::public_trust;
    use common::checks::metric;
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;

    const HOST: &str = "127.0.0.1";

    /// What a test server does, step by step.
    #[derive(Clone)]
    enum Step {
        Say(Vec<u8>),
        /// Read one message of the client's.
        Hear,
        /// Start TLS on the connection.
        Tls(TlsAcceptor),
    }
    use Step::{Hear, Say};

    trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
    impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}

    type Record = std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>;

    /// A server that plays `script` to each client, records the messages it
    /// heard (whole), then reads until the client hangs up. Returns its
    /// port and those messages.
    async fn server(script: Vec<Step>) -> (u16, Record) {
        let listener = TcpListener::bind((HOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let heard = Record::default();
        let record = std::sync::Arc::clone(&heard);
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let (script, record) = (script.clone(), std::sync::Arc::clone(&record));
                tokio::spawn(async move {
                    let mut stream: Box<dyn Duplex> = Box::new(socket);
                    for step in script {
                        match step {
                            Step::Say(bytes) => {
                                if stream.write_all(&bytes).await.is_err() {
                                    return;
                                }
                            }
                            Step::Hear => match read_message(&mut stream).await {
                                Ok(body) => record.lock().unwrap().push(tlv(SEQUENCE, &body)),
                                Err(_) => return,
                            },
                            Step::Tls(acceptor) => match acceptor.accept(stream).await {
                                Ok(tls) => stream = Box::new(tls),
                                Err(_) => return,
                            },
                        }
                    }
                    let mut rest = [0u8; 256];
                    while matches!(stream.read(&mut rest).await, Ok(n) if n > 0) {}
                });
            }
        });
        (port, heard)
    }

    fn result(tag: u8, code: u8, words: &str) -> Vec<u8> {
        tlv(
            tag,
            &[
                tlv(ENUMERATED, &[code]),
                tlv(OCTETS, b""),
                tlv(OCTETS, words.as_bytes()),
            ]
            .concat(),
        )
    }

    /// The end of the probe's search.
    fn done(code: u8, words: &str) -> Vec<u8> {
        message(SEARCH_ID, result(DONE, code, words))
    }

    /// A root entry with these attributes, as an answer to the search.
    fn root_entry(attributes: &[(&str, &[&str])]) -> Vec<u8> {
        let list: Vec<u8> = attributes
            .iter()
            .flat_map(|(name, values)| {
                let set: Vec<u8> = values
                    .iter()
                    .flat_map(|value| tlv(OCTETS, value.as_bytes()))
                    .collect();
                tlv(
                    SEQUENCE,
                    &[tlv(OCTETS, name.as_bytes()), tlv(SET, &set)].concat(),
                )
            })
            .collect();
        message(
            SEARCH_ID,
            tlv(ENTRY, &[tlv(OCTETS, b""), tlv(SEQUENCE, &list)].concat()),
        )
    }

    /// A domain controller's answer.
    fn domain_controller(synchronised: &str, catalog: &str) -> Vec<u8> {
        [
            root_entry(&[
                ("dnsHostName", &["dc01.corp.example.com"]),
                ("defaultNamingContext", &["DC=corp,DC=example,DC=com"]),
                (
                    "namingContexts",
                    &[
                        "DC=corp,DC=example,DC=com",
                        "CN=Configuration,DC=corp,DC=example,DC=com",
                    ],
                ),
                ("isSynchronized", &[synchronised]),
                ("isGlobalCatalogReady", &[catalog]),
            ]),
            done(0, ""),
        ]
        .concat()
    }

    async fn asked(security: LdapSecurity, verify: bool, port: u16, trust: &Trust) -> Outcome {
        ask(
            (security, verify),
            HOST,
            port,
            trust,
            Duration::from_millis(1500),
        )
        .await
    }

    /// A plain server that answers the search with `answer`, and what a
    /// check without TLS says of it.
    async fn plain(answer: Vec<u8>) -> Outcome {
        let (port, _) = server(vec![Hear, Say(answer)]).await;
        asked(LdapSecurity::None, true, port, &public_trust()).await
    }

    fn secured() -> (Authority, TlsAcceptor) {
        let authority = Authority::new();
        let (certificate, key) = authority.issue(HOST, (2099, 1, 1));
        (authority, acceptor(certificate, key))
    }

    #[test]
    fn the_requests_are_ldap_messages_byte_for_byte() {
        // As OpenLDAP's `ldapsearch -ZZ` sends it.
        assert_eq!(
            starttls_request(),
            [
                &[0x30, 0x1d, 0x02, 0x01, 0x01, 0x77, 0x18, 0x80, 0x16][..],
                b"1.3.6.1.4.1.1466.20037"
            ]
            .concat()
        );
        assert_eq!(unbind_request(), [0x30, 0x05, 0x02, 0x01, 0x03, 0x42, 0x00]);

        let search = search_request();
        let (envelope, after) = tagged(&search, SEQUENCE).unwrap();
        assert!(after.is_empty());
        let (id, rest) = tagged(envelope, INTEGER).unwrap();
        assert_eq!(id, [SEARCH_ID]);
        let (body, _) = tagged(rest, SEARCH).unwrap();
        // The entry with the empty name, itself only.
        assert!(body.starts_with(&[
            0x04, 0x00, 0x0a, 0x01, 0x00, 0x0a, 0x01, 0x00, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00,
            0x01, 0x01, 0x00, 0x87, 0x0b
        ]));
        let asked = String::from_utf8_lossy(body);
        for name in ATTRIBUTES {
            assert!(asked.contains(name), "{name}");
        }
        // Longer than 127 bytes: its length is written in the long form.
        assert_eq!(search[1], 0x81);
        assert_eq!(usize::from(search[2]), search.len() - 3);

        assert_eq!(tlv(OCTETS, &[7; 300])[..4], [0x04, 0x82, 0x01, 0x2c]);
        assert_eq!(element(&[0x04, 0x82, 0x01]), None);
        assert_eq!(element(&[0x04, 0x05, 1, 2]), None);
        assert_eq!(element(&[0x04, 0x89, 1]), None);
        assert_eq!(number(&[]), None);
        assert_eq!(number(&[1, 0, 0, 0, 0]), None);
        assert_eq!(number(&[0x01, 0x2c]), Some(300));
    }

    #[tokio::test]
    async fn a_domain_controller_says_who_it_is() {
        let (port, heard) = server(vec![Hear, Say(domain_controller("TRUE", "TRUE")), Hear]).await;
        let outcome = asked(LdapSecurity::None, true, port, &public_trust()).await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert_eq!(
            outcome.message,
            "LDAP ready: domain controller dc01.corp.example.com of corp.example.com, global \
             catalog"
        );
        assert!(outcome.latency_ms.is_some());
        assert!(outcome.metrics.is_empty());
        // The search, then the goodbye: no login.
        for _ in 0..50 {
            if heard.lock().unwrap().len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(*heard.lock().unwrap(), [search_request(), unbind_request()]);

        let no_catalog = plain(domain_controller("TRUE", "FALSE")).await;
        assert_eq!(
            no_catalog.message,
            "LDAP ready: domain controller dc01.corp.example.com of corp.example.com"
        );

        // One that hasn't its domain's data yet.
        let starting = plain(domain_controller("FALSE", "FALSE")).await;
        assert_eq!(starting.status, Health::Down);
        assert_eq!(
            starting.message,
            "LDAP answers, but the domain controller says it isn't synchronised with its domain"
        );
        assert!(starting.latency_ms.is_none());
    }

    #[tokio::test]
    async fn other_directory_servers_say_what_they_serve() {
        let openldap = plain(
            [
                root_entry(&[("namingContexts", &["dc=example,dc=org"])]),
                done(0, ""),
            ]
            .concat(),
        )
        .await;
        assert_eq!(openldap.status, Health::Up, "{}", openldap.message);
        assert_eq!(openldap.message, "LDAP ready: serves dc=example,dc=org");

        let vendor = plain(
            [
                root_entry(&[
                    (
                        "NAMINGCONTEXTS",
                        &["dc=a,dc=org", "o=ipaca", "cn=changelog"],
                    ),
                    ("vendorName", &["389 Project"]),
                    ("vendorVersion", &["389-Directory/2.4.5"]),
                ]),
                done(0, ""),
            ]
            .concat(),
        )
        .await;
        assert_eq!(
            vendor.message,
            "LDAP ready: 389 Project 389-Directory/2.4.5, serves dc=a,dc=org and 2 more"
        );

        // A name without a domain controller's state: not called one.
        let named = plain(
            [
                root_entry(&[
                    ("dnsHostName", &["ldap.example.org"]),
                    ("defaultNamingContext", &["o=example"]),
                ]),
                done(0, ""),
            ]
            .concat(),
        )
        .await;
        assert_eq!(
            named.message,
            "LDAP ready: ldap.example.org, serves o=example"
        );

        // An entry that says nothing the probe asked for.
        let bare = plain([root_entry(&[("objectClass", &["top"])]), done(0, "")].concat()).await;
        assert_eq!(bare.status, Health::Up);
        assert_eq!(bare.message, "LDAP ready");

        assert_eq!(domain("DC=corp, dc=example,DC=com"), "corp.example.com");
        assert_eq!(domain("o=example"), "o=example");
        assert_eq!(domain("ou=x,dc=example"), "ou=x,dc=example");
    }

    #[tokio::test]
    async fn a_server_that_tells_nothing_without_a_login_is_up() {
        // No entry, no error: the root entry is hidden.
        let hidden = plain(done(0, "")).await;
        assert_eq!(hidden.status, Health::Up);
        assert_eq!(
            hidden.message,
            "LDAP ready: it tells nothing of itself without a login"
        );
        let refused = plain(done(50, "")).await;
        assert_eq!(refused.status, Health::Up);
        assert_eq!(
            refused.message,
            "LDAP ready: it tells nothing of itself without a login (insufficient access rights)"
        );
        let bind_first = plain(done(1, "a successful bind must be completed\0")).await;
        assert_eq!(
            bind_first.message,
            "LDAP ready: it tells nothing of itself without a login (operations error: a \
             successful bind must be completed)"
        );
        let unknown = plain(done(123, "")).await;
        assert!(unknown.message.ends_with("(result code 123)"));
    }

    #[tokio::test]
    async fn a_server_that_cant_serve_or_isnt_one_is_down_with_the_reason() {
        for (code, words, message) in [
            (51, "", "LDAP answers, but it can't serve now (busy)"),
            (
                52,
                "shutting down",
                "LDAP answers, but it can't serve now (unavailable: shutting down)",
            ),
            (
                80,
                "",
                "LDAP answers, but it can't serve now (an internal error)",
            ),
        ] {
            let outcome = plain(done(code, words)).await;
            assert_eq!(outcome.status, Health::Down);
            assert_eq!(outcome.message, message);
        }
        // Its own notice that it hangs up (message 0).
        let notice = plain(message(
            0,
            result(EXTENDED_ANSWER, 52, "too many connections"),
        ))
        .await;
        assert_eq!(
            notice.message,
            "LDAP answers, but it closes the connection (unavailable: too many connections)"
        );

        // Something else listens there.
        let other = plain(b"SSH-2.0-OpenSSH_9.6\r\n".to_vec()).await;
        assert_eq!(other.status, Health::Down);
        assert_eq!(
            other.message,
            "The port is open, but LDAP didn't answer: its answer isn't LDAP's"
        );
        // An envelope with something else in it; a length no answer has.
        let odd = plain(tlv(SEQUENCE, &tlv(OCTETS, b"hello"))).await;
        assert_eq!(
            odd.message,
            "The port is open, but LDAP didn't answer: its answer isn't LDAP's"
        );
        let huge = plain(vec![0x30, 0x84, 0x7f, 0xff, 0xff, 0xff]).await;
        assert_eq!(
            huge.message,
            "The port is open, but LDAP didn't answer: its answer isn't LDAP's"
        );
        // Entries without end.
        let endless = plain(root_entry(&[("a", &["b"])]).repeat(MAX_MESSAGES + 1)).await;
        assert_eq!(
            endless.message,
            "The port is open, but LDAP didn't answer: its answer has no end"
        );

        // It says nothing; it hangs up; nobody listens.
        let (port, _) = server(vec![Hear]).await;
        let silent = asked(LdapSecurity::None, true, port, &public_trust()).await;
        assert_eq!(
            silent.message,
            "The port is open, but LDAP didn't answer within 1.5 s"
        );
        let listener = TcpListener::bind((HOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                drop(socket);
            }
        });
        let closed = asked(LdapSecurity::None, true, port, &public_trust()).await;
        assert!(
            closed
                .message
                .starts_with("The port is open, but LDAP didn't answer: "),
            "{}",
            closed.message
        );
        let free = {
            let listener = TcpListener::bind((HOST, 0)).await.unwrap();
            listener.local_addr().unwrap().port()
        };
        let nobody = asked(LdapSecurity::None, true, free, &public_trust()).await;
        assert!(
            nobody.message.starts_with("Connection"),
            "{}",
            nobody.message
        );
    }

    #[tokio::test]
    async fn starttls_is_asked_for_and_the_certificate_judged() {
        let (authority, tls) = secured();
        let (port, heard) = server(vec![
            Hear,
            Say(message(STARTTLS_ID, result(EXTENDED_ANSWER, 0, ""))),
            Step::Tls(tls),
            Hear,
            Say(domain_controller("TRUE", "FALSE")),
        ])
        .await;
        let outcome = asked(LdapSecurity::Starttls, true, port, &authority.trust()).await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert!(
            outcome.message.starts_with(
                "LDAP ready (StartTLS): domain controller dc01.corp.example.com of \
                 corp.example.com. Certificate expires in "
            ),
            "{}",
            outcome.message
        );
        assert_eq!(outcome.metrics[0].key, metric::DAYS_LEFT);
        assert!(outcome.metrics[0].value > 365.0);
        // The search went through TLS: the server read it.
        assert_eq!(
            *heard.lock().unwrap(),
            [starttls_request(), search_request()]
        );

        // A domain controller without a certificate refuses.
        let (port, _) = server(vec![
            Hear,
            Say(message(
                STARTTLS_ID,
                result(EXTENDED_ANSWER, 52, "Error initializing SSL/TLS"),
            )),
        ])
        .await;
        let refused = asked(LdapSecurity::Starttls, true, port, &authority.trust()).await;
        assert_eq!(refused.status, Health::Down);
        assert_eq!(
            refused.message,
            "LDAP answers, but it doesn't start TLS (StartTLS: unavailable: Error initializing \
             SSL/TLS)"
        );
        // It agrees, and something that isn't TLS follows.
        let (port, _) = server(vec![
            Hear,
            Say([
                message(STARTTLS_ID, result(EXTENDED_ANSWER, 0, "")),
                b"this is not TLS, not at all, whatever a client may hope for\r\n".to_vec(),
            ]
            .concat()),
        ])
        .await;
        let broken = asked(LdapSecurity::Starttls, true, port, &authority.trust()).await;
        assert!(
            broken
                .message
                .starts_with("LDAP answers, but TLS didn't start after StartTLS: TLS handshake"),
            "{}",
            broken.message
        );
    }

    #[tokio::test]
    async fn ldaps_is_tls_from_the_start_and_its_certificate_counts() {
        let (authority, tls) = secured();
        let script = vec![
            Step::Tls(tls),
            Hear,
            Say([
                root_entry(&[("namingContexts", &["dc=example,dc=org"])]),
                done(0, ""),
            ]
            .concat()),
        ];
        let (port, _) = server(script).await;
        let outcome = asked(LdapSecurity::Tls, true, port, &authority.trust()).await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert!(
            outcome
                .message
                .starts_with("LDAP ready (TLS): serves dc=example,dc=org. Certificate expires in "),
            "{}",
            outcome.message
        );
        assert!(
            outcome
                .message
                .ends_with("(2099-01-01), issued by Test Authority"),
            "{}",
            outcome.message
        );

        // An issuer nobody knows: Down, unless the check doesn't verify.
        let public = public_trust();
        let untrusted = asked(LdapSecurity::Tls, true, port, &public).await;
        assert_eq!(untrusted.status, Health::Down);
        assert!(
            untrusted.message.starts_with(
                "LDAP answers, but its certificate isn't good: Certificate not trusted"
            ),
            "{}",
            untrusted.message
        );
        assert!(untrusted.metrics[0].value > 365.0);
        let accepted = asked(LdapSecurity::Tls, false, port, &public).await;
        assert_eq!(accepted.status, Health::Up, "{}", accepted.message);

        // Expired: Down whether verified or not.
        let (certificate, key) = authority.issue(HOST, (2020, 1, 1));
        let (port, _) = server(vec![
            Step::Tls(acceptor(certificate, key)),
            Hear,
            Say(done(0, "")),
        ])
        .await;
        let expired = asked(LdapSecurity::Tls, false, port, &authority.trust()).await;
        assert_eq!(expired.status, Health::Down);
        assert!(
            expired
                .message
                .starts_with("LDAP answers, but its certificate isn't good: Certificate expired "),
            "{}",
            expired.message
        );

        // A server that isn't speaking TLS there.
        let (port, _) = server(vec![Say(b"220 mail ESMTP\r\n".to_vec())]).await;
        let plain = asked(LdapSecurity::Tls, true, port, &authority.trust()).await;
        assert!(
            plain
                .message
                .starts_with("The port is open, but LDAP didn't answer: TLS handshake"),
            "{}",
            plain.message
        );
    }

    #[test]
    fn the_usual_port_follows_how_it_is_encrypted() {
        assert_eq!(LdapSecurity::None.default_port(), 389);
        assert_eq!(LdapSecurity::Starttls.default_port(), 389);
        assert_eq!(LdapSecurity::Tls.default_port(), 636);
    }

    /// By hand (`--ignored --nocapture`): directory servers, asked.
    /// `LDAP_TEST_HOSTS` names them, separated by commas (public test
    /// servers unless set); each is asked plain, with StartTLS and by LDAPS.
    /// `LDAP_TEST_UNVERIFIED`: only the certificates' dates count.
    #[tokio::test]
    #[ignore = "talks to directory servers on the network: run by hand"]
    async fn real_directory_servers() {
        let trust = public_trust();
        let hosts = std::env::var("LDAP_TEST_HOSTS")
            .unwrap_or_else(|_| "ldap.forumsys.com,ipa.demo1.freeipa.org,db.debian.org".into());
        let verify = std::env::var_os("LDAP_TEST_UNVERIFIED").is_none();
        for host in hosts.split(',').map(str::trim) {
            for security in LdapSecurity::ALL {
                let outcome = check(host, None, *security, verify, &trust).await;
                println!(
                    "{host} {:?}: {:?} in {:?} ms: {}",
                    security, outcome.status, outcome.latency_ms, outcome.message
                );
            }
        }
    }
}
