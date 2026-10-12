//! Checks as the hub defines them and the probe runs them (M2-04, proposal
//! 0003 §4.1, D-05, D-15, D-49).
//!
//! - [`CheckSpec`] is what to check, one variant per kind with typed
//!   parameters, and [`CheckSpec::validate`] is the single validation: the hub
//!   API, the hub UI and the probe all call it.
//! - [`ProbeConfig`] is everything a probe runs: its report interval and its
//!   checks. The hub sends it in its signed answer to a report
//!   ([`crate::ack::ReportAck::config`]) and the probe caches it on disk.
//! - [`CheckResult`] is one check's outcome in a report, keyed by its stable
//!   [`CheckId`], with any number of [`Metric`]s (proposal 0006 §1.4).
//!
//! Depends only on `serde` and `std`, so the hub's web UI can use it too.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::net::IpAddr;
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

/// The stable identity of a check: a random ID the hub gives it when it is
/// created, kept when it is renamed or retargeted. Results, alert state and
/// silences are keyed by it (D-15).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CheckId(pub String);

impl CheckId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CheckId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a check looks at. JSON: `{"kind": "tcp", "host": "10.0.0.5", "port": 443}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckSpec {
    /// ICMP echoes to a host (an IP address, or a name the probe's machine
    /// resolves): answered or lost, and how fast.
    Ping {
        host: String,
        /// How many echoes one run sends.
        #[serde(
            default = "default_ping_count",
            skip_serializing_if = "is_default_ping_count"
        )]
        count: u8,
        /// A TCP port to try when no echo is answered (a firewall that
        /// drops them), or when the probe may not send any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tcp_port: Option<u16>,
    },
    /// Request a URL; up when the answer is what [`HttpOptions`] expects
    /// (by default: a `GET` answered 2xx).
    Http {
        url: String,
        #[serde(flatten)]
        options: HttpOptions,
    },
    /// Values an SNMP agent has (a switch, a printer, a NAS, a UPS): up
    /// when it answers with every value asked for, as expected. What opens
    /// the agent (the community, the v3 passwords) is not here: those are
    /// the check's [`Secrets`].
    Snmp {
        host: String,
        #[serde(
            default = "default_snmp_port",
            skip_serializing_if = "is_default_snmp_port"
        )]
        port: u16,
        #[serde(default, skip_serializing_if = "is_default")]
        version: SnmpVersion,
        /// v3: the user name.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        user: String,
        /// v3: how the user is authenticated. `None`: not at all.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth: Option<SnmpAuth>,
        /// v3: how the exchange is encrypted (needs `auth`). `None`: it isn't.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        privacy: Option<SnmpPrivacy>,
        /// The values to ask for.
        oids: Vec<SnmpOid>,
    },
    /// Open a TCP connection.
    Tcp { host: String, port: u16 },
    /// CPU usage of the probe's machine.
    Cpu,
    /// Memory usage of the probe's machine.
    Memory,
    /// Usage of one of the probe machine's disks, by mount point (`/`,
    /// `C:\`). Without a path: the system disk.
    Disk {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// Ask DNS for a name: does it resolve, and to what it should?
    Dns {
        name: String,
        #[serde(default, skip_serializing_if = "is_default")]
        record: DnsRecord,
        /// The DNS server to ask, as an IP address. `None`: the ones the
        /// probe's machine uses.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        server: Option<String>,
        /// The answers that are fine (addresses, names, text). Empty: any
        /// answer; else an answer that isn't listed fails the check.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        expect: Vec<String>,
    },
    /// Every fixed disk of the probe's machine, found each time the check
    /// runs: the fullest one's usage counts.
    Disks {
        /// Mount points to leave out: `E:`, `/mnt/scratch`, `/mnt/*`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ignore: Vec<String>,
    },
    /// The newest file at a path on the probe's machine (or a share it can
    /// read): how old it is and how big. For backups that must have run.
    File {
        /// A file, or a folder whose newest file counts.
        path: String,
        /// In a folder, the names that count: `*.bak`, `backup-*.zip`
        /// (`*` any characters, `?` any one). Empty: every file.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pattern: String,
        /// Look in the folder's subfolders too.
        #[serde(default, skip_serializing_if = "is_default")]
        recursive: bool,
        /// The newest file must be at least this big, in bytes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_bytes: Option<u64>,
    },
    /// A service of the probe's machine: is it running? A Windows service
    /// (its service name: `Spooler`), or a systemd unit (`nginx`,
    /// `backup.timer`).
    Service { name: String },
    /// How long the probe's machine has been running.
    Uptime,
    /// The certificate a TLS server presents (HTTPS, LDAPS, SMTPS, …): how
    /// long until it expires, and whether a client would accept it.
    Tls {
        host: String,
        port: u16,
        /// `false`: only the dates count, not who issued it or for which
        /// name (a self-signed certificate, a private authority).
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        verify: bool,
    },
}

fn yes() -> bool {
    true
}

/// The UDP port SNMP agents listen on.
pub const DEFAULT_SNMP_PORT: u16 = 161;
/// The most values one `snmp` check asks for.
pub const MAX_SNMP_OIDS: usize = 20;
/// The longest name of a value, and of a v3 user.
pub const MAX_SNMP_NAME_CHARS: usize = 60;
/// The longest secret of a check, and the shortest SNMP v3 password (RFC 3414).
pub const MAX_SECRET_CHARS: usize = 200;
pub const MIN_SNMP_PASSWORD_CHARS: usize = 8;

/// The names of an `snmp` check's secrets.
pub mod secret {
    /// v2c: the community.
    pub const COMMUNITY: &str = "community";
    /// v3: the user's authentication password.
    pub const AUTH_PASSWORD: &str = "auth_password";
    /// v3: the privacy (encryption) password.
    pub const PRIV_PASSWORD: &str = "priv_password";
}

fn default_snmp_port() -> u16 {
    DEFAULT_SNMP_PORT
}

fn is_default_snmp_port(port: &u16) -> bool {
    *port == DEFAULT_SNMP_PORT
}

/// Which SNMP an agent is asked in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnmpVersion {
    /// A community opens the agent, sent in the clear.
    #[default]
    V2c,
    /// A user, authenticated and encrypted if the check says so.
    V3,
}

/// How an SNMP v3 user is authenticated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnmpAuth {
    Md5,
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

/// How an SNMP v3 exchange is encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnmpPrivacy {
    Des,
    Aes128,
    Aes192,
    Aes256,
}

/// The names as in the JSON, with what the form shows.
macro_rules! named {
    ($type:ident: $($variant:ident => $name:literal, $label:literal;)+) => {
        impl $type {
            pub const ALL: &'static [$type] = &[$($type::$variant),+];
            pub fn as_str(self) -> &'static str {
                match self { $($type::$variant => $name),+ }
            }
            pub fn label(self) -> &'static str {
                match self { $($type::$variant => $label),+ }
            }
            pub fn parse(text: &str) -> Option<Self> {
                match text { $($name => Some($type::$variant),)+ _ => None }
            }
        }
    };
}

named!(SnmpVersion:
    V2c => "v2c", "v2c (a community)";
    V3 => "v3", "v3 (a user)";
);
named!(SnmpAuth:
    Md5 => "md5", "MD5";
    Sha1 => "sha1", "SHA-1";
    Sha224 => "sha224", "SHA-224";
    Sha256 => "sha256", "SHA-256";
    Sha384 => "sha384", "SHA-384";
    Sha512 => "sha512", "SHA-512";
);
named!(SnmpPrivacy:
    Des => "des", "DES";
    Aes128 => "aes128", "AES-128";
    Aes192 => "aes192", "AES-192";
    Aes256 => "aes256", "AES-256";
);

/// One value an `snmp` check asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnmpOid {
    /// Its object identifier in numbers: `1.3.6.1.2.1.1.3.0`.
    pub oid: String,
    /// What to call it in results. Empty: its OID.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// What it must be, as text (`1`, `up`). `None`: any value is fine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<String>,
}

impl SnmpOid {
    /// A value by its OID alone: no name, anything is fine.
    pub fn of(oid: &str) -> Self {
        Self {
            oid: oid.to_string(),
            name: String::new(),
            expect: None,
        }
    }

    /// The name results use: its own, else its OID.
    pub fn label(&self) -> &str {
        if self.name.is_empty() {
            &self.oid
        } else {
            &self.name
        }
    }

    /// The OID's numbers; `None` if it isn't one. A leading dot is allowed.
    pub fn arcs(&self) -> Option<Vec<u64>> {
        let arcs: Vec<u64> = self
            .oid
            .strip_prefix('.')
            .unwrap_or(&self.oid)
            .split('.')
            .map(|arc| {
                (!arc.is_empty() && arc.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| arc.parse::<u32>().ok())
                    .flatten()
                    .map(u64::from)
            })
            .collect::<Option<_>>()?;
        // Two numbers at least; the first is 0, 1 or 2, and under 0 and 1
        // the second is below 40 (how OIDs are encoded).
        let valid =
            (2..=128).contains(&arcs.len()) && arcs[0] <= 2 && (arcs[0] == 2 || arcs[1] < 40);
        valid.then_some(arcs)
    }
}

/// What a check needs that nobody may read back (D-86): a community, a
/// password. By name ([`secret`]). The hub seals them in its database and
/// never shows them; the probe gets them with its checks and keeps them in
/// its private folder. `Debug` shows the names only.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secrets(pub BTreeMap<String, String>);

impl fmt::Debug for Secrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.0.keys()).finish()
    }
}

impl Secrets {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// The names that have a value, in order.
    pub fn names(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }
}

/// The echoes a `ping` check sends when it doesn't say.
pub const DEFAULT_PING_COUNT: u8 = 4;
/// The most echoes a `ping` check sends: they must fit in a check's time.
pub const MAX_PING_COUNT: u8 = 20;

fn default_ping_count() -> u8 {
    DEFAULT_PING_COUNT
}

fn is_default_ping_count(count: &u8) -> bool {
    *count == DEFAULT_PING_COUNT
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// How an `http` check asks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpMethod {
    #[default]
    Get,
    /// Headers only: lighter, but there is no body to search.
    Head,
}

impl HttpMethod {
    /// `get`, `head`: as in the JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            HttpMethod::Get => "get",
            HttpMethod::Head => "head",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "get" => Some(HttpMethod::Get),
            "head" => Some(HttpMethod::Head),
            _ => None,
        }
    }
}

/// A header an `http` check sends. Not for secrets: everyone who sees the
/// check sees it (D-78).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

/// What an `http` check sends and expects, beyond its URL. Each is left out
/// of the JSON at its default, so `{"kind": "http", "url": …}` is a plain
/// `GET` that must answer 2xx.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpOptions {
    #[serde(default, skip_serializing_if = "is_default")]
    pub method: HttpMethod,
    /// The status codes that count as up, e.g. `200-299, 401`
    /// ([`status_ranges`]). Empty: [`DEFAULT_HTTP_STATUS`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
    /// Text the answer's body must contain (as written: case matters).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub contains: String,
    /// Text the answer's body must not contain.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub absent: String,
    /// `false`: a redirect is the answer (expect its 3xx status).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub follow_redirects: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<HttpHeader>,
    /// `false`: any certificate will do over HTTPS.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub verify: bool,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            method: HttpMethod::Get,
            status: String::new(),
            contains: String::new(),
            absent: String::new(),
            follow_redirects: true,
            headers: Vec::new(),
            verify: true,
        }
    }
}

/// The status codes an `http` check expects when it names none.
pub const DEFAULT_HTTP_STATUS: &str = "200-299";
/// The most status codes or ranges an `http` check lists.
pub const MAX_STATUS_RANGES: usize = 20;
/// The longest text an `http` check looks for, in characters.
pub const MAX_KEYWORD_CHARS: usize = 200;
/// The most headers an `http` check sends.
pub const MAX_HTTP_HEADERS: usize = 10;

/// The status codes `text` names: codes and ranges separated by commas
/// (`200`, `200-299, 401`). Empty: [`DEFAULT_HTTP_STATUS`].
pub fn status_ranges(text: &str) -> Result<Vec<RangeInclusive<u16>>, CheckError> {
    let text = match text.trim() {
        "" => DEFAULT_HTTP_STATUS,
        text => text,
    };
    let code = |part: &str| {
        part.trim()
            .parse::<u16>()
            .ok()
            .filter(|code| (100..=599).contains(code))
            .ok_or_else(|| CheckError(format!("\"{}\" isn't a status code (100-599)", part.trim())))
    };
    let ranges = text
        .split(',')
        .map(|part| {
            let (from, to) = match part.split_once('-') {
                Some((from, to)) => (code(from)?, code(to)?),
                None => (code(part)?, code(part)?),
            };
            if from > to {
                return Err(CheckError(format!(
                    "the status range {from}-{to} is backwards"
                )));
            }
            Ok(from..=to)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if ranges.len() > MAX_STATUS_RANGES {
        return Err(CheckError(format!(
            "more than {MAX_STATUS_RANGES} status codes or ranges"
        )));
    }
    Ok(ranges)
}

impl HttpOptions {
    /// Is `status` one this check expects? (Nothing is, if the list is invalid.)
    pub fn expects(&self, status: u16) -> bool {
        status_ranges(&self.status).is_ok_and(|ranges| ranges.iter().any(|r| r.contains(&status)))
    }

    /// The expected status codes as written, or the default.
    pub fn expected_status(&self) -> &str {
        match self.status.trim() {
            "" => DEFAULT_HTTP_STATUS,
            status => status,
        }
    }

    /// Does the check read the answer's body?
    pub fn searches_body(&self) -> bool {
        !self.contains.is_empty() || !self.absent.is_empty()
    }

    fn validate(&self) -> Result<(), CheckError> {
        status_ranges(&self.status)?;
        for keyword in [&self.contains, &self.absent] {
            if keyword.chars().count() > MAX_KEYWORD_CHARS {
                return Err(CheckError(format!(
                    "the text to look for is longer than {MAX_KEYWORD_CHARS} characters"
                )));
            }
        }
        if self.method == HttpMethod::Head && self.searches_body() {
            return Err(CheckError::new(
                "a HEAD request has no body to look for text in: use GET",
            ));
        }
        if self.headers.len() > MAX_HTTP_HEADERS {
            return Err(CheckError(format!("more than {MAX_HTTP_HEADERS} headers")));
        }
        for header in &self.headers {
            let name_ok = !header.name.is_empty()
                && header
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !name_ok {
                return Err(CheckError(format!(
                    "\"{}\" isn't a header name (letters, digits, - and _)",
                    header.name
                )));
            }
            let value_ok = header.value.chars().count() <= MAX_TARGET_CHARS
                && header.value.trim() == header.value
                && !header.value.chars().any(char::is_control);
            if !value_ok {
                return Err(CheckError(format!(
                    "the value of the header {} has spaces at its ends, control characters, or is too long",
                    header.name
                )));
            }
        }
        Ok(())
    }
}

/// Every kind, in the order the UI offers them.
pub const CHECK_KINDS: [&str; 13] = [
    "ping", "http", "tcp", "tls", "dns", "snmp", "cpu", "memory", "disk", "disks", "file",
    "service", "uptime",
];

/// The longest service name (Windows allows 256 characters).
pub const MAX_SERVICE_NAME_CHARS: usize = 256;

/// The kind of record a `dns` check asks for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsRecord {
    /// IPv4 addresses.
    #[default]
    A,
    /// IPv6 addresses.
    Aaaa,
    /// The name this one is an alias of.
    Cname,
    /// Mail servers, each with its preference (`10 mail.example.com`).
    Mx,
    /// Text (SPF, verification strings).
    Txt,
    /// The name servers of a domain.
    Ns,
}

impl DnsRecord {
    pub const ALL: [DnsRecord; 6] = [
        DnsRecord::A,
        DnsRecord::Aaaa,
        DnsRecord::Cname,
        DnsRecord::Mx,
        DnsRecord::Txt,
        DnsRecord::Ns,
    ];

    /// `a`, `aaaa`, …: as in the JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            DnsRecord::A => "a",
            DnsRecord::Aaaa => "aaaa",
            DnsRecord::Cname => "cname",
            DnsRecord::Mx => "mx",
            DnsRecord::Txt => "txt",
            DnsRecord::Ns => "ns",
        }
    }

    /// `A`, `AAAA`, …: as people write them.
    pub fn label(self) -> &'static str {
        match self {
            DnsRecord::A => "A",
            DnsRecord::Aaaa => "AAAA",
            DnsRecord::Cname => "CNAME",
            DnsRecord::Mx => "MX",
            DnsRecord::Txt => "TXT",
            DnsRecord::Ns => "NS",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == text)
    }
}

/// The longest name DNS has.
pub const MAX_DNS_NAME_CHARS: usize = 253;
/// The most answers a `dns` check lists as expected.
pub const MAX_DNS_EXPECTED: usize = 20;

/// A host: a name or an IP address, without spaces or a path.
fn host_name<'a>(field: &str, value: &'a str) -> Result<&'a str, CheckError> {
    let host = required(field, value)?;
    if host.contains(char::is_whitespace) || host.contains('/') {
        return Err(CheckError(format!(
            "the {field} is a name or an IP address"
        )));
    }
    Ok(host)
}

/// The most mount points a `disks` check leaves out.
pub const MAX_IGNORED_DISKS: usize = 20;

/// The port a `tls` check looks at when none is typed (HTTPS).
pub const DEFAULT_TLS_PORT: u16 = 443;

/// The longest check name, in characters.
pub const MAX_NAME_CHARS: usize = 100;
/// The longest host, URL or path, in characters.
pub const MAX_TARGET_CHARS: usize = 2000;
/// The most checks one probe runs (keeps the hub's answer small).
pub const MAX_CHECKS_PER_PROBE: usize = 500;

/// Report intervals the hub accepts, in seconds.
pub const MIN_INTERVAL_SECS: u32 = 5;
pub const MAX_INTERVAL_SECS: u32 = 3600;
/// A probe's report interval until the hub says otherwise.
pub const DEFAULT_INTERVAL_SECS: u32 = 60;

impl CheckSpec {
    /// A plain `ping` check: [`DEFAULT_PING_COUNT`] echoes to `host`.
    pub fn ping(host: &str) -> Self {
        CheckSpec::Ping {
            host: host.to_string(),
            count: DEFAULT_PING_COUNT,
            tcp_port: None,
        }
    }

    /// A plain `http` check: a `GET` of `url` that must answer 2xx.
    pub fn http(url: &str) -> Self {
        CheckSpec::Http {
            url: url.to_string(),
            options: HttpOptions::default(),
        }
    }

    /// A plain `snmp` check: v2c on the usual port, asking for `oids`.
    pub fn snmp(host: &str, oids: Vec<SnmpOid>) -> Self {
        CheckSpec::Snmp {
            host: host.to_string(),
            port: DEFAULT_SNMP_PORT,
            version: SnmpVersion::V2c,
            user: String::new(),
            auth: None,
            privacy: None,
            oids,
        }
    }

    /// The kind, as in the JSON `kind` field (one of [`CHECK_KINDS`]).
    pub fn kind(&self) -> &'static str {
        match self {
            CheckSpec::Ping { .. } => "ping",
            CheckSpec::Http { .. } => "http",
            CheckSpec::Snmp { .. } => "snmp",
            CheckSpec::Tcp { .. } => "tcp",
            CheckSpec::Cpu => "cpu",
            CheckSpec::Memory => "memory",
            CheckSpec::Dns { .. } => "dns",
            CheckSpec::Disk { .. } => "disk",
            CheckSpec::Disks { .. } => "disks",
            CheckSpec::File { .. } => "file",
            CheckSpec::Service { .. } => "service",
            CheckSpec::Uptime => "uptime",
            CheckSpec::Tls { .. } => "tls",
        }
    }

    /// What it looks at, for people: `10.0.0.1`, `db:5432`, a URL, a disk.
    /// Checks of the probe's own machine say `localhost`.
    pub fn target(&self) -> String {
        match self {
            CheckSpec::Ping { host, .. } => host.clone(),
            CheckSpec::Http { url, .. } => url.clone(),
            CheckSpec::Snmp { host, port, .. } if *port == DEFAULT_SNMP_PORT => host.clone(),
            CheckSpec::Snmp { host, port, .. } => format!("{host}:{port}"),
            CheckSpec::Tcp { host, port } | CheckSpec::Tls { host, port, .. }
                if host.contains(':') =>
            {
                format!("[{host}]:{port}")
            }
            CheckSpec::Tcp { host, port } | CheckSpec::Tls { host, port, .. } => {
                format!("{host}:{port}")
            }
            CheckSpec::Dns {
                name,
                record,
                server,
                ..
            } => match server {
                Some(server) => format!("{name} ({} at {server})", record.label()),
                None => format!("{name} ({})", record.label()),
            },
            CheckSpec::Disk { path: Some(path) } => path.clone(),
            CheckSpec::Disk { path: None } => "system disk".to_string(),
            CheckSpec::Disks { ignore } if ignore.is_empty() => "all disks".to_string(),
            CheckSpec::Disks { ignore } => format!("all disks but {}", ignore.join(", ")),
            CheckSpec::File { path, pattern, .. } if pattern.is_empty() => path.clone(),
            CheckSpec::File { path, pattern, .. } => format!("{path} ({pattern})"),
            CheckSpec::Service { name } => name.clone(),
            CheckSpec::Cpu | CheckSpec::Memory | CheckSpec::Uptime => "localhost".to_string(),
        }
    }

    /// Whether the probe can run it. The one rule set for the hub and the probe.
    pub fn validate(&self) -> Result<(), CheckError> {
        match self {
            CheckSpec::Ping {
                host,
                count,
                tcp_port,
            } => {
                if !(1..=MAX_PING_COUNT).contains(count) {
                    return Err(CheckError(format!(
                        "a ping sends 1-{MAX_PING_COUNT} echoes"
                    )));
                }
                if *tcp_port == Some(0) {
                    return Err(CheckError::new("the TCP port must be 1-65535"));
                }
                host_name("host", host).map(|_| ())
            }
            CheckSpec::Dns {
                name,
                server,
                expect,
                ..
            } => {
                let name = host_name("name", name)?;
                if name.chars().count() > MAX_DNS_NAME_CHARS {
                    return Err(CheckError(format!(
                        "the name is longer than {MAX_DNS_NAME_CHARS} characters"
                    )));
                }
                if let Some(server) = server
                    && server.parse::<IpAddr>().is_err()
                {
                    return Err(CheckError::new(
                        "the DNS server is an IP address, e.g. 192.168.1.1",
                    ));
                }
                if expect.len() > MAX_DNS_EXPECTED {
                    return Err(CheckError(format!(
                        "more than {MAX_DNS_EXPECTED} expected answers"
                    )));
                }
                expect
                    .iter()
                    .try_for_each(|answer| required("expected answer", answer).map(|_| ()))
            }
            CheckSpec::Http { url, options } => {
                options.validate()?;
                let url = required("url", url)?;
                let rest = url
                    .strip_prefix("https://")
                    .or_else(|| url.strip_prefix("http://"))
                    .ok_or_else(|| {
                        CheckError::new("the URL must start with http:// or https://")
                    })?;
                if rest.is_empty() || rest.starts_with('/') || url.contains(char::is_whitespace) {
                    return Err(CheckError::new(
                        "the URL needs a host, e.g. https://example.com",
                    ));
                }
                Ok(())
            }
            CheckSpec::Tcp { host, port } | CheckSpec::Tls { host, port, .. } => {
                host_name("host", host)?;
                if *port == 0 {
                    return Err(CheckError::new("the port must be 1-65535"));
                }
                Ok(())
            }
            CheckSpec::Disk { path: Some(path) } => required("path", path).map(|_| ()),
            CheckSpec::Disk { path: None } | CheckSpec::Cpu | CheckSpec::Memory => Ok(()),
            CheckSpec::Disks { ignore } => {
                if ignore.len() > MAX_IGNORED_DISKS {
                    return Err(CheckError(format!(
                        "more than {MAX_IGNORED_DISKS} disks to leave out"
                    )));
                }
                ignore
                    .iter()
                    .try_for_each(|disk| required("disk to leave out", disk).map(|_| ()))
            }
            CheckSpec::File { path, pattern, .. } => {
                let path = required("path", path)?;
                // Whichever system the probe runs on: /…, C:\…, \\server\share.
                let drive = path.as_bytes().get(1) == Some(&b':')
                    && path.starts_with(|c: char| c.is_ascii_alphabetic())
                    && path[2..].starts_with(['\\', '/']);
                if !(path.starts_with('/') || path.starts_with("\\\\") || drive) {
                    return Err(CheckError::new(
                        "the path must be a full one, e.g. /var/backups, D:\\Backups or \\\\nas\\backups",
                    ));
                }
                if pattern.chars().count() > MAX_KEYWORD_CHARS
                    || pattern.contains(['/', '\\'])
                    || pattern.trim() != pattern
                {
                    return Err(CheckError::new(
                        "the pattern is for file names, e.g. *.bak: no folders, no spaces at its ends",
                    ));
                }
                Ok(())
            }
            CheckSpec::Service { name } => {
                let name = required("service name", name)?;
                if name.chars().count() > MAX_SERVICE_NAME_CHARS
                    || name.contains(['/', '\\'])
                    || name.chars().any(char::is_control)
                {
                    return Err(CheckError::new(
                        "the service name is a Windows service's name or a systemd unit, e.g. Spooler, nginx",
                    ));
                }
                Ok(())
            }
            CheckSpec::Snmp {
                host,
                port,
                version,
                user,
                auth,
                privacy,
                oids,
            } => {
                host_name("host", host)?;
                if *port == 0 {
                    return Err(CheckError::new("the port must be 1-65535"));
                }
                match version {
                    SnmpVersion::V2c if !user.is_empty() || auth.is_some() || privacy.is_some() => {
                        return Err(CheckError::new(
                            "a user, authentication and privacy are SNMP v3's: v2c has a community",
                        ));
                    }
                    SnmpVersion::V2c => {}
                    SnmpVersion::V3 => {
                        let user = required("user", user)?;
                        if user.chars().count() > MAX_SNMP_NAME_CHARS {
                            return Err(CheckError(format!(
                                "the user is longer than {MAX_SNMP_NAME_CHARS} characters"
                            )));
                        }
                        if privacy.is_some() && auth.is_none() {
                            return Err(CheckError::new(
                                "privacy needs authentication: choose how the user is authenticated",
                            ));
                        }
                    }
                }
                if oids.is_empty() || oids.len() > MAX_SNMP_OIDS {
                    return Err(CheckError(format!(
                        "an SNMP check asks for 1-{MAX_SNMP_OIDS} values"
                    )));
                }
                let mut labels = HashSet::new();
                for value in oids {
                    if value.arcs().is_none() {
                        return Err(CheckError(format!(
                            "\"{}\" isn't an OID in numbers, e.g. 1.3.6.1.2.1.1.3.0",
                            value.oid
                        )));
                    }
                    if value.name.trim() != value.name
                        || value.name.chars().count() > MAX_SNMP_NAME_CHARS
                        || value.name.chars().any(char::is_control)
                    {
                        return Err(CheckError(format!(
                            "a value's name takes at most {MAX_SNMP_NAME_CHARS} characters"
                        )));
                    }
                    if !labels.insert(value.label()) {
                        return Err(CheckError(format!("{} is asked for twice", value.label())));
                    }
                    if let Some(expect) = &value.expect
                        && (expect.trim().is_empty()
                            || expect.trim() != expect
                            || expect.chars().count() > MAX_KEYWORD_CHARS)
                    {
                        return Err(CheckError(format!(
                            "what {} must be is 1-{MAX_KEYWORD_CHARS} characters, without spaces around",
                            value.label()
                        )));
                    }
                }
                Ok(())
            }
            CheckSpec::Uptime => Ok(()),
        }
    }
}

/// `value`, which must be neither blank nor too long.
impl CheckSpec {
    /// The secrets this check needs, by name ([`secret`]): none for most.
    pub fn secret_names(&self) -> Vec<&'static str> {
        match self {
            CheckSpec::Snmp {
                version: SnmpVersion::V2c,
                ..
            } => vec![secret::COMMUNITY],
            CheckSpec::Snmp { auth, privacy, .. } => auth
                .map(|_| secret::AUTH_PASSWORD)
                .into_iter()
                .chain(privacy.map(|_| secret::PRIV_PASSWORD))
                .collect(),
            _ => vec![],
        }
    }

    /// Whether `secrets` are what this check needs: each one there, none
    /// besides. The hub and the probe both ask.
    pub fn validate_secrets(&self, secrets: &Secrets) -> Result<(), CheckError> {
        let needed = self.secret_names();
        for name in &needed {
            let what = match *name {
                secret::COMMUNITY => "community",
                secret::AUTH_PASSWORD => "authentication password",
                _ => "privacy password",
            };
            let value = secrets.get(name).unwrap_or_default();
            if value.is_empty() {
                return Err(CheckError(format!("the {what} is needed")));
            }
            if value.chars().count() > MAX_SECRET_CHARS || value.chars().any(char::is_control) {
                return Err(CheckError(format!(
                    "the {what} takes at most {MAX_SECRET_CHARS} characters"
                )));
            }
            if *name != secret::COMMUNITY && value.chars().count() < MIN_SNMP_PASSWORD_CHARS {
                return Err(CheckError(format!(
                    "the {what} has at least {MIN_SNMP_PASSWORD_CHARS} characters (SNMP v3's rule)"
                )));
            }
        }
        if let Some(extra) = secrets.0.keys().find(|k| !needed.contains(&k.as_str())) {
            return Err(CheckError(format!(
                "this check has no secret named {extra}"
            )));
        }
        Ok(())
    }
}

fn required<'a>(field: &str, value: &'a str) -> Result<&'a str, CheckError> {
    if value.trim().is_empty() {
        return Err(CheckError(format!("the {field} can't be empty")));
    }
    if value.trim() != value {
        return Err(CheckError(format!(
            "the {field} has spaces at the start or end"
        )));
    }
    if value.chars().count() > MAX_TARGET_CHARS {
        return Err(CheckError(format!(
            "the {field} is longer than {MAX_TARGET_CHARS} characters"
        )));
    }
    Ok(value)
}

/// Why a check or config can't be used. The text is for people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckError(pub String);

impl CheckError {
    fn new(message: &str) -> Self {
        Self(message.to_string())
    }
}

impl fmt::Display for CheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CheckError {}

/// A check's name: not blank, at most [`MAX_NAME_CHARS`].
pub fn validate_name(name: &str) -> Result<(), CheckError> {
    if name.trim().is_empty() {
        return Err(CheckError::new("the check name can't be empty"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(CheckError(format!(
            "the check name is longer than {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(())
}

/// A report interval within [`MIN_INTERVAL_SECS`]..=[`MAX_INTERVAL_SECS`].
pub fn validate_interval(secs: u32) -> Result<(), CheckError> {
    if !(MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(&secs) {
        return Err(CheckError(format!(
            "the interval must be {MIN_INTERVAL_SECS}-{MAX_INTERVAL_SECS} seconds"
        )));
    }
    Ok(())
}

/// One check as the probe runs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckDefinition {
    pub id: CheckId,
    pub name: String,
    pub spec: CheckSpec,
    /// What the check needs that isn't in its spec (D-86); empty for most.
    #[serde(default, skip_serializing_if = "Secrets::is_empty")]
    pub secrets: Secrets,
}

/// Everything a probe runs, as the hub sends it (only enabled checks).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeConfig {
    /// Identifies this exact content (the hub derives it from the rest). The
    /// probe echoes it in its reports, so the hub knows what it runs.
    pub revision: String,
    /// Seconds between two reports.
    pub interval_secs: u32,
    pub checks: Vec<CheckDefinition>,
}

impl ProbeConfig {
    /// Whether the probe can run all of it. A config is applied whole or not
    /// at all: the probe keeps the previous one if this fails.
    pub fn validate(&self) -> Result<(), CheckError> {
        if self.revision.is_empty() {
            return Err(CheckError::new("the config has no revision"));
        }
        validate_interval(self.interval_secs)?;
        if self.checks.len() > MAX_CHECKS_PER_PROBE {
            return Err(CheckError(format!(
                "more than {MAX_CHECKS_PER_PROBE} checks"
            )));
        }
        let mut ids = HashSet::new();
        for check in &self.checks {
            if !ids.insert(&check.id) {
                return Err(CheckError(format!("check {} appears twice", check.id)));
            }
            validate_name(&check.name)
                .and_then(|()| check.spec.validate())
                .and_then(|()| check.spec.validate_secrets(&check.secrets))
                .map_err(|e| CheckError(format!("check \"{}\": {e}", check.name)))?;
        }
        Ok(())
    }
}

/// A config the probe refused, reported until the hub sends another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigRejection {
    pub revision: String,
    pub error: String,
}

/// Did the check succeed? The probe reports only this; warning and down
/// thresholds on the metrics are the hub's business (proposal 0006 §1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Health {
    Up,
    Down,
}

impl Health {
    /// `"Up"` or `"Down"`, as serialised and stored.
    pub fn as_str(self) -> &'static str {
        match self {
            Health::Up => "Up",
            Health::Down => "Down",
        }
    }
}

/// One measured value, e.g. `{"key": "usage", "value": 42.5, "unit": "%"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    /// Stable within a kind; see [`metric`] for the keys each kind reports.
    pub key: String,
    pub value: f64,
    pub unit: String,
}

impl Metric {
    pub fn new(key: &str, value: f64, unit: &str) -> Self {
        Self {
            key: key.to_string(),
            value,
            unit: unit.to_string(),
        }
    }
}

/// Metric keys and units. The first metric of a result is its headline value.
pub mod metric {
    /// `cpu`, `memory`, `disk`: percent used (hub thresholds apply to it).
    pub const USAGE: &str = "usage";
    /// `memory`, `disk`: gigabytes used.
    pub const USED: &str = "used";
    /// `disk`: gigabytes free.
    pub const FREE: &str = "free";
    /// `memory`, `disk`: gigabytes in total.
    pub const TOTAL: &str = "total";
    /// `uptime`: seconds since the machine started.
    pub const UPTIME: &str = "uptime";
    /// `dns`: how many different answers came back.
    pub const ANSWERS: &str = "answers";
    /// `disks`: how many disks the check found.
    pub const VOLUMES: &str = "volumes";
    /// `ping`: percent of the echoes that weren't answered (hub
    /// thresholds apply to it, beside the latency).
    pub const LOSS: &str = "loss";
    /// `ping`: milliseconds the slowest answer took (the result's latency
    /// is the average).
    pub const LATENCY_MAX: &str = "latency_max";
    /// `ping`: milliseconds one answer's time differs from the one before,
    /// on average.
    pub const JITTER: &str = "jitter";
    /// `file`: hours since the newest file was written (hub thresholds
    /// apply to it).
    pub const AGE: &str = "age";
    /// `file`: megabytes of the newest file.
    pub const SIZE: &str = "size";
    /// `file`: how many files match.
    pub const FILES: &str = "files";
    /// `tls`: days until the certificate expires (hub thresholds apply to
    /// it: at or under a limit).
    pub const DAYS_LEFT: &str = "days_left";

    pub const PERCENT: &str = "%";
    pub const GIGABYTES: &str = "GB";
    pub const SECONDS: &str = "s";
    pub const DAYS: &str = "d";
    pub const HOURS: &str = "h";
    pub const MILLISECONDS: &str = "ms";
    pub const MEGABYTES: &str = "MB";
}

/// One check's outcome in a report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub check_id: CheckId,
    pub status: Health,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<Metric>,
}

impl CheckResult {
    /// The metric with this key, if the result has it.
    pub fn metric(&self, key: &str) -> Option<&Metric> {
        self.metrics.iter().find(|m| m.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(id: &str, spec: CheckSpec) -> CheckDefinition {
        CheckDefinition {
            id: CheckId(id.into()),
            name: format!("check {id}"),
            spec,
            secrets: Secrets::default(),
        }
    }

    fn config(checks: Vec<CheckDefinition>) -> ProbeConfig {
        ProbeConfig {
            revision: "r1".into(),
            interval_secs: 60,
            checks,
        }
    }

    #[test]
    fn specs_serialise_with_their_kind() {
        let tcp = CheckSpec::Tcp {
            host: "db".into(),
            port: 5432,
        };
        assert_eq!(
            serde_json::to_value(&tcp).unwrap(),
            serde_json::json!({"kind": "tcp", "host": "db", "port": 5432})
        );
        let disk: CheckSpec = serde_json::from_str(r#"{"kind":"disk"}"#).unwrap();
        assert_eq!(disk, CheckSpec::Disk { path: None });
        assert_eq!(
            serde_json::to_string(&CheckSpec::Cpu).unwrap(),
            r#"{"kind":"cpu"}"#
        );
    }

    #[test]
    fn a_ping_sends_four_echoes_unless_it_says_otherwise() {
        let plain = r#"{"kind":"ping","host":"10.0.0.1"}"#;
        assert_eq!(
            serde_json::to_string(&CheckSpec::ping("10.0.0.1")).unwrap(),
            plain
        );
        assert_eq!(
            serde_json::from_str::<CheckSpec>(plain).unwrap(),
            CheckSpec::ping("10.0.0.1")
        );
        let ping = |count, tcp_port| CheckSpec::Ping {
            host: "10.0.0.1".into(),
            count,
            tcp_port,
        };
        assert_eq!(
            serde_json::to_value(ping(10, Some(443))).unwrap(),
            serde_json::json!({ "kind": "ping", "host": "10.0.0.1", "count": 10, "tcp_port": 443 })
        );
        assert_eq!(ping(1, None).validate(), Ok(()));
        assert_eq!(ping(MAX_PING_COUNT, Some(22)).validate(), Ok(()));
        assert!(ping(0, None).validate().unwrap_err().0.contains("echoes"));
        assert!(ping(MAX_PING_COUNT + 1, None).validate().is_err());
        assert!(
            ping(4, Some(0))
                .validate()
                .unwrap_err()
                .0
                .contains("TCP port")
        );
    }

    fn dns(name: &str, server: Option<&str>, expect: &[&str]) -> CheckSpec {
        CheckSpec::Dns {
            name: name.into(),
            record: DnsRecord::A,
            server: server.map(str::to_string),
            expect: expect.iter().map(|e| e.to_string()).collect(),
        }
    }

    #[test]
    fn a_dns_check_asks_for_a_record_of_a_name() {
        let plain = r#"{"kind":"dns","name":"example.com"}"#;
        assert_eq!(
            serde_json::to_string(&dns("example.com", None, &[])).unwrap(),
            plain
        );
        assert_eq!(
            serde_json::from_str::<CheckSpec>(plain).unwrap(),
            dns("example.com", None, &[])
        );
        let mx = CheckSpec::Dns {
            name: "example.com".into(),
            record: DnsRecord::Mx,
            server: Some("192.168.1.1".into()),
            expect: vec!["10 mail.example.com".into()],
        };
        assert_eq!(
            serde_json::to_value(&mx).unwrap(),
            serde_json::json!({
                "kind": "dns", "name": "example.com", "record": "mx",
                "server": "192.168.1.1", "expect": ["10 mail.example.com"],
            })
        );
        assert_eq!(mx.validate(), Ok(()));
        assert_eq!(mx.target(), "example.com (MX at 192.168.1.1)");
        assert_eq!(dns("example.com", None, &[]).target(), "example.com (A)");
        for record in DnsRecord::ALL {
            assert_eq!(DnsRecord::parse(record.as_str()), Some(record));
        }

        for (spec, why) in [
            (dns("", None, &[]), "can't be empty"),
            (
                dns("https://example.com", None, &[]),
                "a name or an IP address",
            ),
            (
                dns("example.com", Some("dns.example.com"), &[]),
                "DNS server",
            ),
            (dns("example.com", None, &[""]), "can't be empty"),
            (dns(&"a".repeat(254), None, &[]), "longer than"),
        ] {
            let error = spec.validate().expect_err(&format!("{spec:?}"));
            assert!(error.0.contains(why), "{spec:?}: {error}");
        }
    }

    #[test]
    fn a_service_check_names_a_service_of_either_system() {
        let service = |name: &str| CheckSpec::Service { name: name.into() };
        assert_eq!(
            serde_json::to_string(&service("Spooler")).unwrap(),
            r#"{"kind":"service","name":"Spooler"}"#
        );
        for name in [
            "Spooler",
            "MSSQL$SQLEXPRESS",
            "nginx",
            "postgresql@16-main.service",
            "My Agent",
        ] {
            assert_eq!(service(name).validate(), Ok(()), "{name}");
            assert_eq!(service(name).target(), name);
        }
        for name in ["", " nginx", "system/nginx", "C:\\agent.exe"] {
            assert!(service(name).validate().is_err(), "{name}");
        }
    }

    #[test]
    fn a_ping_takes_an_address_or_a_name() {
        for host in ["192.168.1.1", "2001:db8::1", "router.lan", "printer-2"] {
            assert_eq!(CheckSpec::ping(host).validate(), Ok(()), "{host}");
        }
        assert!(CheckSpec::ping("https://router.lan/").validate().is_err());
    }

    #[test]
    fn a_disks_check_is_every_disk_but_those_it_leaves_out() {
        let all = CheckSpec::Disks { ignore: vec![] };
        assert_eq!(serde_json::to_string(&all).unwrap(), r#"{"kind":"disks"}"#);
        assert_eq!(
            serde_json::from_str::<CheckSpec>(r#"{"kind":"disks"}"#).unwrap(),
            all
        );
        assert_eq!(all.target(), "all disks");
        let some = CheckSpec::Disks {
            ignore: vec!["E:".into(), "/mnt/*".into()],
        };
        assert_eq!(some.target(), "all disks but E:, /mnt/*");
        assert_eq!(some.validate(), Ok(()));
        let blank = CheckSpec::Disks {
            ignore: vec!["".into()],
        };
        assert!(blank.validate().unwrap_err().0.contains("can't be empty"));
        let many = CheckSpec::Disks {
            ignore: vec!["X:".into(); MAX_IGNORED_DISKS + 1],
        };
        assert!(many.validate().is_err());
    }

    fn file(path: &str, pattern: &str) -> CheckSpec {
        CheckSpec::File {
            path: path.into(),
            pattern: pattern.into(),
            recursive: false,
            min_bytes: None,
        }
    }

    #[test]
    fn a_file_check_takes_a_full_path_of_any_system_and_a_name_pattern() {
        let json = r#"{"kind":"file","path":"D:\\Backups","pattern":"*.bak"}"#;
        let spec = file("D:\\Backups", "*.bak");
        assert_eq!(serde_json::to_string(&spec).unwrap(), json);
        assert_eq!(serde_json::from_str::<CheckSpec>(json).unwrap(), spec);
        assert_eq!(spec.target(), "D:\\Backups (*.bak)");
        assert_eq!(file("/srv/dump.sql", "").target(), "/srv/dump.sql");
        let full = CheckSpec::File {
            path: "/var/backups".into(),
            pattern: String::new(),
            recursive: true,
            min_bytes: Some(1_048_576),
        };
        assert_eq!(
            serde_json::to_value(&full).unwrap(),
            serde_json::json!({ "kind": "file", "path": "/var/backups", "recursive": true, "min_bytes": 1048576 })
        );

        for path in [
            "/var/backups",
            "D:\\Backups",
            "c:/backups",
            "\\\\nas\\backups\\sql",
        ] {
            assert_eq!(file(path, "").validate(), Ok(()), "{path}");
        }
        for (path, pattern, why) in [
            ("", "", "can't be empty"),
            ("backups", "", "full one"),
            ("D:Backups", "", "full one"),
            ("./backups", "*.bak", "full one"),
            ("/var/backups", "sql/*.bak", "file names"),
            ("/var/backups", " *.bak", "file names"),
        ] {
            let error = file(path, pattern).validate().expect_err(path);
            assert!(error.0.contains(why), "{path} {pattern}: {error}");
        }
    }

    fn http_with(change: impl FnOnce(&mut HttpOptions)) -> CheckSpec {
        let mut options = HttpOptions::default();
        change(&mut options);
        CheckSpec::Http {
            url: "https://example.com/health".into(),
            options,
        }
    }

    #[test]
    fn a_plain_http_check_is_only_its_url_and_options_sit_beside_it() {
        let plain = r#"{"kind":"http","url":"https://example.com/health"}"#;
        assert_eq!(serde_json::to_string(&http_with(|_| {})).unwrap(), plain);
        assert_eq!(
            serde_json::from_str::<CheckSpec>(plain).unwrap(),
            http_with(|_| {})
        );

        let full = http_with(|o| {
            o.method = HttpMethod::Head;
            o.status = "200, 301-308".into();
            o.follow_redirects = false;
            o.verify = false;
            o.headers = vec![HttpHeader {
                name: "Accept".into(),
                value: "text/html".into(),
            }];
        });
        let json = serde_json::to_value(&full).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "http", "url": "https://example.com/health", "method": "head",
                "status": "200, 301-308", "follow_redirects": false, "verify": false,
                "headers": [{ "name": "Accept", "value": "text/html" }],
            })
        );
        assert_eq!(serde_json::from_value::<CheckSpec>(json).unwrap(), full);
        assert_eq!(full.validate(), Ok(()));
    }

    #[test]
    fn expected_status_codes_are_codes_and_ranges() {
        assert_eq!(status_ranges("").unwrap(), vec![200..=299]);
        assert_eq!(
            status_ranges(" 200 , 301-308,401").unwrap(),
            vec![200..=200, 301..=308, 401..=401]
        );
        for (text, why) in [
            ("ok", "isn't a status code"),
            ("99", "isn't a status code"),
            ("200-", "isn't a status code"),
            ("200,,204", "isn't a status code"),
            ("299-200", "backwards"),
        ] {
            let error = status_ranges(text).expect_err(text);
            assert!(error.0.contains(why), "{text}: {error}");
        }
        let any = HttpOptions::default();
        assert!(any.expects(204) && !any.expects(301) && !any.expects(500));
        let login = HttpOptions {
            status: "401, 403".into(),
            ..HttpOptions::default()
        };
        assert!(login.expects(401) && !login.expects(200));
        assert_eq!(login.expected_status(), "401, 403");
        assert_eq!(any.expected_status(), "200-299");
    }

    #[test]
    fn http_options_are_checked() {
        let header = |name: &str, value: &str| HttpHeader {
            name: name.into(),
            value: value.into(),
        };
        let cases: [(CheckSpec, &str); 6] = [
            (http_with(|o| o.status = "2xx".into()), "status code"),
            (
                http_with(|o| o.contains = "x".repeat(MAX_KEYWORD_CHARS + 1)),
                "longer than",
            ),
            (
                http_with(|o| {
                    o.method = HttpMethod::Head;
                    o.absent = "error".into();
                }),
                "HEAD",
            ),
            (
                http_with(|o| o.headers = vec![header("X Y", "1")]),
                "header name",
            ),
            (
                http_with(|o| o.headers = vec![header("X-Y", "a\r\nb")]),
                "control characters",
            ),
            (
                http_with(|o| o.headers = vec![header("A", "1"); MAX_HTTP_HEADERS + 1]),
                "headers",
            ),
        ];
        for (spec, expected) in cases {
            let error = spec.validate().expect_err(&format!("{spec:?}"));
            assert!(error.0.contains(expected), "{spec:?}: {error}");
        }
        let fine = http_with(|o| {
            o.contains = "Welcome".into();
            o.absent = "Maintenance".into();
            o.headers = vec![header("Accept-Language", "en")];
        });
        assert_eq!(fine.validate(), Ok(()));
    }

    #[test]
    fn a_tls_check_verifies_the_certificate_unless_told_not_to() {
        let tls = |verify| CheckSpec::Tls {
            host: "example.com".into(),
            port: 443,
            verify,
        };
        let json = r#"{"kind":"tls","host":"example.com","port":443}"#;
        assert_eq!(serde_json::to_string(&tls(true)).unwrap(), json);
        assert_eq!(serde_json::from_str::<CheckSpec>(json).unwrap(), tls(true));
        let unverified = serde_json::to_value(tls(false)).unwrap();
        assert_eq!(unverified["verify"], false);
        assert_eq!(
            serde_json::from_value::<CheckSpec>(unverified).unwrap(),
            tls(false)
        );
        assert_eq!(tls(true).target(), "example.com:443");
    }

    #[test]
    fn every_kind_is_listed_and_named_as_serialised() {
        let all = [
            CheckSpec::ping("10.0.0.1"),
            CheckSpec::http("https://example.com"),
            CheckSpec::Tcp {
                host: "db".into(),
                port: 1,
            },
            CheckSpec::Tls {
                host: "example.com".into(),
                port: DEFAULT_TLS_PORT,
                verify: true,
            },
            dns("example.com", None, &[]),
            CheckSpec::snmp("10.0.0.2", vec![SnmpOid::of("1.3.6.1.2.1.1.3.0")]),
            CheckSpec::Cpu,
            CheckSpec::Memory,
            CheckSpec::Disk { path: None },
            CheckSpec::Disks { ignore: vec![] },
            file("/var/backups", "*.bak"),
            CheckSpec::Service {
                name: "nginx".into(),
            },
            CheckSpec::Uptime,
        ];
        assert_eq!(all.len(), CHECK_KINDS.len());
        for (spec, kind) in all.iter().zip(CHECK_KINDS) {
            assert_eq!(spec.kind(), kind);
            let json = serde_json::to_value(spec).unwrap();
            assert_eq!(json["kind"], kind);
            assert_eq!(spec.validate(), Ok(()), "{kind}");
        }
    }

    #[test]
    fn targets_read_well() {
        let tcp = |host: &str| CheckSpec::Tcp {
            host: host.into(),
            port: 443,
        };
        assert_eq!(tcp("db").target(), "db:443");
        assert_eq!(tcp("::1").target(), "[::1]:443");
        assert_eq!(CheckSpec::Memory.target(), "localhost");
        assert_eq!(CheckSpec::Disk { path: None }.target(), "system disk");
    }

    #[test]
    fn invalid_specs_say_why() {
        let cases = [
            (CheckSpec::ping(""), "can't be empty"),
            (CheckSpec::ping("router lan"), "a name or an IP address"),
            (CheckSpec::http("example.com"), "http://"),
            (CheckSpec::http("https://"), "needs a host"),
            (
                CheckSpec::Tcp {
                    host: "db".into(),
                    port: 0,
                },
                "port",
            ),
            (
                CheckSpec::Tcp {
                    host: " db".into(),
                    port: 5,
                },
                "spaces",
            ),
            (
                CheckSpec::Tls {
                    host: "https://example.com".into(),
                    port: 443,
                    verify: true,
                },
                "a name or an IP address",
            ),
            (
                CheckSpec::Disk {
                    path: Some("".into()),
                },
                "can't be empty",
            ),
        ];
        for (spec, expected) in cases {
            let error = spec.validate().expect_err(&format!("{spec:?}"));
            assert!(error.0.contains(expected), "{spec:?}: {error}");
        }
    }

    #[test]
    fn a_config_is_valid_only_as_a_whole() {
        assert_eq!(config(vec![check("a", CheckSpec::Cpu)]).validate(), Ok(()));

        let twice = config(vec![
            check("a", CheckSpec::Cpu),
            check("a", CheckSpec::Memory),
        ]);
        assert!(twice.validate().unwrap_err().0.contains("twice"));

        let bad = config(vec![
            check("a", CheckSpec::Cpu),
            check("b", CheckSpec::ping("x y")),
        ]);
        assert!(bad.validate().unwrap_err().0.contains("check \"check b\""));

        let mut slow = config(vec![]);
        slow.interval_secs = MAX_INTERVAL_SECS + 1;
        assert!(slow.validate().is_err());

        let mut unnamed = config(vec![check("a", CheckSpec::Cpu)]);
        unnamed.checks[0].name = " ".into();
        assert!(unnamed.validate().is_err());
    }

    #[test]
    fn a_config_of_an_unknown_kind_does_not_parse() {
        // A newer hub's new kind: the probe refuses the whole config and keeps its own.
        let json = r#"{"revision":"r","interval_secs":60,"checks":[{"id":"a","name":"x","spec":{"kind":"snmp"}}]}"#;
        assert!(serde_json::from_str::<ProbeConfig>(json).is_err());
    }

    #[test]
    fn results_leave_out_what_they_dont_have() {
        let result = CheckResult {
            check_id: CheckId("c1".into()),
            status: Health::Up,
            message: "ok".into(),
            latency_ms: None,
            metrics: vec![],
        };
        assert_eq!(
            serde_json::to_string(&result).unwrap(),
            r#"{"check_id":"c1","status":"Up","message":"ok"}"#
        );
        let with_metric = CheckResult {
            metrics: vec![Metric::new(metric::USAGE, 12.5, metric::PERCENT)],
            ..result
        };
        assert_eq!(with_metric.metric(metric::USAGE).unwrap().value, 12.5);
        assert!(with_metric.metric(metric::FREE).is_none());
    }
}

#[cfg(test)]
mod snmp_tests {
    use super::*;

    fn value(oid: &str, name: &str, expect: Option<&str>) -> SnmpOid {
        SnmpOid {
            oid: oid.into(),
            name: name.into(),
            expect: expect.map(str::to_string),
        }
    }

    fn v2c(oids: Vec<SnmpOid>) -> CheckSpec {
        CheckSpec::snmp("192.168.1.5", oids)
    }

    fn v3(auth: Option<SnmpAuth>, privacy: Option<SnmpPrivacy>) -> CheckSpec {
        CheckSpec::Snmp {
            host: "switch.lan".into(),
            port: 1161,
            version: SnmpVersion::V3,
            user: "monitor".into(),
            auth,
            privacy,
            oids: vec![value("1.3.6.1.2.1.1.3.0", "", None)],
        }
    }

    /// `spec` with another user and other values.
    fn with(mut spec: CheckSpec, new_user: &str, new_oids: Option<Vec<SnmpOid>>) -> CheckSpec {
        if let CheckSpec::Snmp { user, oids, .. } = &mut spec {
            *user = new_user.into();
            if let Some(new_oids) = new_oids {
                *oids = new_oids;
            }
        }
        spec
    }

    fn secrets(pairs: &[(&str, &str)]) -> Secrets {
        Secrets(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    #[test]
    fn an_snmp_checks_json_leaves_out_what_is_the_default() {
        let plain = v2c(vec![value("1.3.6.1.2.1.1.3.0", "", None)]);
        let json = serde_json::to_string(&plain).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"snmp","host":"192.168.1.5","oids":[{"oid":"1.3.6.1.2.1.1.3.0"}]}"#
        );
        assert_eq!(serde_json::from_str::<CheckSpec>(&json).unwrap(), plain);
        assert_eq!(plain.kind(), "snmp");
        assert_eq!(plain.target(), "192.168.1.5");

        let full = with(
            v3(Some(SnmpAuth::Sha256), Some(SnmpPrivacy::Aes128)),
            "monitor",
            Some(vec![value("1.3.6.1.2.1.2.2.1.8.1", "Uplink", Some("1"))]),
        );
        let json = serde_json::to_string(&full).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"snmp","host":"switch.lan","port":1161,"version":"v3","user":"monitor","auth":"sha256","privacy":"aes128","oids":[{"oid":"1.3.6.1.2.1.2.2.1.8.1","name":"Uplink","expect":"1"}]}"#
        );
        assert_eq!(serde_json::from_str::<CheckSpec>(&json).unwrap(), full);
        assert_eq!(full.target(), "switch.lan:1161");
        assert_eq!(full.validate(), Ok(()));
    }

    #[test]
    fn an_oid_is_numbers_with_dots() {
        let arcs = |oid: &str| value(oid, "", None).arcs();
        assert_eq!(
            arcs("1.3.6.1.2.1.1.3.0"),
            Some(vec![1, 3, 6, 1, 2, 1, 1, 3, 0])
        );
        assert_eq!(
            arcs(".1.3.6.1"),
            Some(vec![1, 3, 6, 1]),
            "as net-snmp prints them"
        );
        assert_eq!(arcs("2.999.1"), Some(vec![2, 999, 1]));
        for wrong in [
            "",
            "1",
            "1.",
            "1..3",
            "sysUpTime.0",
            "1.3.6.x",
            "3.1",
            "1.40",
            "1.3.-6",
            "1.3 .6",
            "1.3.99999999999",
        ] {
            assert_eq!(arcs(wrong), None, "{wrong:?}");
            assert!(
                v2c(vec![value(wrong, "", None)]).validate().is_err(),
                "{wrong:?}"
            );
        }
    }

    #[test]
    fn an_snmp_check_is_refused_with_the_reason() {
        let error = |spec: CheckSpec| spec.validate().unwrap_err().to_string();
        assert_eq!(error(v2c(vec![])), "an SNMP check asks for 1-20 values");
        let twice = vec![value("1.3.6.1.2.1.1.3.0", "", None); 2];
        assert_eq!(error(v2c(twice)), "1.3.6.1.2.1.1.3.0 is asked for twice");
        let blank = vec![value("1.3.6.1.2.1.1.3.0", "Up", Some(" "))];
        assert!(error(v2c(blank)).starts_with("what Up must be"));

        let with_user = with(
            v2c(vec![value("1.3.6.1.2.1.1.3.0", "", None)]),
            "monitor",
            None,
        );
        assert!(error(with_user).contains("v2c has a community"));
        let no_user = with(v3(None, None), "", None);
        assert_eq!(error(no_user), "the user can't be empty");
        assert!(
            error(v3(None, Some(SnmpPrivacy::Aes128))).starts_with("privacy needs authentication")
        );
        assert_eq!(v3(None, None).validate(), Ok(()));
    }

    #[test]
    fn a_check_says_which_secrets_it_needs_and_takes_no_others() {
        let plain = v2c(vec![value("1.3.6.1.2.1.1.3.0", "", None)]);
        assert_eq!(plain.secret_names(), [secret::COMMUNITY]);
        assert_eq!(
            plain
                .validate_secrets(&Secrets::default())
                .unwrap_err()
                .to_string(),
            "the community is needed"
        );
        assert_eq!(
            plain.validate_secrets(&secrets(&[("community", "public")])),
            Ok(())
        );
        assert!(
            plain
                .validate_secrets(&secrets(&[("community", "public"), ("auth_password", "x")]))
                .is_err()
        );

        assert!(v3(None, None).secret_names().is_empty());
        let private = v3(Some(SnmpAuth::Sha1), Some(SnmpPrivacy::Aes128));
        assert_eq!(
            private.secret_names(),
            [secret::AUTH_PASSWORD, secret::PRIV_PASSWORD]
        );
        let short = secrets(&[("auth_password", "short"), ("priv_password", "long enough")]);
        assert!(
            private
                .validate_secrets(&short)
                .unwrap_err()
                .to_string()
                .contains("at least 8 characters")
        );
        let good = secrets(&[
            ("auth_password", "long enough"),
            ("priv_password", "long enough"),
        ]);
        assert_eq!(private.validate_secrets(&good), Ok(()));

        // Other kinds have none.
        assert_eq!(CheckSpec::Cpu.validate_secrets(&Secrets::default()), Ok(()));
        assert!(
            CheckSpec::Cpu
                .validate_secrets(&secrets(&[("community", "x")]))
                .is_err()
        );
    }

    #[test]
    fn secrets_are_never_printed_and_travel_with_the_checks_definition() {
        let kept = secrets(&[("community", "s3cret")]);
        assert_eq!(format!("{kept:?}"), r#"{"community"}"#);
        let definition = CheckDefinition {
            id: CheckId("c1".into()),
            name: "Switch".into(),
            spec: v2c(vec![value("1.3.6.1.2.1.1.3.0", "", None)]),
            secrets: kept,
        };
        assert!(!format!("{definition:?}").contains("s3cret"));
        let json = serde_json::to_string(&definition).unwrap();
        assert!(
            json.ends_with(r#""secrets":{"community":"s3cret"}}"#),
            "{json}"
        );
        assert_eq!(
            serde_json::from_str::<CheckDefinition>(&json).unwrap(),
            definition
        );

        // A probe refuses a config whose check lacks its secret.
        let mut config = ProbeConfig {
            revision: "r1".into(),
            interval_secs: 60,
            checks: vec![definition],
        };
        assert_eq!(config.validate(), Ok(()));
        config.checks[0].secrets = Secrets::default();
        assert_eq!(
            config.validate().unwrap_err().to_string(),
            "check \"Switch\": the community is needed"
        );
        // A check without secrets is written as before.
        let plain = CheckDefinition {
            id: CheckId("c2".into()),
            name: "CPU".into(),
            spec: CheckSpec::Cpu,
            secrets: Secrets::default(),
        };
        assert_eq!(
            serde_json::to_string(&plain).unwrap(),
            r#"{"id":"c2","name":"CPU","spec":{"kind":"cpu"}}"#
        );
    }
}
