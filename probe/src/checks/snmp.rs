//! The `snmp` check (M5-08, D-86; tables, shares, rates and meanings:
//! M5-09, D-87): ask an SNMP agent (a switch, a printer, a NAS, a UPS) for
//! values, by their OIDs.
//!
//! v2c with a community, or v3 with a user, authenticated and encrypted as
//! the check says (`snmp2`; v3's cryptography in pure Rust: FP-2). Read-only:
//! the probe only ever sends GET, GETNEXT and GETBULK. Up when the agent
//! answers with every value asked for, each one what the check expects of
//! it if it says. Numbers become metrics (the hub judges them by the
//! value's limits); the message lists the values.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use common::checks::{
    MAX_SNMP_METRICS, MAX_SNMP_ROWS, Metric, Secrets, SnmpAuth, SnmpOid, SnmpPrivacy, SnmpVersion,
    metric, oid_arcs, secret,
};
use snmp2::v3::{Auth, AuthProtocol, Cipher, Security};
use snmp2::{AsyncSession, Error, Oid, Value};

use super::Outcome;

/// How long the agent has to answer one request, and how often it is asked
/// (UDP: a request or its answer can be lost).
const ANSWER_TIMEOUT: Duration = Duration::from_secs(3);
const ATTEMPTS: usize = 2;
/// How many values one GET asks for, and how many rows one GETBULK.
const VALUES_AT_ONCE: usize = 20;
const ROWS_AT_ONCE: u32 = 20;
/// How many values the message lists (the rest are counted), and how many
/// rows of a table it lists one by one (more are summed up).
const VALUES_NAMED: usize = 6;
const ROWS_NAMED: usize = 4;
/// How many problems the message of a check that is down names.
const PROBLEMS_NAMED: usize = 5;
/// The longest text of a value the message shows.
const MAX_TEXT_CHARS: usize = 80;
/// A counter not read for this long is forgotten: its next reading starts
/// a rate anew.
const RATE_MEMORY: Duration = Duration::from_secs(3600);

/// What to ask, and of whom: an `snmp` check's settings.
pub struct Query<'a> {
    /// The check's ID: what its counters are remembered under.
    pub check_id: &'a str,
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

/// A number as results show it: whole, or with up to two decimals.
fn number_text(number: f64) -> String {
    let text = format!("{number:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// What a value from the agent is worth.
fn reading(value: &Value<'_>) -> Reading {
    match value {
        Value::Integer(n) => Reading::Number(*n as f64, ""),
        Value::Counter32(n) | Value::Unsigned32(n) => Reading::Number(f64::from(*n), ""),
        Value::Counter64(n) => Reading::Number(*n as f64, ""),
        // Hundredths of a second: in seconds.
        Value::Timeticks(ticks) => Reading::Number(f64::from(*ticks) / 100.0, "s"),
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

/// Why the agent's values couldn't be had.
#[derive(Debug)]
enum Failure {
    /// No answer in time, however often it was asked.
    Silent,
    /// The agent said no: asking again changes nothing.
    Refused(Error),
    Other(Error),
}

/// `error` as a failure: a refusal, or something a new try may get past.
fn failed(error: Error) -> Failure {
    match error {
        Error::AuthFailure(_) | Error::CommunityMismatch => Failure::Refused(error),
        other => Failure::Other(other),
    }
}

/// Why the agent's values couldn't be had, for people.
fn failure_text(failure: &Failure, query: &Query<'_>, timeout: Duration) -> String {
    let error = match failure {
        Failure::Silent => {
            return format!(
                "No answer from {}:{} (SNMP {}) within {} s: is SNMP on, is this probe allowed, and is the {} right? An agent doesn't answer a wrong one",
                query.host,
                query.port,
                query.version.as_str(),
                timeout.as_secs_f32(),
                if query.version == SnmpVersion::V2c {
                    "community"
                } else {
                    "user"
                },
            );
        }
        Failure::Refused(error) | Failure::Other(error) => error,
    };
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

/// An OID's numbers as the key its answer is kept under.
fn key(arcs: &[u64]) -> String {
    arcs.iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

/// What a check asks the agent for: single values, and columns of tables.
/// Each OID once, whichever values need it.
#[derive(Debug, Default, PartialEq)]
struct Plan {
    values: Vec<Vec<u64>>,
    columns: Vec<Vec<u64>>,
}

/// `None`: one of the OIDs isn't one (the hub and the probe validated
/// them, so this doesn't happen).
fn plan(asked: &[SnmpOid]) -> Option<Plan> {
    let mut plan = Plan::default();
    for value in asked {
        let list = if value.walk {
            &mut plan.columns
        } else {
            &mut plan.values
        };
        for oid in [Some(&value.oid), value.rows.as_ref(), value.of.as_ref()]
            .into_iter()
            .flatten()
        {
            let arcs = oid_arcs(oid)?;
            if !list.contains(&arcs) {
                list.push(arcs);
            }
        }
    }
    Some(plan)
}

/// What the agent answered: values by their OID, and the rows of columns
/// (each row by its index: what follows the column's OID).
#[derive(Debug, Default)]
struct Answers {
    values: HashMap<String, Reading>,
    columns: HashMap<String, Vec<(String, Reading)>>,
}

impl Answers {
    fn value(&self, oid: &str) -> Reading {
        oid_arcs(oid)
            .and_then(|arcs| self.values.get(&key(&arcs)).cloned())
            .unwrap_or(Reading::Missing)
    }

    fn column(&self, oid: &str) -> &[(String, Reading)] {
        oid_arcs(oid)
            .and_then(|arcs| self.columns.get(&key(&arcs)))
            .map_or(&[], Vec::as_slice)
    }

    /// The row `index` of a column.
    fn cell(&self, oid: &str, index: &str) -> Reading {
        self.column(oid)
            .iter()
            .find(|(row, _)| row == index)
            .map_or(Reading::Missing, |(_, reading)| reading.clone())
    }
}

fn oid_of(arcs: &[u64]) -> Result<Oid<'static>, Failure> {
    Oid::from(arcs).map_err(|_| Failure::Other(Error::AsnInvalidLen))
}

/// GET: the agent's values for `oids`, in their order.
async fn get(
    session: &mut AsyncSession,
    oids: &[Oid<'static>],
    timeout: Duration,
) -> Result<Vec<Reading>, Failure> {
    let asked: Vec<&Oid<'_>> = oids.iter().collect();
    let mut failure = Failure::Silent;
    let mut left = ATTEMPTS;
    // v3: the agent's clock is learned from its first answer, and the
    // request then repeated (the library says so with `AuthUpdated`).
    let mut clock_known = false;
    while left > 0 {
        left -= 1;
        match tokio::time::timeout(timeout, session.get_many(&asked)).await {
            Err(_) => failure = Failure::Silent,
            // The agent refused the request as a whole (v1 style).
            Ok(Ok(answer)) if answer.error_status != 0 => {
                return Ok(vec![Reading::Missing; oids.len()]);
            }
            Ok(Ok(answer)) => {
                let mut readings: Vec<Reading> =
                    answer.varbinds.map(|(_, value)| reading(&value)).collect();
                readings.resize(oids.len(), Reading::Missing);
                return Ok(readings);
            }
            Ok(Err(Error::AuthUpdated)) if !clock_known => {
                clock_known = true;
                left += 1;
            }
            Ok(Err(error)) => match failed(error) {
                refused @ Failure::Refused(_) => return Err(refused),
                other => failure = other,
            },
        }
    }
    Err(failure)
}

/// GETBULK: what follows `after` in the agent's values, each with its OID.
async fn following(
    session: &mut AsyncSession,
    after: &Oid<'static>,
    timeout: Duration,
) -> Result<Vec<(Vec<u64>, Reading)>, Failure> {
    let mut failure = Failure::Silent;
    let mut left = ATTEMPTS;
    let mut clock_known = false;
    while left > 0 {
        left -= 1;
        match tokio::time::timeout(timeout, session.getbulk(&[after], 0, ROWS_AT_ONCE)).await {
            Err(_) => failure = Failure::Silent,
            Ok(Ok(answer)) if answer.error_status != 0 => return Ok(Vec::new()),
            Ok(Ok(answer)) => {
                return Ok(answer
                    .varbinds
                    .filter_map(|(oid, value)| {
                        Some((oid_arcs(&oid.to_id_string())?, reading(&value)))
                    })
                    .collect());
            }
            Ok(Err(Error::AuthUpdated)) if !clock_known => {
                clock_known = true;
                left += 1;
            }
            Ok(Err(error)) => match failed(error) {
                refused @ Failure::Refused(_) => return Err(refused),
                other => failure = other,
            },
        }
    }
    Err(failure)
}

/// Walk a column of a table: its rows in the agent's order, up to one more
/// than [`MAX_SNMP_ROWS`] (so that a table that is longer is known to be).
async fn walk(
    session: &mut AsyncSession,
    column: &[u64],
    timeout: Duration,
) -> Result<Vec<(String, Reading)>, Failure> {
    let mut rows = Vec::new();
    let mut last = column.to_vec();
    'table: loop {
        let answers = following(session, &oid_of(&last)?, timeout).await?;
        if answers.is_empty() {
            break;
        }
        for (oid, reading) in answers {
            // Past the column, at the end of what the agent has, or an
            // agent that goes in circles: the table is read.
            if oid.len() <= column.len()
                || !oid.starts_with(column)
                || oid <= last
                || reading == Reading::Missing
            {
                break 'table;
            }
            rows.push((key(&oid[column.len()..]), reading));
            last = oid;
            if rows.len() > MAX_SNMP_ROWS {
                break 'table;
            }
        }
    }
    Ok(rows)
}

/// One exchange: a session, v3's discovery, and everything asked for.
async fn exchange(query: &Query<'_>, plan: &Plan, timeout: Duration) -> Result<Answers, Failure> {
    let target = (query.host, query.port);
    let mut session = match query.version {
        SnmpVersion::V2c => {
            let community = query.secrets.get(secret::COMMUNITY).unwrap_or_default();
            AsyncSession::new_v2c(target, community.as_bytes(), 0).await
        }
        SnmpVersion::V3 => AsyncSession::new_v3(target, 0, security(query)).await,
    }
    .map_err(|_| Failure::Other(Error::Send))?;
    if query.version == SnmpVersion::V3 {
        let mut discovered = Err(Failure::Silent);
        for _ in 0..ATTEMPTS {
            discovered = match tokio::time::timeout(timeout, session.init()).await {
                Err(_) => Err(Failure::Silent),
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(failed(error)),
            };
            if !matches!(discovered, Err(Failure::Silent | Failure::Other(_))) {
                break;
            }
        }
        discovered?;
    }
    let mut answers = Answers::default();
    for some in plan.values.chunks(VALUES_AT_ONCE) {
        let oids: Vec<Oid<'static>> = some
            .iter()
            .map(|arcs| oid_of(arcs))
            .collect::<Result<_, _>>()?;
        let readings = get(&mut session, &oids, timeout).await?;
        for (arcs, reading) in some.iter().zip(readings) {
            answers.values.insert(key(arcs), reading);
        }
    }
    for column in &plan.columns {
        let rows = walk(&mut session, column, timeout).await?;
        answers.columns.insert(key(column), rows);
    }
    Ok(answers)
}

/// The counters of `per second` values as they were last read, to tell
/// how fast they grow: by check and by value.
#[derive(Default)]
struct Rates(HashMap<String, (f64, Instant)>);

impl Rates {
    /// How fast the counter `name` of `check_id` grew since it was last
    /// read. `None`: not read before, or it went down (the device
    /// restarted, the counter went round).
    fn per_second(
        &mut self,
        check_id: &str,
        name: &str,
        counter: f64,
        now: Instant,
    ) -> Option<f64> {
        let previous = self
            .0
            .insert(format!("{check_id}\n{name}"), (counter, now))?;
        let (before, then) = previous;
        let secs = now.checked_duration_since(then)?.as_secs_f64();
        (secs > 0.0 && counter >= before).then(|| (counter - before) / secs)
    }

    fn forget_old(&mut self, now: Instant) {
        self.0.retain(|_, (_, read)| {
            now.checked_duration_since(*read)
                .is_none_or(|age| age < RATE_MEMORY)
        });
    }
}

static RATES: LazyLock<Mutex<Rates>> = LazyLock::new(Mutex::default);

/// Ask the agent, and judge what it says.
pub async fn check(query: Query<'_>) -> Outcome {
    ask(&query, ANSWER_TIMEOUT).await
}

async fn ask(query: &Query<'_>, timeout: Duration) -> Outcome {
    let Some(plan) = plan(query.oids) else {
        return Outcome::down("One of the values isn't an OID in numbers");
    };
    let start = Instant::now();
    match exchange(query, &plan, timeout).await {
        Ok(answers) => {
            let elapsed = start.elapsed();
            let now = Instant::now();
            let mut rates = RATES
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            rates.forget_old(now);
            judge(query.check_id, query.oids, &answers, &mut rates, now).with_latency(elapsed)
        }
        Err(failure) => Outcome::down(failure_text(&failure, query, timeout)),
    }
}

/// One value as results show it.
#[derive(Debug, Clone, PartialEq)]
struct Seen {
    /// What the message shows: the number with its unit, the text, or what
    /// the value means.
    text: String,
    /// The value alone, as the agent gave it: what an expectation may name.
    bare: String,
    /// The number the hub keeps a history of, with its unit.
    number: Option<(f64, String)>,
}

impl Seen {
    fn words(text: &str) -> Self {
        Self {
            text: text.to_string(),
            bare: text.to_string(),
            number: None,
        }
    }

    /// Is this one of `words`? By the value or by its meaning, in any case.
    fn is_any_of(&self, words: &[String]) -> bool {
        words.iter().any(|word| {
            word.eq_ignore_ascii_case(&self.bare) || word.eq_ignore_ascii_case(&self.text)
        })
    }
}

/// What `raw` is worth to `value`: as a share of `whole`, as a rate, in its
/// unit, with its meaning. `None`: the agent has no such value.
fn see(
    value: &SnmpOid,
    raw: &Reading,
    whole: Option<&Reading>,
    rate: impl FnOnce(f64) -> Option<f64>,
) -> Option<Seen> {
    let (number, unit) = match raw {
        Reading::Missing => return None,
        Reading::Text(text) => {
            let meaning = value.names.get(text);
            return Some(Seen {
                text: meaning.unwrap_or(text).clone(),
                bare: text.clone(),
                number: None,
            });
        }
        Reading::Number(number, unit) => (*number, *unit),
    };
    let bare = number_text(number);
    if let Some(meaning) = value.names.get(&bare) {
        return Some(Seen {
            text: meaning.clone(),
            bare,
            number: None,
        });
    }
    let (mut number, mut unit) = (number, unit.to_string());
    if value.of.is_some() {
        // A level below zero is the device saying it doesn't know.
        match whole {
            Some(Reading::Number(whole, _)) if *whole > 0.0 && number >= 0.0 => {
                number = number / whole * 100.0;
                unit = metric::PERCENT.to_string();
            }
            _ => return Some(Seen::words("unknown")),
        }
    }
    if value.rate {
        match rate(number) {
            Some(per_second) => number = per_second,
            // The first reading of a counter: its rate is known next time.
            None => return Some(Seen::words("measuring")),
        }
    }
    if let Some(scale) = value.scale {
        number *= scale;
    }
    if !value.unit.is_empty() {
        unit = value.unit.clone();
    }
    let shown = number_text(number);
    Some(Seen {
        text: if unit.is_empty() {
            shown.clone()
        } else {
            format!("{shown} {unit}")
        },
        // A number that was worked out is compared as it is shown.
        bare: if value.of.is_some() || value.rate || value.scale.is_some() {
            shown
        } else {
            bare
        },
        number: (!number.is_nan()).then_some((number, unit)),
    })
}

/// What is wrong with a value that is there, if anything.
fn problem(value: &SnmpOid, label: &str, seen: &Seen) -> Option<String> {
    if !value.expect.is_empty() && !seen.is_any_of(&value.expect) {
        return Some(format!(
            "{label} is {} (expected {})",
            seen.text,
            value.expect.join(" or ")
        ));
    }
    seen.is_any_of(&value.not)
        .then(|| format!("{label} is {}", seen.text))
}

/// The rows of a table in a few words: each one where they are few, else
/// how many are what (`18 up, 6 down`), or from what to what (`31-44 °C`).
fn table_text(label: &str, rows: &[(String, Seen)]) -> Vec<String> {
    if rows.len() <= ROWS_NAMED {
        return rows
            .iter()
            .map(|(row, seen)| format!("{row} {}", seen.text))
            .collect();
    }
    let numbers: Vec<&(f64, String)> = rows.iter().filter_map(|(_, s)| s.number.as_ref()).collect();
    if numbers.len() == rows.len() {
        let lowest = numbers
            .iter()
            .map(|(n, _)| *n)
            .fold(f64::INFINITY, f64::min);
        let highest = numbers
            .iter()
            .map(|(n, _)| *n)
            .fold(f64::NEG_INFINITY, f64::max);
        let unit = match numbers[0].1.as_str() {
            "" => String::new(),
            unit => format!(" {unit}"),
        };
        return vec![format!(
            "{label}: {} rows, {} to {}{unit}",
            rows.len(),
            number_text(lowest),
            number_text(highest)
        )];
    }
    // In the order they first appear, the most frequent first.
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for (_, seen) in rows {
        match counts.iter_mut().find(|(text, _)| *text == seen.text) {
            Some((_, count)) => *count += 1,
            None => counts.push((&seen.text, 1)),
        }
    }
    counts.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let more = counts.len().saturating_sub(3);
    let mut text = counts
        .iter()
        .take(3)
        .map(|(text, count)| format!("{count} {text}"))
        .collect::<Vec<_>>()
        .join(", ");
    if more > 0 {
        text.push_str(&format!(" and {more} other"));
    }
    vec![format!("{label}: {text}")]
}

/// `items`, the first `named` of them, and how many more there are.
fn listed(mut items: Vec<String>, named: usize, separator: &str) -> String {
    let more = items.len().saturating_sub(named);
    items.truncate(named);
    let mut text = items.join(separator);
    if more > 0 {
        text.push_str(&format!(" and {more} more"));
    }
    text
}

/// What the answers are worth: every value there, and as expected.
fn judge(
    check_id: &str,
    asked: &[SnmpOid],
    answers: &Answers,
    rates: &mut Rates,
    now: Instant,
) -> Outcome {
    let mut problems = Vec::new();
    let mut metrics = Vec::new();
    let mut shown = Vec::new();
    let mut seen_values = 0u32;
    let mut notes = Vec::new();
    let mut numbers_left_out = false;
    for value in asked {
        let label = value.label();
        // Each value to judge: its name, what the agent gave, and what it
        // is a share of.
        let mut entries: Vec<(String, Reading, Option<Reading>)> = Vec::new();
        if value.walk {
            let rows = answers.column(&value.oid);
            if rows.is_empty() {
                problems.push(format!("{label}: the agent has no such table"));
                continue;
            }
            if rows.len() > MAX_SNMP_ROWS {
                notes.push(format!("{label}: the first {MAX_SNMP_ROWS} rows only"));
            }
            for (index, raw) in rows.iter().take(MAX_SNMP_ROWS) {
                let row = match value
                    .rows
                    .as_deref()
                    .map(|names| answers.cell(names, index))
                {
                    Some(Reading::Text(name)) if !name.is_empty() => name,
                    Some(Reading::Number(number, _)) => number_text(number),
                    _ => index.clone(),
                };
                let whole = value.of.as_deref().map(|of| answers.cell(of, index));
                entries.push((value.row_label(&row), raw.clone(), whole));
            }
        } else {
            let whole = value.of.as_deref().map(|of| answers.value(of));
            entries.push((label.to_string(), answers.value(&value.oid), whole));
        }
        let mut rows = Vec::new();
        for (name, raw, whole) in entries {
            let rate = |counter| rates.per_second(check_id, &name, counter, now);
            let Some(seen) = see(value, &raw, whole.as_ref(), rate) else {
                problems.push(format!("{name}: the agent has no such value"));
                continue;
            };
            seen_values += 1;
            problems.extend(problem(value, &name, &seen));
            if let Some((number, unit)) = &seen.number {
                if metrics.len() < MAX_SNMP_METRICS {
                    metrics.push(Metric::new(&name, *number, unit));
                } else {
                    numbers_left_out = true;
                }
            }
            rows.push((name, seen));
        }
        if value.walk {
            shown.extend(table_text(label, &rows));
        } else {
            shown.extend(
                rows.iter()
                    .map(|(name, seen)| format!("{name} {}", seen.text)),
            );
        }
    }
    if numbers_left_out {
        notes.push(format!(
            "more than {MAX_SNMP_METRICS} numbers: the rest have no history and no limits"
        ));
    }
    metrics.push(Metric::new(metric::ANSWERS, f64::from(seen_values), ""));
    if !problems.is_empty() {
        return Outcome::down(listed(problems, PROBLEMS_NAMED, "; ")).with_metrics(metrics);
    }
    let mut message = listed(shown, VALUES_NAMED, ", ");
    for note in notes {
        message.push_str(&format!(" ({note})"));
    }
    Outcome::up(message).with_metrics(metrics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::{Health, SnmpLimit};
    use std::collections::BTreeMap;
    use std::ops::Bound;
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

    /// A whole number as BER writes it: as few bytes as its sign allows.
    fn integer(n: i64) -> Vec<u8> {
        let bytes = n.to_be_bytes();
        let mut start = 0;
        while start < 7
            && ((bytes[start] == 0 && bytes[start + 1] & 0x80 == 0)
                || (bytes[start] == 0xff && bytes[start + 1] & 0x80 != 0))
        {
            start += 1;
        }
        bytes[start..].to_vec()
    }

    fn number(bytes: &[u8]) -> usize {
        bytes.iter().fold(0, |n, b| (n << 8) | *b as usize)
    }

    /// An OID's numbers from its bytes.
    fn arcs_of(bytes: &[u8]) -> Vec<u64> {
        let mut arcs = vec![u64::from(bytes[0] / 40), u64::from(bytes[0] % 40)];
        let mut arc = 0u64;
        for byte in &bytes[1..] {
            arc = (arc << 7) | u64::from(byte & 0x7f);
            if byte & 0x80 == 0 {
                arcs.push(arc);
                arc = 0;
            }
        }
        arcs
    }

    fn oid_bytes(arcs: &[u64]) -> Vec<u8> {
        Oid::from(arcs).unwrap().as_bytes().to_vec()
    }

    fn varbind(arcs: &[u64], value: &[u8]) -> Vec<u8> {
        tlv(
            0x30,
            &[tlv(0x06, &oid_bytes(arcs)), value.to_vec()].concat(),
        )
    }

    type Mib = BTreeMap<Vec<u64>, Vec<u8>>;

    /// The agent's answer to `request`, or none (another community: a real
    /// agent stays silent). It knows GET and GETBULK.
    fn answer(request: &[u8], community: &[u8], mib: &Mib) -> Option<Vec<u8>> {
        let (_, message, _) = read(request);
        let (_, version, rest) = read(message);
        let (_, asked_community, rest) = read(rest);
        if asked_community != community {
            return None;
        }
        let (kind, pdu, _) = read(rest);
        let (_, request_id, rest) = read(pdu);
        // GET: error status and index. GETBULK: non-repeaters, repetitions.
        let (_, _, rest) = read(rest);
        let (_, repetitions, rest) = read(rest);
        let (_, mut varbinds, _) = read(rest);
        let mut answered = Vec::new();
        while !varbinds.is_empty() {
            let (_, asked, next) = read(varbinds);
            let (_, oid, _) = read(asked);
            let oid = arcs_of(oid);
            if kind == 0xA5 {
                let mut following = mib
                    .range((Bound::Excluded(oid.clone()), Bound::Unbounded))
                    .take(number(repetitions));
                let mut last = oid;
                for _ in 0..number(repetitions) {
                    match following.next() {
                        Some((oid, value)) => {
                            answered.extend(varbind(oid, value));
                            last = oid.clone();
                        }
                        // 0x82: endOfMibView.
                        None => {
                            answered.extend(varbind(&last, &tlv(0x82, &[])));
                            break;
                        }
                    }
                }
            } else {
                // 0x80: noSuchObject.
                let value = mib.get(&oid).cloned().unwrap_or_else(|| tlv(0x80, &[]));
                answered.extend(varbind(&oid, &value));
            }
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
    async fn agent(community: &'static str, values: Vec<(String, Vec<u8>)>) -> u16 {
        let mib: Mib = values
            .into_iter()
            .map(|(oid, value)| (oid_arcs(&oid).unwrap(), value))
            .collect();
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = socket.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut buffer = [0u8; 4096];
            while let Ok((len, from)) = socket.recv_from(&mut buffer).await {
                if let Some(reply) = answer(&buffer[..len], community.as_bytes(), &mib) {
                    let _ = socket.send_to(&reply, from).await;
                }
            }
        });
        port
    }

    fn int(n: i64) -> Vec<u8> {
        tlv(0x02, &integer(n))
    }

    fn text(text: &str) -> Vec<u8> {
        tlv(0x04, text.as_bytes())
    }

    const UPTIME: &str = "1.3.6.1.2.1.1.3.0";
    const NAME: &str = "1.3.6.1.2.1.1.5.0";
    const LINK: &str = "1.3.6.1.2.1.2.2.1.8.1";
    const TRAFFIC: &str = "1.3.6.1.2.1.31.1.1.1.6.1";
    // A printer's supplies: what each is, how much it holds, how much is left.
    const SUPPLY_NAME: &str = "1.3.6.1.2.1.43.11.1.1.6";
    const SUPPLY_MAX: &str = "1.3.6.1.2.1.43.11.1.1.8";
    const SUPPLY_LEVEL: &str = "1.3.6.1.2.1.43.11.1.1.9";
    // A switch's ports: their state and their names.
    const PORT_STATE: &str = "1.3.6.1.2.1.2.2.1.8";
    const PORT_NAME: &str = "1.3.6.1.2.1.31.1.1.1.1";

    /// A device that is a bit of everything: a switch with six ports and
    /// a printer's three supplies.
    async fn device() -> u16 {
        let mut values = vec![
            // TimeTicks: 123456 hundredths of a second.
            (UPTIME.to_string(), tlv(0x43, &integer(123_456))),
            (NAME.to_string(), text("core-switch")),
            // Counter64.
            (TRAFFIC.to_string(), tlv(0x46, &integer(4_000_000_000))),
        ];
        for (row, (name, max, level)) in [
            ("Black Toner", 100, 85),
            ("Cyan Toner", 2000, 80),
            // -2 and -3: the printer doesn't know.
            ("Waste Box", -2, -3),
        ]
        .into_iter()
        .enumerate()
        {
            let row = row + 1;
            values.push((format!("{SUPPLY_NAME}.1.{row}"), text(name)));
            values.push((format!("{SUPPLY_MAX}.1.{row}"), int(max)));
            values.push((format!("{SUPPLY_LEVEL}.1.{row}"), int(level)));
        }
        // Ports 1 to 6; 3 and 5 are down.
        for port in 1..=6 {
            let state = if port == 3 || port == 5 { 2 } else { 1 };
            values.push((format!("{PORT_STATE}.{port}"), int(state)));
            values.push((
                format!("{PORT_NAME}.{port}"),
                text(&format!("ge-0/0/{port}")),
            ));
        }
        agent("s3cret", values).await
    }

    fn value(oid: &str, name: &str, expect: Option<&str>) -> SnmpOid {
        SnmpOid {
            name: name.into(),
            expect: expect.map(str::to_string).into_iter().collect(),
            ..SnmpOid::of(oid)
        }
    }

    /// A value as the check form writes it.
    fn line(text: &str) -> SnmpOid {
        let value = common::snmp::parse_value(text).unwrap();
        value.validate().unwrap();
        value
    }

    async fn ask_v2c(port: u16, community: &str, oids: &[SnmpOid]) -> Outcome {
        let secrets = Secrets(BTreeMap::from([(
            secret::COMMUNITY.to_string(),
            community.to_string(),
        )]));
        let query = Query {
            check_id: "test",
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

    fn metrics(outcome: &Outcome) -> Vec<(&str, f64, &str)> {
        outcome
            .metrics
            .iter()
            .map(|m| (m.key.as_str(), m.value, m.unit.as_str()))
            .collect()
    }

    #[tokio::test]
    async fn an_agent_answers_with_its_values_and_numbers_become_metrics() {
        let port = device().await;
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
        assert_eq!(
            metrics(&outcome),
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
        let port = device().await;
        let down = ask_v2c(port, "s3cret", &[value(LINK, "Uplink", Some("2"))]).await;
        assert_eq!(down.status, Health::Down);
        assert_eq!(down.message, "Uplink is 1 (expected 2)");
        // The value is still a metric: the hub keeps its history.
        assert_eq!(down.metrics[0].key, "Uplink");

        let text = ask_v2c(port, "s3cret", &[value(NAME, "Name", Some("Core-Switch"))]).await;
        assert_eq!(text.status, Health::Up, "{}", text.message);

        // One of several accepted values; and a value that must not be.
        let either = ask_v2c(port, "s3cret", &[line(&format!("Uplink = {LINK} = 5 | 1"))]).await;
        assert_eq!(either.status, Health::Up, "{}", either.message);
        let neither = ask_v2c(port, "s3cret", &[line(&format!("Uplink = {LINK} = 5 | 6"))]).await;
        assert_eq!(neither.message, "Uplink is 1 (expected 5 or 6)");
        let forbidden = ask_v2c(
            port,
            "s3cret",
            &[line(&format!(
                "Uplink = {LINK} ; means 1=up, 2=down ; not UP"
            ))],
        )
        .await;
        assert_eq!(forbidden.status, Health::Down);
        assert_eq!(forbidden.message, "Uplink is up");

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
    async fn a_table_is_walked_and_each_row_is_a_share_of_its_capacity() {
        let port = device().await;
        let toner = line(&format!(
            "Supply = {SUPPLY_LEVEL} ; rows {SUPPLY_NAME} ; of {SUPPLY_MAX} ; warn < 20 ; down < 5"
        ));
        assert_eq!(toner.warn, Some(SnmpLimit::Under(20.0)));
        let outcome = ask_v2c(port, "s3cret", &[toner]).await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert_eq!(
            outcome.message,
            "Supply: Black Toner 85 %, Supply: Cyan Toner 4 %, Supply: Waste Box unknown"
        );
        // What the printer doesn't know is no number: nothing to judge.
        assert_eq!(
            metrics(&outcome),
            [
                ("Supply: Black Toner", 85.0, "%"),
                ("Supply: Cyan Toner", 4.0, "%"),
                (metric::ANSWERS, 3.0, ""),
            ]
        );

        // Without a column of names, a row goes by its index.
        let plain = ask_v2c(
            port,
            "s3cret",
            &[line(&format!("Level = {SUPPLY_LEVEL} ; walk"))],
        )
        .await;
        assert_eq!(plain.message, "Level: 1.1 85, Level: 1.2 80, Level: 1.3 -3");
    }

    #[tokio::test]
    async fn the_states_of_a_table_are_counted_and_none_may_be_a_forbidden_one() {
        let port = device().await;
        let states =
            format!("Port = {PORT_STATE} ; rows {PORT_NAME} ; means 1=up, 2=down, 6=not present");
        let outcome = ask_v2c(port, "s3cret", &[line(&states)]).await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert_eq!(outcome.message, "Port: 4 up, 2 down");
        // A state is no measure: only how many values were read is kept.
        assert_eq!(metrics(&outcome), [(metric::ANSWERS, 6.0, "")]);

        let strict = ask_v2c(port, "s3cret", &[line(&format!("{states} ; not down"))]).await;
        assert_eq!(strict.status, Health::Down);
        assert_eq!(
            strict.message,
            "Port: ge-0/0/3 is down; Port: ge-0/0/5 is down"
        );
        let all_up = ask_v2c(
            port,
            "s3cret",
            &[line(&states.replacen(" ; rows", " = up ; rows", 1))],
        )
        .await;
        assert_eq!(
            all_up.message,
            "Port: ge-0/0/3 is down (expected up); Port: ge-0/0/5 is down (expected up)"
        );

        // A table the agent doesn't have.
        let none = ask_v2c(
            port,
            "s3cret",
            &[line("Disk = 1.3.6.1.4.1.6574.2.1.1.5 ; walk")],
        )
        .await;
        assert_eq!(none.status, Health::Down);
        assert_eq!(none.message, "Disk: the agent has no such table");
    }

    #[tokio::test]
    async fn a_wrong_community_gets_no_answer_and_the_message_says_what_to_check() {
        let port = device().await;
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

    fn seen(value: &SnmpOid, raw: &Reading) -> Seen {
        see(value, raw, None, |_| None).unwrap()
    }

    #[test]
    fn values_are_read_as_numbers_or_text() {
        let plain = SnmpOid::of(UPTIME);
        let shown = |value: &Value<'_>| seen(&plain, &reading(value));
        assert_eq!(reading(&Value::Integer(-5)), Reading::Number(-5.0, ""));
        assert_eq!(shown(&Value::Timeticks(250)).text, "2.5 s");
        assert_eq!(shown(&Value::Timeticks(250)).bare, "2.5");
        assert_eq!(shown(&Value::Counter64(7)).text, "7");
        assert_eq!(
            shown(&Value::OctetString(b" HP LaserJet\r\n")).text,
            "HP LaserJet"
        );
        assert_eq!(
            shown(&Value::OctetString(&[0x00, 0x1b, 0x21, 0xff, 0x0a, 0x01])).text,
            "00:1b:21:ff:0a:01",
            "a MAC address isn't text"
        );
        assert_eq!(
            shown(&Value::IpAddress([192, 168, 1, 1])).text,
            "192.168.1.1"
        );
        assert_eq!(reading(&Value::NoSuchInstance), Reading::Missing);
        assert_eq!(see(&plain, &Reading::Missing, None, |_| None), None);
        let long = "x".repeat(200);
        assert_eq!(
            shown(&Value::OctetString(long.as_bytes()))
                .text
                .chars()
                .count(),
            MAX_TEXT_CHARS + 1
        );
    }

    #[test]
    fn a_value_is_scaled_given_its_unit_and_its_meaning() {
        // Seconds (from TimeTicks) to minutes.
        let runtime = line("Runtime = 1.3.6.1.4.1.318.1.1.1.2.2.3.0 ; / 60 ; unit min");
        let shown = seen(&runtime, &Reading::Number(5400.0, "s"));
        assert_eq!(shown.text, "90 min");
        assert_eq!(shown.number, Some((90.0, "min".to_string())));

        let tenths = line("1.3.6.1.4.1.9.1.0 ; x 0.1 ; unit °C");
        let shown = seen(&tenths, &Reading::Number(234.0, ""));
        assert_eq!(
            (shown.text.as_str(), shown.bare.as_str()),
            ("23.4 °C", "23.4")
        );

        // A state has a meaning and no number; an unknown one stays as it is.
        let battery =
            line("Battery = 1.3.6.1.4.1.318.1.1.1.2.1.1.0 = normal ; means 2=normal, 3=low");
        let low = seen(&battery, &Reading::Number(3.0, ""));
        assert_eq!(
            (low.text.as_str(), low.bare.as_str(), &low.number),
            ("low", "3", &None)
        );
        assert_eq!(
            problem(&battery, "Battery", &low).unwrap(),
            "Battery is low (expected normal)"
        );
        let normal = seen(&battery, &Reading::Number(2.0, ""));
        assert_eq!(problem(&battery, "Battery", &normal), None);
        let odd = seen(&battery, &Reading::Number(9.0, ""));
        assert_eq!(odd.text, "9");
        assert!(problem(&battery, "Battery", &odd).is_some());
        // Text has meanings too.
        let smart = line("1.3.6.1.4.1.24681.1.3.11.1.7.1 ; means GOOD=healthy");
        assert_eq!(seen(&smart, &Reading::Text("GOOD".into())).text, "healthy");

        // A share of what holds nothing, or of nothing at all: unknown.
        let share = line("1.3.6.1.4.1.9.1.0 ; of 1.3.6.1.4.1.9.2.0");
        let of = |level: f64, whole: Option<Reading>| {
            see(&share, &Reading::Number(level, ""), whole.as_ref(), |_| {
                None
            })
            .unwrap()
        };
        assert_eq!(of(250.0, Some(Reading::Number(1000.0, ""))).text, "25 %");
        assert_eq!(of(1.0, Some(Reading::Number(3.0, ""))).text, "33.33 %");
        assert_eq!(of(250.0, Some(Reading::Number(0.0, ""))).text, "unknown");
        assert_eq!(of(250.0, Some(Reading::Missing)).number, None);
        assert_eq!(of(250.0, None).text, "unknown");
    }

    fn answers(values: &[(&str, Reading)], columns: &[(&str, Vec<Reading>)]) -> Answers {
        Answers {
            values: values
                .iter()
                .map(|(oid, reading)| (oid.to_string(), reading.clone()))
                .collect(),
            columns: columns
                .iter()
                .map(|(oid, rows)| {
                    let rows = rows
                        .iter()
                        .enumerate()
                        .map(|(row, reading)| ((row + 1).to_string(), reading.clone()))
                        .collect();
                    (oid.to_string(), rows)
                })
                .collect(),
        }
    }

    #[test]
    fn a_counter_becomes_a_rate_from_its_second_reading_on() {
        let traffic = [line(&format!(
            "In = {TRAFFIC} ; per second ; x 0.000008 ; unit Mbit/s"
        ))];
        let mut rates = Rates::default();
        let start = Instant::now();
        let read = |counter: f64, after: u64, rates: &mut Rates| {
            judge(
                "c1",
                &traffic,
                &answers(&[(TRAFFIC, Reading::Number(counter, ""))], &[]),
                rates,
                start + Duration::from_secs(after),
            )
        };
        let first = read(1_000_000.0, 0, &mut rates);
        assert_eq!(first.status, Health::Up);
        assert_eq!(first.message, "In measuring");
        assert_eq!(metrics(&first), [(metric::ANSWERS, 1.0, "")]);
        // 75 MB more in a minute: 10 Mbit/s.
        let second = read(76_000_000.0, 60, &mut rates);
        assert_eq!(second.message, "In 10 Mbit/s");
        assert!((second.metrics[0].value - 10.0).abs() < 1e-9);
        // The device restarted: its counter starts again, and so does the rate.
        let restarted = read(5_000.0, 120, &mut rates);
        assert_eq!(restarted.message, "In measuring");
        assert_eq!(read(5_000.0, 180, &mut rates).message, "In 0 Mbit/s");

        // Another check's counter of the same name is another counter.
        let other = judge(
            "c2",
            &traffic,
            &answers(&[(TRAFFIC, Reading::Number(9.0, ""))], &[]),
            &mut rates,
            start + Duration::from_secs(200),
        );
        assert_eq!(other.message, "In measuring");
        // What wasn't read for an hour is forgotten.
        assert_eq!(rates.0.len(), 2);
        rates.forget_old(start + Duration::from_secs(200) + RATE_MEMORY);
        assert!(rates.0.is_empty());
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
        let read: Vec<(String, Reading)> = asked
            .iter()
            .map(|value| (value.oid.clone(), Reading::Number(1.0, "")))
            .collect();
        let read: Vec<(&str, Reading)> =
            read.iter().map(|(o, r)| (o.as_str(), r.clone())).collect();
        let outcome = judge(
            "c",
            &asked,
            &answers(&read, &[]),
            &mut Rates::default(),
            Instant::now(),
        );
        assert_eq!(outcome.status, Health::Up);
        assert!(
            outcome.message.ends_with("Port 5 1 and 3 more"),
            "{}",
            outcome.message
        );
        assert_eq!(outcome.metrics.len(), 10);
    }

    #[test]
    fn a_long_table_is_cut_and_only_so_many_numbers_are_kept() {
        const TEMPERATURE: &str = "1.3.6.1.4.1.9.9.13.1.3.1.3";
        const SPEED: &str = "1.3.6.1.4.1.9.9.13.1.4.1.3";
        let asked = [
            line(&format!("Temperature = {TEMPERATURE} ; walk ; unit °C")),
            line(&format!("Fan = {SPEED} ; walk ; unit RPM")),
        ];
        let column = |from: f64| -> Vec<Reading> {
            (0..=MAX_SNMP_ROWS)
                .map(|row| Reading::Number(from + row as f64, ""))
                .collect()
        };
        let outcome = judge(
            "c",
            &asked,
            &answers(&[], &[(TEMPERATURE, column(30.0)), (SPEED, column(1000.0))]),
            &mut Rates::default(),
            Instant::now(),
        );
        assert_eq!(outcome.status, Health::Up);
        assert_eq!(
            outcome.message,
            "Temperature: 64 rows, 30 to 93 °C, Fan: 64 rows, 1000 to 1063 RPM (Temperature: the first 64 rows only) (Fan: the first 64 rows only) (more than 100 numbers: the rest have no history and no limits)"
        );
        // The numbers kept, and how many values were read.
        assert_eq!(outcome.metrics.len(), MAX_SNMP_METRICS + 1);
        assert_eq!(outcome.metrics.last().unwrap().value, 128.0);
        assert_eq!(outcome.metrics[64].key, "Fan: 1");

        // Many problems are counted too.
        let strict = [line(&format!("Temperature = {TEMPERATURE} = 0 ; walk"))];
        let down = judge(
            "c",
            &strict,
            &answers(&[], &[(TEMPERATURE, column(30.0))]),
            &mut Rates::default(),
            Instant::now(),
        );
        assert_eq!(down.status, Health::Down);
        assert!(down.message.ends_with(" and 59 more"), "{}", down.message);
    }

    #[test]
    fn each_oid_is_asked_for_once() {
        let asked = [
            line(&format!(
                "Supply = {SUPPLY_LEVEL} ; rows {SUPPLY_NAME} ; of {SUPPLY_MAX}"
            )),
            line(&format!("Capacity = {SUPPLY_MAX} ; rows {SUPPLY_NAME}")),
            line(&format!("Used = {UPTIME} ; of {NAME}")),
            line(&format!("Name = {NAME}")),
        ];
        let plan = plan(&asked).unwrap();
        let arcs = |oid: &str| oid_arcs(oid).unwrap();
        assert_eq!(
            plan.columns,
            [arcs(SUPPLY_LEVEL), arcs(SUPPLY_NAME), arcs(SUPPLY_MAX)]
        );
        assert_eq!(plan.values, [arcs(UPTIME), arcs(NAME)]);
        assert_eq!(super::plan(&[SnmpOid::of("sysUpTime.0")]), None);
    }

    /// A real agent, for the v3 exchange no fake here speaks: run by hand
    /// (`--ignored`) with `SNMP_TEST_HOST`, `SNMP_TEST_USER`,
    /// `SNMP_TEST_AUTH` (`sha1`, …), `SNMP_TEST_AUTH_PASSWORD`, and for
    /// privacy `SNMP_TEST_PRIV` and `SNMP_TEST_PRIV_PASSWORD` (D-86). It
    /// also walks the agent's interfaces (D-87).
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
                check_id: "real",
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

        // Its interfaces, as a table: their states by name, and their traffic.
        let table = [
            line(&format!(
                "Port = {PORT_STATE} ; rows {PORT_NAME} ; means 1=up, 2=down"
            )),
            line("In = 1.3.6.1.2.1.31.1.1.1.6 ; rows 1.3.6.1.2.1.31.1.1.1.1 ; per second"),
        ];
        let walked = check(query(&host, &user, how, &table, &secrets)).await;
        println!("walk: {} | {:?}", walked.message, walked.metrics);
        assert_eq!(walked.status, Health::Up, "{}", walked.message);
        assert!(walked.message.starts_with("Port: "), "{}", walked.message);
        tokio::time::sleep(Duration::from_secs(2)).await;
        let again = check(query(&host, &user, how, &table, &secrets)).await;
        println!("walk again: {} | {:?}", again.message, again.metrics);
        assert!(again.metrics.len() > 1, "rates from the second reading on");

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
