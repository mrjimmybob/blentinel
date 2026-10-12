//! The `snmp` check (M5-08, D-86): ask an SNMP agent (a switch, a printer,
//! a NAS, a UPS) for values, by their OIDs.
//!
//! v2c with a community, or v3 with a user, authenticated and encrypted as
//! the check says (`snmp2`; v3's cryptography in pure Rust: FP-2). Read-only:
//! the probe only ever sends GET. Up when the agent answers with every value
//! asked for, each one what the check expects of it if it says. Numbers
//! become metrics; the message lists the values.

use std::time::{Duration, Instant};

use common::checks::{
    Metric, Secrets, SnmpAuth, SnmpOid, SnmpPrivacy, SnmpVersion, metric, secret,
};
use snmp2::v3::{Auth, AuthProtocol, Cipher, Security};
use snmp2::{AsyncSession, Error, Oid, Value};

use super::Outcome;

/// How long the agent has to answer one request, and how often it is asked
/// (UDP: a request or its answer can be lost).
const ANSWER_TIMEOUT: Duration = Duration::from_secs(3);
const ATTEMPTS: usize = 2;
/// How many values the message lists (the rest are counted).
const VALUES_NAMED: usize = 6;
/// The longest text of a value the message shows.
const MAX_TEXT_CHARS: usize = 80;

/// What to ask, and of whom: an `snmp` check's settings.
pub struct Query<'a> {
    pub host: &'a str,
    pub port: u16,
    pub version: SnmpVersion,
    pub user: &'a str,
    pub auth: Option<SnmpAuth>,
    pub privacy: Option<SnmpPrivacy>,
    pub oids: &'a [SnmpOid],
    pub secrets: &'a Secrets,
}

/// One value as the agent gave it.
#[derive(Debug, Clone, PartialEq)]
enum Reading {
    /// A number (counters, gauges, integers), and the unit it is in.
    Number(f64, &'static str),
    Text(String),
    /// The agent has no such value.
    Missing,
}

impl Reading {
    /// As the message shows it and an expectation compares it.
    fn text(&self) -> String {
        match self {
            // A whole number is written as one: an interface's state is `1`.
            Reading::Number(n, unit) if n.fract() == 0.0 => format!("{n:.0}{unit}"),
            Reading::Number(n, unit) => format!("{n}{unit}"),
            Reading::Text(text) => text.clone(),
            Reading::Missing => "no such value".into(),
        }
    }

    /// The value alone, to compare with what the check expects.
    fn bare(&self) -> String {
        match self {
            Reading::Number(n, _) if n.fract() == 0.0 => format!("{n:.0}"),
            Reading::Number(n, _) => n.to_string(),
            other => other.text(),
        }
    }
}

/// What a value from the agent is worth.
fn reading(value: &Value<'_>) -> Reading {
    match value {
        Value::Integer(n) => Reading::Number(*n as f64, ""),
        Value::Counter32(n) | Value::Unsigned32(n) => Reading::Number(f64::from(*n), ""),
        Value::Counter64(n) => Reading::Number(*n as f64, ""),
        // Hundredths of a second: in seconds.
        Value::Timeticks(ticks) => Reading::Number(f64::from(*ticks) / 100.0, " s"),
        Value::Boolean(b) => Reading::Text(b.to_string()),
        Value::OctetString(bytes) => Reading::Text(text_of(bytes)),
        Value::IpAddress(ip) => Reading::Text(std::net::Ipv4Addr::from(*ip).to_string()),
        Value::ObjectIdentifier(oid) => Reading::Text(oid.to_id_string()),
        Value::NoSuchObject | Value::NoSuchInstance | Value::EndOfMibView | Value::Null => {
            Reading::Missing
        }
        other => Reading::Text(format!("{other:?}")),
    }
}

/// Bytes as text where they are text (most are: names, descriptions), else
/// in hexadecimal (a MAC address). Cut to what a message can show.
fn text_of(bytes: &[u8]) -> String {
    let text = match std::str::from_utf8(bytes) {
        Ok(text)
            if !text
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t') =>
        {
            text.trim().replace(['\r', '\n', '\t'], " ")
        }
        _ => bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":"),
    };
    match text.char_indices().nth(MAX_TEXT_CHARS) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text,
    }
}

fn auth_protocol(auth: SnmpAuth) -> AuthProtocol {
    match auth {
        SnmpAuth::Md5 => AuthProtocol::Md5,
        SnmpAuth::Sha1 => AuthProtocol::Sha1,
        SnmpAuth::Sha224 => AuthProtocol::Sha224,
        SnmpAuth::Sha256 => AuthProtocol::Sha256,
        SnmpAuth::Sha384 => AuthProtocol::Sha384,
        SnmpAuth::Sha512 => AuthProtocol::Sha512,
    }
}

fn cipher(privacy: SnmpPrivacy) -> Cipher {
    match privacy {
        SnmpPrivacy::Des => Cipher::Des,
        SnmpPrivacy::Aes128 => Cipher::Aes128,
        SnmpPrivacy::Aes192 => Cipher::Aes192,
        SnmpPrivacy::Aes256 => Cipher::Aes256,
    }
}

/// The v3 user as the library wants it.
fn security(query: &Query<'_>) -> Security {
    let password = |name| query.secrets.get(name).unwrap_or_default().as_bytes();
    let user = Security::new(query.user.as_bytes(), password(secret::AUTH_PASSWORD));
    match (query.auth, query.privacy) {
        (None, _) => user.with_auth(Auth::NoAuthNoPriv),
        (Some(auth), None) => user
            .with_auth_protocol(auth_protocol(auth))
            .with_auth(Auth::AuthNoPriv),
        (Some(auth), Some(privacy)) => {
            user.with_auth_protocol(auth_protocol(auth))
                .with_auth(Auth::AuthPriv {
                    cipher: cipher(privacy),
                    privacy_password: password(secret::PRIV_PASSWORD).to_vec(),
                })
        }
    }
}

/// Why the agent's values couldn't be had, for people.
fn failure(error: &Error, query: &Query<'_>) -> String {
    match error {
        Error::AuthFailure(kind) => format!(
            "The agent didn't accept the user {} or its passwords ({kind})",
            query.user
        ),
        Error::CommunityMismatch => "The agent answered for another community".into(),
        Error::Send | Error::Receive => {
            "The agent can't be reached (nothing listens there?)".into()
        }
        // An agent answers a user it doesn't know, or a level it doesn't
        // allow, with a report the library can't read as values.
        other if query.version == SnmpVersion::V3 => format!(
            "The agent didn't answer with values: does it know the user {}, with this authentication and privacy? ({other})",
            query.user
        ),
        other => format!("The agent's answer can't be used: {other}"),
    }
}

/// One exchange: a session, v3's discovery, and every value asked for.
async fn exchange(query: &Query<'_>, oids: &[Oid<'static>]) -> Result<Vec<Reading>, Error> {
    let target = (query.host, query.port);
    let mut session = match query.version {
        SnmpVersion::V2c => {
            let community = query.secrets.get(secret::COMMUNITY).unwrap_or_default();
            AsyncSession::new_v2c(target, community.as_bytes(), 0).await
        }
        SnmpVersion::V3 => AsyncSession::new_v3(target, 0, security(query)).await,
    }
    .map_err(|_| Error::Send)?;
    if query.version == SnmpVersion::V3 {
        session.init().await?;
    }
    let asked: Vec<&Oid<'_>> = oids.iter().collect();
    // v3: the agent's clock is learned from its first answer, and the
    // request then repeated (the library says so with `AuthUpdated`).
    let mut tries = 2;
    loop {
        tries -= 1;
        match session.get_many(&asked).await {
            Ok(answer) if answer.error_status != 0 => {
                // The agent refused the request as a whole (v1 style).
                return Ok(vec![Reading::Missing; oids.len()]);
            }
            Ok(answer) => {
                let mut readings: Vec<Reading> =
                    answer.varbinds.map(|(_, value)| reading(&value)).collect();
                readings.resize(oids.len(), Reading::Missing);
                return Ok(readings);
            }
            Err(Error::AuthUpdated) if tries > 0 => {}
            Err(e) => return Err(e),
        }
    }
}

/// Ask the agent, and judge what it says.
pub async fn check(query: Query<'_>) -> Outcome {
    ask(&query, ANSWER_TIMEOUT).await
}

async fn ask(query: &Query<'_>, timeout: Duration) -> Outcome {
    // The hub and the probe validated the OIDs: these are numbers.
    let oids: Option<Vec<Oid<'static>>> = query
        .oids
        .iter()
        .map(|value| Oid::from(&value.arcs()?).ok())
        .collect();
    let Some(oids) = oids else {
        return Outcome::down("One of the values isn't an OID in numbers");
    };
    let version = match query.version {
        SnmpVersion::V2c => "v2c",
        SnmpVersion::V3 => "v3",
    };
    let mut last = format!(
        "No answer from {}:{} (SNMP {version}) within {} s: is SNMP on, is this probe allowed, and is the {} right? An agent doesn't answer a wrong one",
        query.host,
        query.port,
        timeout.as_secs_f32(),
        if query.version == SnmpVersion::V2c {
            "community"
        } else {
            "user"
        },
    );
    for _ in 0..ATTEMPTS {
        let start = Instant::now();
        match tokio::time::timeout(timeout, exchange(query, &oids)).await {
            Ok(Ok(readings)) => return judge(query.oids, &readings).with_latency(start.elapsed()),
            // A refusal is an answer: asking again changes nothing.
            Ok(Err(e @ (Error::AuthFailure(_) | Error::CommunityMismatch))) => {
                return Outcome::down(failure(&e, query));
            }
            Ok(Err(e)) => last = failure(&e, query),
            Err(_) => {}
        }
    }
    Outcome::down(last)
}

/// What the readings are worth: every value there, and as expected.
fn judge(asked: &[SnmpOid], readings: &[Reading]) -> Outcome {
    let mut problems = Vec::new();
    let mut metrics = Vec::new();
    let mut shown = Vec::new();
    for (value, reading) in asked.iter().zip(readings) {
        let label = value.label();
        match reading {
            Reading::Missing => problems.push(format!("{label}: the agent has no such value")),
            reading => {
                if let Some(expect) = &value.expect
                    && reading.bare() != *expect
                {
                    problems.push(format!("{label} is {} (expected {expect})", reading.text()));
                }
                if let Reading::Number(number, unit) = reading {
                    metrics.push(Metric::new(label, *number, unit.trim()));
                }
                shown.push(format!("{label} {}", reading.text()));
            }
        }
    }
    metrics.push(Metric::new(metric::ANSWERS, shown.len() as f64, ""));
    if !problems.is_empty() {
        return Outcome::down(problems.join("; ")).with_metrics(metrics);
    }
    let more = shown.len().saturating_sub(VALUES_NAMED);
    shown.truncate(VALUES_NAMED);
    let mut message = shown.join(", ");
    if more > 0 {
        message.push_str(&format!(" and {more} more"));
    }
    Outcome::up(message).with_metrics(metrics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::Health;
    use std::collections::BTreeMap;
    use tokio::net::UdpSocket;

    // --- A tiny SNMP v2c agent, written out in bytes (BER) -----------------

    /// Tag, length, content.
    fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        match content.len() {
            len @ 0..=127 => out.push(len as u8),
            len @ 128..=255 => out.extend([0x81, len as u8]),
            len => out.extend([0x82, (len >> 8) as u8, len as u8]),
        }
        out.extend(content);
        out
    }

    /// The first element of `bytes`: its tag, its content, and what follows.
    fn read(bytes: &[u8]) -> (u8, &[u8], &[u8]) {
        let (len, header) = match bytes[1] {
            0x81 => (bytes[2] as usize, 3),
            0x82 => (((bytes[2] as usize) << 8) | bytes[3] as usize, 4),
            len => (len as usize, 2),
        };
        (
            bytes[0],
            &bytes[header..header + len],
            &bytes[header + len..],
        )
    }

    fn integer(n: u32) -> Vec<u8> {
        let bytes = n.to_be_bytes();
        let start = bytes.iter().position(|b| *b != 0).unwrap_or(3);
        // A leading zero keeps a high first bit from meaning "negative".
        let mut content = if bytes[start] & 0x80 != 0 {
            vec![0]
        } else {
            vec![]
        };
        content.extend(&bytes[start..]);
        content
    }

    /// The agent's answer to `request`, or none (another community: a real
    /// agent stays silent). `values`: an encoded OID to its encoded value.
    fn answer(
        request: &[u8],
        community: &[u8],
        values: &BTreeMap<Vec<u8>, Vec<u8>>,
    ) -> Option<Vec<u8>> {
        let (_, message, _) = read(request);
        let (_, version, rest) = read(message);
        let (_, asked_community, rest) = read(rest);
        if asked_community != community {
            return None;
        }
        let (_, pdu, _) = read(rest);
        let (_, request_id, rest) = read(pdu);
        let (_, _, rest) = read(rest); // error status
        let (_, _, rest) = read(rest); // error index
        let (_, mut varbinds, _) = read(rest);
        let mut answered = Vec::new();
        while !varbinds.is_empty() {
            let (_, varbind, next) = read(varbinds);
            let (_, oid, _) = read(varbind);
            // 0x80: noSuchObject.
            let value = values.get(oid).cloned().unwrap_or_else(|| tlv(0x80, &[]));
            answered.extend(tlv(0x30, &[tlv(0x06, oid), value].concat()));
            varbinds = next;
        }
        let pdu = [
            tlv(0x02, request_id),
            tlv(0x02, &[0]),
            tlv(0x02, &[0]),
            tlv(0x30, &answered),
        ]
        .concat();
        // 0xA2: a response.
        Some(tlv(
            0x30,
            &[tlv(0x02, version), tlv(0x04, community), tlv(0xA2, &pdu)].concat(),
        ))
    }

    /// An agent on a local port that knows `values` and `community`.
    async fn agent(community: &'static str, values: &[(&str, Vec<u8>)]) -> u16 {
        let values: BTreeMap<Vec<u8>, Vec<u8>> = values
            .iter()
            .map(|(oid, value)| {
                let arcs = SnmpOid::of(oid).arcs().unwrap();
                (Oid::from(&arcs).unwrap().as_bytes().to_vec(), value.clone())
            })
            .collect();
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = socket.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut buffer = [0u8; 2048];
            while let Ok((len, from)) = socket.recv_from(&mut buffer).await {
                if let Some(reply) = answer(&buffer[..len], community.as_bytes(), &values) {
                    let _ = socket.send_to(&reply, from).await;
                }
            }
        });
        port
    }

    const UPTIME: &str = "1.3.6.1.2.1.1.3.0";
    const NAME: &str = "1.3.6.1.2.1.1.5.0";
    const LINK: &str = "1.3.6.1.2.1.2.2.1.8.1";
    const TRAFFIC: &str = "1.3.6.1.2.1.31.1.1.1.6.1";

    async fn switch() -> u16 {
        agent(
            "s3cret",
            &[
                // TimeTicks: 123456 hundredths of a second.
                (UPTIME, tlv(0x43, &integer(123_456))),
                (NAME, tlv(0x04, b"core-switch")),
                (LINK, tlv(0x02, &integer(1))),
                // Counter64.
                (TRAFFIC, tlv(0x46, &integer(4_000_000_000))),
            ],
        )
        .await
    }

    fn value(oid: &str, name: &str, expect: Option<&str>) -> SnmpOid {
        SnmpOid {
            oid: oid.into(),
            name: name.into(),
            expect: expect.map(str::to_string),
        }
    }

    async fn ask_v2c(port: u16, community: &str, oids: &[SnmpOid]) -> Outcome {
        let secrets = Secrets(BTreeMap::from([(
            secret::COMMUNITY.to_string(),
            community.to_string(),
        )]));
        let query = Query {
            host: "127.0.0.1",
            port,
            version: SnmpVersion::V2c,
            user: "",
            auth: None,
            privacy: None,
            oids,
            secrets: &secrets,
        };
        ask(&query, Duration::from_millis(400)).await
    }

    #[tokio::test]
    async fn an_agent_answers_with_its_values_and_numbers_become_metrics() {
        let port = switch().await;
        let oids = [
            value(UPTIME, "Uptime", None),
            value(NAME, "Name", None),
            value(LINK, "Uplink", Some("1")),
            value(TRAFFIC, "", None),
        ];
        let outcome = ask_v2c(port, "s3cret", &oids).await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert_eq!(
            outcome.message,
            format!("Uptime 1234.56 s, Name core-switch, Uplink 1, {TRAFFIC} 4000000000")
        );
        assert!(outcome.latency_ms.is_some());
        let metrics: Vec<(&str, f64, &str)> = outcome
            .metrics
            .iter()
            .map(|m| (m.key.as_str(), m.value, m.unit.as_str()))
            .collect();
        assert_eq!(
            metrics,
            [
                ("Uptime", 1234.56, "s"),
                ("Uplink", 1.0, ""),
                (TRAFFIC, 4_000_000_000.0, ""),
                (metric::ANSWERS, 4.0, ""),
            ]
        );
    }

    #[tokio::test]
    async fn a_value_that_isnt_what_is_expected_or_isnt_there_is_down() {
        let port = switch().await;
        let down = ask_v2c(port, "s3cret", &[value(LINK, "Uplink", Some("2"))]).await;
        assert_eq!(down.status, Health::Down);
        assert_eq!(down.message, "Uplink is 1 (expected 2)");
        // The value is still a metric: the hub keeps its history.
        assert_eq!(down.metrics[0].key, "Uplink");

        let text = ask_v2c(port, "s3cret", &[value(NAME, "Name", Some("core-switch"))]).await;
        assert_eq!(text.status, Health::Up, "{}", text.message);

        let missing = ask_v2c(
            port,
            "s3cret",
            &[
                value(NAME, "Name", None),
                value("1.3.6.1.4.1.9.9.9.0", "Fan", None),
            ],
        )
        .await;
        assert_eq!(missing.status, Health::Down);
        assert_eq!(missing.message, "Fan: the agent has no such value");
    }

    #[tokio::test]
    async fn a_wrong_community_gets_no_answer_and_the_message_says_what_to_check() {
        let port = switch().await;
        let outcome = ask_v2c(port, "public", &[value(UPTIME, "", None)]).await;
        assert_eq!(outcome.status, Health::Down);
        assert!(
            outcome
                .message
                .starts_with(&format!("No answer from 127.0.0.1:{port} (SNMP v2c)")),
            "{}",
            outcome.message
        );
        assert!(outcome.message.contains("community"), "{}", outcome.message);
        // The community itself is never in a message.
        assert!(!outcome.message.contains("public"));
    }

    #[tokio::test]
    async fn nobody_listening_is_down() {
        // A port that was just free.
        let port = {
            let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            socket.local_addr().unwrap().port()
        };
        let outcome = ask_v2c(port, "public", &[value(UPTIME, "", None)]).await;
        assert_eq!(outcome.status, Health::Down);
        assert!(
            outcome.message.starts_with("No answer")
                || outcome.message.starts_with("The agent can't be reached"),
            "{}",
            outcome.message
        );
    }

    #[test]
    fn values_are_read_as_numbers_or_text() {
        assert_eq!(reading(&Value::Integer(-5)), Reading::Number(-5.0, ""));
        assert_eq!(reading(&Value::Timeticks(250)).text(), "2.5 s");
        assert_eq!(reading(&Value::Timeticks(250)).bare(), "2.5");
        assert_eq!(reading(&Value::Counter64(7)).text(), "7");
        assert_eq!(
            reading(&Value::OctetString(b" HP LaserJet\r\n")).text(),
            "HP LaserJet"
        );
        assert_eq!(
            reading(&Value::OctetString(&[0x00, 0x1b, 0x21, 0xff, 0x0a, 0x01])).text(),
            "00:1b:21:ff:0a:01",
            "a MAC address isn't text"
        );
        assert_eq!(
            reading(&Value::IpAddress([192, 168, 1, 1])).text(),
            "192.168.1.1"
        );
        assert_eq!(reading(&Value::NoSuchInstance), Reading::Missing);
        let long = "x".repeat(200);
        assert_eq!(
            reading(&Value::OctetString(long.as_bytes()))
                .text()
                .chars()
                .count(),
            MAX_TEXT_CHARS + 1
        );
    }

    #[test]
    fn many_values_are_counted_not_all_listed() {
        let asked: Vec<SnmpOid> = (0..9)
            .map(|i| {
                value(
                    &format!("1.3.6.1.2.1.2.2.1.8.{i}"),
                    &format!("Port {i}"),
                    None,
                )
            })
            .collect();
        let readings = vec![Reading::Number(1.0, ""); 9];
        let outcome = judge(&asked, &readings);
        assert_eq!(outcome.status, Health::Up);
        assert!(
            outcome.message.ends_with("Port 5 1 and 3 more"),
            "{}",
            outcome.message
        );
        assert_eq!(outcome.metrics.len(), 10);
    }

    /// A real agent, for the v3 exchange no fake here speaks: run by hand
    /// (`--ignored`) with `SNMP_TEST_HOST`, `SNMP_TEST_USER`,
    /// `SNMP_TEST_AUTH` (`sha1`, …), `SNMP_TEST_AUTH_PASSWORD`, and for
    /// privacy `SNMP_TEST_PRIV` and `SNMP_TEST_PRIV_PASSWORD` (D-86).
    #[tokio::test]
    #[ignore = "needs a real SNMP v3 agent: see the comment"]
    async fn a_real_v3_agent_answers() {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let host = var("SNMP_TEST_HOST").expect("SNMP_TEST_HOST");
        let user = var("SNMP_TEST_USER").expect("SNMP_TEST_USER");
        let auth = var("SNMP_TEST_AUTH").map(|a| SnmpAuth::parse(&a).expect("an auth protocol"));
        let privacy = var("SNMP_TEST_PRIV").map(|p| SnmpPrivacy::parse(&p).expect("a cipher"));
        let mut secrets = BTreeMap::new();
        for (name, variable) in [
            (secret::AUTH_PASSWORD, "SNMP_TEST_AUTH_PASSWORD"),
            (secret::PRIV_PASSWORD, "SNMP_TEST_PRIV_PASSWORD"),
        ] {
            if let Some(value) = var(variable) {
                secrets.insert(name.to_string(), value);
            }
        }
        let secrets = Secrets(secrets);
        let oids = [value(UPTIME, "Uptime", None), value(NAME, "Name", None)];
        fn query<'a>(
            host: &'a str,
            user: &'a str,
            (auth, privacy): (Option<SnmpAuth>, Option<SnmpPrivacy>),
            oids: &'a [SnmpOid],
            secrets: &'a Secrets,
        ) -> Query<'a> {
            Query {
                host,
                port: 161,
                version: SnmpVersion::V3,
                user,
                auth,
                privacy,
                oids,
                secrets,
            }
        }
        let how = (auth, privacy);
        let outcome = check(query(&host, &user, how, &oids, &secrets)).await;
        println!("{} | {:?}", outcome.message, outcome.metrics);
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert!(
            outcome.message.starts_with("Uptime "),
            "{}",
            outcome.message
        );

        // A wrong password is refused, or gets no answer: never Up.
        if auth.is_some() {
            let mut wrong = secrets.clone();
            wrong
                .0
                .insert(secret::AUTH_PASSWORD.into(), "not the password".into());
            let refused = check(query(&host, &user, how, &oids, &wrong)).await;
            println!("wrong password: {}", refused.message);
            assert_eq!(refused.status, Health::Down);
        }
        let nobody = check(query(&host, "blentinel-no-such-user", how, &oids, &secrets)).await;
        println!("unknown user: {}", nobody.message);
        assert_eq!(nobody.status, Health::Down);
    }
}
