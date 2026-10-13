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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// A database server: does it accept connections? The probe speaks the
    /// start of the server's own protocol and logs in nowhere (M5-10, D-88).
    Database {
        engine: DbEngine,
        host: String,
        /// `None`: the engine's usual port ([`DbEngine::default_port`]).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
    },
    /// A mail server, greeted in its own protocol and encrypted as it
    /// should be; the probe never logs in.
    Mail {
        protocol: MailProtocol,
        host: String,
        /// `None`: the protocol's usual port ([`MailProtocol::default_port`]).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        /// How the connection is encrypted.
        #[serde(default, skip_serializing_if = "is_default")]
        security: MailSecurity,
        /// `false`: only the certificate's dates count, not who issued it
        /// or for which name.
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        verify: bool,
    },
    /// A directory server (Active Directory, OpenLDAP), asked in LDAP for
    /// what it tells anyone of itself; the probe never logs in.
    Ldap {
        host: String,
        /// `None`: the usual port ([`LdapSecurity::default_port`]).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        /// How the connection is encrypted.
        #[serde(default, skip_serializing_if = "is_default")]
        security: LdapSecurity,
        /// `false`: only the certificate's dates count, not who issued it
        /// or for which name.
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        verify: bool,
    },
    /// A time server asked the time (SNTP): how far the clock of the
    /// probe's machine is from it.
    Ntp {
        host: String,
        /// `None`: [`DEFAULT_NTP_PORT`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
    },
    /// A UPS, by what the server that watches it says (Network UPS
    /// Tools): Down when its battery is low; on battery and other signs
    /// are the hub's Warning.
    Nut {
        host: String,
        /// `None`: [`DEFAULT_NUT_PORT`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        /// The UPS's name on that server. Empty: the only one it has.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        ups: String,
    },
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
    /// The health of the probe machine's drives, as the drives say it
    /// themselves (SMART) and the machine's system reads it: Down when one
    /// says it is failing; warning signs are the hub's Warning.
    Smart {
        /// Drives to leave out, by model or serial number: `WDC WD40*`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ignore: Vec<String>,
    },
    /// How hot the probe's machine is, by its own sensors (CPU, board,
    /// drives): the hottest one's temperature counts.
    Temperature {
        /// The sensors that count, by name: `coretemp*`. None: every one.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        sensors: Vec<String>,
    },
    /// The virtual machines of the probe's machine, a Hyper-V host, as
    /// Hyper-V itself lists them: Down when one that must run doesn't, or
    /// is in a critical state; other trouble is the hub's Warning.
    Hyperv {
        /// The machines that must run, by name: `web01`, `sql-*`. None:
        /// every one is counted, none must run.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        vms: Vec<String>,
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
    /// The processes of the probe's machine by a name: how many run, and
    /// the CPU and memory they use together. Down when their number is
    /// outside `min`..`max`.
    Process {
        /// The program's name: `sqlservr`, `nginx`, `backup-*` (`*` any
        /// characters, `?` any one; `.exe` or not, whatever the case).
        name: String,
        /// Text the command line must contain, to tell apart programs of
        /// one name (`java`, `python3`): `app.jar`. Empty: any.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        command: String,
        /// The fewest that must run (`0`: none need to).
        #[serde(
            default = "default_process_min",
            skip_serializing_if = "is_default_process_min"
        )]
        min: u32,
        /// The most that may run. `None`: any number.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<u32>,
    },
    /// A network interface of the probe's machine, by its name (`eth0`,
    /// `Ethernet`): is its link up, and what goes through it.
    Interface { name: String },
    /// The events of a Windows event log that a source, event IDs and a
    /// level pick out, in the last `minutes`: how many, and the latest.
    /// The hub judges: events are trouble, or (`wanted`) must be there.
    Eventlog {
        /// The log: `System`, `Application`, `Security`, or a channel such
        /// as `Microsoft-Windows-Backup`.
        log: String,
        /// The source (provider) that wrote them: `disk`, `Ntfs`. Empty: any.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        source: String,
        /// The event IDs that count. Empty: any.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ids: Vec<u32>,
        /// Events of this level and worse.
        #[serde(default, skip_serializing_if = "is_default")]
        level: EventLevel,
        /// How far back to look.
        #[serde(
            default = "default_event_minutes",
            skip_serializing_if = "is_default_event_minutes"
        )]
        minutes: u32,
        /// How many events it takes: that many are trouble, or (`wanted`)
        /// that many must be there.
        #[serde(
            default = "default_event_count",
            skip_serializing_if = "is_default_event_count"
        )]
        count: u32,
        /// The events must be there (a backup's "succeeded"): too few are
        /// the trouble.
        #[serde(default, skip_serializing_if = "is_default")]
        wanted: bool,
        /// Trouble is a Warning, not Down.
        #[serde(default, skip_serializing_if = "is_default")]
        warning: bool,
    },
    /// How long the probe's machine has been running.
    Uptime,
    /// The updates the probe's machine waits for, as its system knows them:
    /// how many, how many of them security updates, whether it must restart
    /// and when the system last looked. Reported, never installed (D-59).
    /// The hub turns these into a Warning by the three limits here, in days;
    /// `null` turns a limit off.
    Updates {
        /// Warning once security updates have waited longer than this
        /// (`0`: as soon as one waits).
        #[serde(default = "default_update_days")]
        security_days: Option<u32>,
        /// Warning once a restart has been needed for longer than this.
        #[serde(default = "default_update_days")]
        reboot_days: Option<u32>,
        /// Warning once the system hasn't looked for updates for longer
        /// than this.
        #[serde(default = "default_stale_days")]
        stale_days: Option<u32>,
    },
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
/// The most rows of a table one walked value reports.
pub const MAX_SNMP_ROWS: usize = 64;
/// The most numbers one `snmp` check reports (the hub keeps each one's
/// history): the rest are in the message only.
pub const MAX_SNMP_METRICS: usize = 100;
/// The most values a value may have to be (or not be), and meanings it has.
pub const MAX_SNMP_WORDS: usize = 30;
/// The longest unit of a value, and meaning of one of its values.
pub const MAX_SNMP_UNIT_CHARS: usize = 12;
pub const MAX_SNMP_MEANING_CHARS: usize = 40;
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

/// `updates`: the days security updates, or a restart, may wait before the
/// hub warns, unless the check says otherwise: none.
pub const DEFAULT_UPDATE_DAYS: u32 = 0;
/// `updates`: the days without the system looking for updates before the
/// hub warns, unless the check says otherwise.
pub const DEFAULT_STALE_DAYS: u32 = 14;
/// `updates`: the longest any of its limits may be, in days.
pub const MAX_UPDATE_DAYS: u32 = 365;

/// How bad an event of a Windows event log is: an `eventlog` check counts
/// the events of its level and worse.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventLevel {
    Critical,
    #[default]
    Error,
    Warning,
    /// Every event, information included.
    Information,
}

/// `eventlog`: how far back a check looks unless it says otherwise, in
/// minutes; the furthest (31 days); and the most event IDs it may name.
pub const DEFAULT_EVENT_MINUTES: u32 = 60;
pub const MAX_EVENT_MINUTES: u32 = 31 * 24 * 60;
pub const MAX_EVENT_IDS: usize = 20;
/// `eventlog`: how many events it takes unless the check says otherwise,
/// and the most it may ask for (the probe counts no further).
pub const DEFAULT_EVENT_COUNT: u32 = 1;
pub const MAX_EVENT_COUNT: u32 = 1000;
/// `eventlog`: the longest name of a log or a source, in characters.
pub const MAX_EVENT_NAME_CHARS: usize = 256;

fn default_event_minutes() -> u32 {
    DEFAULT_EVENT_MINUTES
}

fn is_default_event_minutes(minutes: &u32) -> bool {
    *minutes == DEFAULT_EVENT_MINUTES
}

fn default_event_count() -> u32 {
    DEFAULT_EVENT_COUNT
}

fn is_default_event_count(count: &u32) -> bool {
    *count == DEFAULT_EVENT_COUNT
}

/// A name of a log or a source: what Windows' names are made of, and
/// nothing that would end the text it is put in when the log is asked.
fn event_name(name: &str) -> bool {
    name.chars().count() <= MAX_EVENT_NAME_CHARS
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || " -_./()$".contains(c))
}

/// `interface`: the longest name of an interface, in characters (Windows
/// allows 256).
pub const MAX_INTERFACE_NAME_CHARS: usize = 256;

/// `process`: the fewest that must run, unless the check says otherwise.
pub const DEFAULT_PROCESS_MIN: u32 = 1;
/// `process`: the largest number a check may ask for.
pub const MAX_PROCESS_COUNT: u32 = 10_000;
/// `process`: the longest name, and text of a command line, in characters.
pub const MAX_PROCESS_NAME_CHARS: usize = 256;

fn default_process_min() -> u32 {
    DEFAULT_PROCESS_MIN
}

fn is_default_process_min(min: &u32) -> bool {
    *min == DEFAULT_PROCESS_MIN
}

fn default_update_days() -> Option<u32> {
    Some(DEFAULT_UPDATE_DAYS)
}

fn default_stale_days() -> Option<u32> {
    Some(DEFAULT_STALE_DAYS)
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

/// A limit on a value an `snmp` check reads (M5-09): past it, the hub
/// makes the check Warning or Down.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnmpLimit {
    /// Reached when the value is over this (a temperature).
    Over(f64),
    /// Reached when the value is under this (toner left).
    Under(f64),
}

impl SnmpLimit {
    pub fn at(self) -> f64 {
        match self {
            SnmpLimit::Over(at) | SnmpLimit::Under(at) => at,
        }
    }

    pub fn is_under(self) -> bool {
        matches!(self, SnmpLimit::Under(_))
    }

    pub fn reached(self, value: f64) -> bool {
        match self {
            SnmpLimit::Over(at) => value > at,
            SnmpLimit::Under(at) => value < at,
        }
    }
}

/// The database servers a `database` check knows how to greet (M5-10,
/// D-88). **To add one:** a variant here with its names and port, a module
/// with its greeting in the probe (`probe/src/checks/database/`), and the
/// arm the compiler then asks for there. The form, the API and the docs'
/// list of names follow from [`DbEngine::ALL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbEngine {
    Postgres,
    /// MySQL and MariaDB greet alike.
    Mysql,
    Mssql,
    Mongodb,
}

named!(DbEngine:
    Postgres => "postgres", "PostgreSQL";
    Mysql => "mysql", "MySQL or MariaDB";
    Mssql => "mssql", "Microsoft SQL Server";
    Mongodb => "mongodb", "MongoDB";
);
named!(EventLevel:
    Critical => "critical", "Critical only";
    Error => "error", "Error and worse";
    Warning => "warning", "Warning and worse";
    Information => "information", "Every event (information too)";
);

impl DbEngine {
    /// The TCP port its servers listen on unless told otherwise.
    pub fn default_port(self) -> u16 {
        match self {
            DbEngine::Postgres => 5432,
            DbEngine::Mysql => 3306,
            DbEngine::Mssql => 1433,
            DbEngine::Mongodb => 27017,
        }
    }
}

/// What a `mail` check speaks (M5-16c, D-95).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailProtocol {
    /// Sending and receiving between servers, and submission by clients.
    Smtp,
    Imap,
    Pop3,
}

/// How a `mail` check's connection is encrypted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailSecurity {
    /// The usual port, where the server is asked to start TLS. A server
    /// that doesn't offer it is Down.
    #[default]
    Starttls,
    /// TLS from the first byte, on the port for it.
    Tls,
    /// No encryption: only the greeting.
    None,
}

/// How an `ldap` check's connection is encrypted (M5-16g, D-99).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LdapSecurity {
    /// No encryption: what the probe reads is what the server tells
    /// anyone, and every server answers so.
    #[default]
    None,
    /// The usual port, where the server is asked to start TLS. A server
    /// that doesn't is Down.
    Starttls,
    /// LDAPS: TLS from the first byte, on the port for it.
    Tls,
}

named!(LdapSecurity:
    None => "none", "Not encrypted (port 389)";
    Starttls => "starttls", "StartTLS (port 389, then encrypted)";
    Tls => "tls", "LDAPS: TLS from the start (port 636)";
);

impl LdapSecurity {
    /// The TCP port a directory server listens on for it unless told
    /// otherwise. A domain controller's global catalog (3268, 3269) is
    /// typed.
    pub fn default_port(self) -> u16 {
        match self {
            LdapSecurity::Tls => 636,
            LdapSecurity::None | LdapSecurity::Starttls => 389,
        }
    }
}

named!(MailProtocol:
    Smtp => "smtp", "SMTP";
    Imap => "imap", "IMAP";
    Pop3 => "pop3", "POP3";
);
named!(MailSecurity:
    Starttls => "starttls", "STARTTLS (the usual port, then encrypted)";
    Tls => "tls", "TLS from the start";
    None => "none", "Not encrypted (only the greeting)";
);

impl MailProtocol {
    /// The TCP port its servers listen on for `security` unless told
    /// otherwise. Submission by mail programs (587) is typed.
    pub fn default_port(self, security: MailSecurity) -> u16 {
        match (self, security) {
            (MailProtocol::Smtp, MailSecurity::Tls) => 465,
            (MailProtocol::Smtp, _) => 25,
            (MailProtocol::Imap, MailSecurity::Tls) => 993,
            (MailProtocol::Imap, _) => 143,
            (MailProtocol::Pop3, MailSecurity::Tls) => 995,
            (MailProtocol::Pop3, _) => 110,
        }
    }
}

/// One value an `snmp` check asks for. Only the OID is needed; the rest
/// says how to read it (M5-09, D-87): as a table, as a share of another
/// value, as a rate, in words, and when it is too much or too little.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SnmpOid {
    /// Its object identifier in numbers: `1.3.6.1.2.1.1.3.0`.
    pub oid: String,
    /// What to call it in results. Empty: its OID.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// What it must be: any of these (`1`, `up`). Empty: anything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expect: Vec<String>,
    /// What it must not be: none of these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not: Vec<String>,
    /// The OID is a column of a table: every row of it is asked for (every
    /// interface, every disk), up to [`MAX_SNMP_ROWS`].
    #[serde(default, skip_serializing_if = "is_default")]
    pub walk: bool,
    /// Of a table: the column whose values name the rows (`ifName`).
    /// `None`: a row is called by its index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<String>,
    /// The OID of the value this one is a share of (its capacity; in a
    /// table, the column of it): the result is a percentage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub of: Option<String>,
    /// A counter: the result is how fast it grows, per second.
    #[serde(default, skip_serializing_if = "is_default")]
    pub rate: bool,
    /// The number is multiplied by this (`0.1` for tenths of a degree).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    /// The unit of the result (`°C`, `%`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    /// What its values mean (`1` is `normal`): results show the meaning, and
    /// `expect` and `not` may use it. Such a value is a state, not a
    /// measure: no history of it is kept.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, String>,
    /// Past this, the check is Warning; past `down`, Down. The hub judges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn: Option<SnmpLimit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down: Option<SnmpLimit>,
}

impl SnmpOid {
    /// A value by its OID alone: no name, anything is fine.
    pub fn of(oid: &str) -> Self {
        Self {
            oid: oid.to_string(),
            ..Self::default()
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

    /// The name of the number one row of this table reports.
    pub fn row_label(&self, row: &str) -> String {
        format!("{}: {row}", self.label())
    }

    /// Is the number called `key` in a result this value's? Its own, or a
    /// row's of its table.
    pub fn owns(&self, key: &str) -> bool {
        key == self.label()
            || (self.walk
                && key
                    .strip_prefix(self.label())
                    .is_some_and(|row| row.starts_with(": ")))
    }

    /// The OID's numbers; `None` if it isn't one. A leading dot is allowed.
    pub fn arcs(&self) -> Option<Vec<u64>> {
        oid_arcs(&self.oid)
    }

    /// `Err` with what is wrong, for the person.
    pub fn validate(&self) -> Result<(), CheckError> {
        let label = self.label();
        let wrong = |what: String| Err(CheckError(format!("{label}: {what}")));
        if self.arcs().is_none() {
            return Err(CheckError(format!(
                "\"{}\" isn't an OID in numbers, e.g. 1.3.6.1.2.1.1.3.0",
                self.oid
            )));
        }
        if self.name.trim() != self.name
            || self.name.chars().count() > MAX_SNMP_NAME_CHARS
            || self.name.chars().any(char::is_control)
        {
            return Err(CheckError(format!(
                "a value's name takes at most {MAX_SNMP_NAME_CHARS} characters"
            )));
        }
        for other in [&self.rows, &self.of].into_iter().flatten() {
            if oid_arcs(other).is_none() {
                return wrong(format!("\"{other}\" isn't an OID in numbers"));
            }
        }
        if self.rows.is_some() && !self.walk {
            return wrong("only the rows of a table have names: \"rows\" goes with a walk".into());
        }
        if self.rate && self.of.is_some() {
            return wrong("a value is a rate or a share of another, not both".into());
        }
        if self
            .scale
            .is_some_and(|scale| !scale.is_finite() || scale == 0.0)
        {
            return wrong("the factor is a number other than 0".into());
        }
        if self.unit.trim() != self.unit
            || self.unit.chars().count() > MAX_SNMP_UNIT_CHARS
            || self.unit.chars().any(char::is_control)
        {
            return wrong(format!(
                "a unit takes at most {MAX_SNMP_UNIT_CHARS} characters"
            ));
        }
        if !self.expect.is_empty() && !self.not.is_empty() {
            return wrong("say what it must be or what it must not be, not both".into());
        }
        if self.expect.len() > MAX_SNMP_WORDS || self.not.len() > MAX_SNMP_WORDS {
            return wrong(format!("at most {MAX_SNMP_WORDS} values to compare with"));
        }
        if self.expect.iter().chain(&self.not).any(|word| {
            word.trim().is_empty()
                || word.trim() != word
                || word.chars().count() > MAX_KEYWORD_CHARS
        }) {
            return Err(CheckError(format!(
                "what {label} must be is 1-{MAX_KEYWORD_CHARS} characters, without spaces around"
            )));
        }
        if self.names.len() > MAX_SNMP_WORDS {
            return wrong(format!("at most {MAX_SNMP_WORDS} meanings"));
        }
        if self.names.iter().any(|(value, meaning)| {
            [value, meaning].into_iter().any(|text| {
                text.is_empty()
                    || text.trim() != text
                    || text.chars().count() > MAX_SNMP_MEANING_CHARS
                    || text.chars().any(char::is_control)
            })
        }) {
            return wrong(format!(
                "a value and its meaning take 1-{MAX_SNMP_MEANING_CHARS} characters each"
            ));
        }
        if [self.warn, self.down]
            .into_iter()
            .flatten()
            .any(|limit| !limit.at().is_finite())
        {
            return wrong("a limit is a number".into());
        }
        if let (Some(warn), Some(down)) = (self.warn, self.down) {
            if warn.is_under() != down.is_under() {
                return wrong("its warning and down limits are both over or both under".into());
            }
            let ordered = if warn.is_under() {
                down.at() <= warn.at()
            } else {
                down.at() >= warn.at()
            };
            if !ordered {
                return wrong("the down limit is reached after the warning, not before".into());
            }
        }
        Ok(())
    }
}

/// The numbers of an OID written with dots; `None` if it isn't one. A
/// leading dot is allowed.
pub fn oid_arcs(oid: &str) -> Option<Vec<u64>> {
    let arcs: Vec<u64> = oid
        .strip_prefix('.')
        .unwrap_or(oid)
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
    let valid = (2..=128).contains(&arcs.len()) && arcs[0] <= 2 && (arcs[0] == 2 || arcs[1] < 40);
    valid.then_some(arcs)
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
pub const CHECK_KINDS: [&str; 25] = [
    "ping",
    "http",
    "tcp",
    "tls",
    "dns",
    "database",
    "mail",
    "ldap",
    "ntp",
    "nut",
    "snmp",
    "cpu",
    "memory",
    "disk",
    "disks",
    "smart",
    "temperature",
    "hyperv",
    "file",
    "service",
    "process",
    "interface",
    "eventlog",
    "updates",
    "uptime",
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

/// The most mount points a `disks` check, and drives a `smart` check,
/// leaves out.
pub const MAX_IGNORED_DISKS: usize = 20;

/// The most sensor names a `temperature` check takes.
pub const MAX_SENSOR_NAMES: usize = 20;

/// The most virtual machines a `hyperv` check may name.
pub const MAX_VM_NAMES: usize = 50;

/// The TCP port a UPS server (NUT's `upsd`) listens on.
pub const DEFAULT_NUT_PORT: u16 = 3493;

/// The UDP port time servers answer on.
pub const DEFAULT_NTP_PORT: u16 = 123;

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

    /// An `eventlog` check of a log: an error in the last hour is Down.
    pub fn eventlog(log: impl Into<String>) -> Self {
        CheckSpec::Eventlog {
            log: log.into(),
            source: String::new(),
            ids: Vec::new(),
            level: EventLevel::default(),
            minutes: DEFAULT_EVENT_MINUTES,
            count: DEFAULT_EVENT_COUNT,
            wanted: false,
            warning: false,
        }
    }

    /// A `process` check of a name: at least one must run.
    pub fn process(name: impl Into<String>) -> Self {
        CheckSpec::Process {
            name: name.into(),
            command: String::new(),
            min: DEFAULT_PROCESS_MIN,
            max: None,
        }
    }

    /// An `updates` check with the usual limits.
    pub fn updates() -> Self {
        CheckSpec::Updates {
            security_days: default_update_days(),
            reboot_days: default_update_days(),
            stale_days: default_stale_days(),
        }
    }

    /// The kind, as in the JSON `kind` field (one of [`CHECK_KINDS`]).
    pub fn kind(&self) -> &'static str {
        match self {
            CheckSpec::Ping { .. } => "ping",
            CheckSpec::Http { .. } => "http",
            CheckSpec::Snmp { .. } => "snmp",
            CheckSpec::Tcp { .. } => "tcp",
            CheckSpec::Database { .. } => "database",
            CheckSpec::Mail { .. } => "mail",
            CheckSpec::Ldap { .. } => "ldap",
            CheckSpec::Ntp { .. } => "ntp",
            CheckSpec::Nut { .. } => "nut",
            CheckSpec::Cpu => "cpu",
            CheckSpec::Memory => "memory",
            CheckSpec::Dns { .. } => "dns",
            CheckSpec::Disk { .. } => "disk",
            CheckSpec::Disks { .. } => "disks",
            CheckSpec::Smart { .. } => "smart",
            CheckSpec::Temperature { .. } => "temperature",
            CheckSpec::Hyperv { .. } => "hyperv",
            CheckSpec::File { .. } => "file",
            CheckSpec::Service { .. } => "service",
            CheckSpec::Process { .. } => "process",
            CheckSpec::Interface { .. } => "interface",
            CheckSpec::Eventlog { .. } => "eventlog",
            CheckSpec::Uptime => "uptime",
            CheckSpec::Updates { .. } => "updates",
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
            CheckSpec::Database { engine, host, port } => {
                let port = port.unwrap_or(engine.default_port());
                format!("{host}:{port} ({})", engine.label())
            }
            // As NUT's own tools name a UPS: `ups@host`.
            CheckSpec::Nut { host, port, ups } => {
                let at = if ups.is_empty() {
                    host.clone()
                } else {
                    format!("{ups}@{host}")
                };
                match port {
                    Some(port) => format!("{at}:{port}"),
                    None => at,
                }
            }
            CheckSpec::Ntp { host, port: None }
            | CheckSpec::Ldap {
                host, port: None, ..
            } => host.clone(),
            CheckSpec::Ldap {
                host,
                port: Some(port),
                ..
            } => format!("{host}:{port}"),
            CheckSpec::Ntp {
                host,
                port: Some(port),
            } => format!("{host}:{port}"),
            CheckSpec::Mail {
                protocol,
                host,
                port,
                security,
                ..
            } => {
                let port = port.unwrap_or(protocol.default_port(*security));
                format!("{host}:{port} ({})", protocol.label())
            }
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
            CheckSpec::Smart { ignore } if ignore.is_empty() => "disk health".to_string(),
            CheckSpec::Smart { ignore } => format!("disk health but {}", ignore.join(", ")),
            CheckSpec::Temperature { sensors } if sensors.is_empty() => {
                "hottest sensor".to_string()
            }
            CheckSpec::Temperature { sensors } => format!("hottest of {}", sensors.join(", ")),
            CheckSpec::Hyperv { vms } if vms.is_empty() => "virtual machines".to_string(),
            CheckSpec::Hyperv { vms } => vms.join(", "),
            CheckSpec::File { path, pattern, .. } if pattern.is_empty() => path.clone(),
            CheckSpec::File { path, pattern, .. } => format!("{path} ({pattern})"),
            CheckSpec::Service { name } | CheckSpec::Interface { name } => name.clone(),
            CheckSpec::Eventlog { log, source, .. } if source.is_empty() => log.clone(),
            CheckSpec::Eventlog { log, source, .. } => format!("{log} ({source})"),
            CheckSpec::Process { name, command, .. } if command.is_empty() => name.clone(),
            CheckSpec::Process { name, command, .. } => format!("{name} ({command})"),
            CheckSpec::Cpu | CheckSpec::Memory | CheckSpec::Uptime | CheckSpec::Updates { .. } => {
                "localhost".to_string()
            }
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
            CheckSpec::Nut { host, port, ups } => {
                if *port == Some(0) {
                    return Err(CheckError::new("the port must be 1-65535"));
                }
                if ups.contains(|c: char| c.is_whitespace() || c == '@' || c == '"') {
                    return Err(CheckError::new(
                        "the UPS's name is one word, as the server lists it",
                    ));
                }
                host_name("host", host).map(|_| ())
            }
            CheckSpec::Database { host, port, .. }
            | CheckSpec::Mail { host, port, .. }
            | CheckSpec::Ldap { host, port, .. }
            | CheckSpec::Ntp { host, port } => {
                if *port == Some(0) {
                    return Err(CheckError::new("the port must be 1-65535"));
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
            CheckSpec::Disks { ignore } | CheckSpec::Smart { ignore } => {
                if ignore.len() > MAX_IGNORED_DISKS {
                    return Err(CheckError(format!(
                        "more than {MAX_IGNORED_DISKS} disks to leave out"
                    )));
                }
                ignore
                    .iter()
                    .try_for_each(|disk| required("disk to leave out", disk).map(|_| ()))
            }
            CheckSpec::Temperature { sensors } => {
                if sensors.len() > MAX_SENSOR_NAMES {
                    return Err(CheckError(format!(
                        "more than {MAX_SENSOR_NAMES} sensor names"
                    )));
                }
                sensors
                    .iter()
                    .try_for_each(|sensor| required("sensor name", sensor).map(|_| ()))
            }
            CheckSpec::Hyperv { vms } => {
                if vms.len() > MAX_VM_NAMES {
                    return Err(CheckError(format!(
                        "more than {MAX_VM_NAMES} virtual machine names"
                    )));
                }
                vms.iter()
                    .try_for_each(|vm| required("virtual machine name", vm).map(|_| ()))
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
            CheckSpec::Eventlog {
                log,
                source,
                ids,
                minutes,
                count,
                ..
            } => {
                let log = required("log", log)?;
                if !event_name(log) || !event_name(source) {
                    return Err(CheckError::new(
                        "a log and a source are names as Windows shows them, e.g. System, Microsoft-Windows-Backup, disk",
                    ));
                }
                if ids.len() > MAX_EVENT_IDS || ids.iter().any(|id| *id > u32::from(u16::MAX)) {
                    return Err(CheckError(format!(
                        "an event log check names at most {MAX_EVENT_IDS} event IDs, each 0-65535"
                    )));
                }
                if !(1..=MAX_EVENT_MINUTES).contains(minutes) {
                    return Err(CheckError(format!(
                        "an event log check looks back 1-{MAX_EVENT_MINUTES} minutes (31 days)"
                    )));
                }
                if !(1..=MAX_EVENT_COUNT).contains(count) {
                    return Err(CheckError(format!(
                        "the number of events is 1-{MAX_EVENT_COUNT}"
                    )));
                }
                Ok(())
            }
            CheckSpec::Interface { name } => {
                let name = required("interface name", name)?;
                if name.chars().count() > MAX_INTERFACE_NAME_CHARS
                    || name.contains(['/', '\\'])
                    || name.chars().any(char::is_control)
                {
                    return Err(CheckError::new(
                        "the interface name is as the system shows it, e.g. eth0, Ethernet, Wi-Fi",
                    ));
                }
                Ok(())
            }
            CheckSpec::Process {
                name,
                command,
                min,
                max,
            } => {
                let name = required("process name", name)?;
                let long = |text: &str| text.chars().count() > MAX_PROCESS_NAME_CHARS;
                if long(name) || name.contains(['/', '\\']) || name.chars().any(char::is_control) {
                    return Err(CheckError::new(
                        "the process name is a program's name, not its path, e.g. sqlservr, nginx, backup-*",
                    ));
                }
                if long(command) || command.chars().any(char::is_control) {
                    return Err(CheckError(format!(
                        "the text of the command line is at most {MAX_PROCESS_NAME_CHARS} characters"
                    )));
                }
                let most = max.unwrap_or(MAX_PROCESS_COUNT);
                if *min > MAX_PROCESS_COUNT || most > MAX_PROCESS_COUNT || *min > most {
                    return Err(CheckError(format!(
                        "the numbers of processes are 0-{MAX_PROCESS_COUNT}, the fewest not above the most"
                    )));
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
                    value.validate()?;
                    if !labels.insert(value.label()) {
                        return Err(CheckError(format!("{} is asked for twice", value.label())));
                    }
                }
                Ok(())
            }
            CheckSpec::Uptime => Ok(()),
            CheckSpec::Updates {
                security_days,
                reboot_days,
                stale_days,
            } => {
                let too_long = [security_days, reboot_days, stale_days]
                    .into_iter()
                    .flatten()
                    .any(|days| *days > MAX_UPDATE_DAYS);
                if too_long {
                    return Err(CheckError(format!(
                        "an updates check's limits are 0-{MAX_UPDATE_DAYS} days"
                    )));
                }
                Ok(())
            }
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckDefinition {
    pub id: CheckId,
    pub name: String,
    pub spec: CheckSpec,
    /// What the check needs that isn't in its spec (D-86); empty for most.
    #[serde(default, skip_serializing_if = "Secrets::is_empty")]
    pub secrets: Secrets,
}

/// Everything a probe runs, as the hub sends it (only enabled checks).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// `ntp`: seconds between the machine's clock and the time server's,
    /// whichever is ahead (hub thresholds apply to it).
    pub const OFFSET: &str = "offset";
    /// `ntp`: how far the time server is from a reference clock (1: next
    /// to one).
    pub const STRATUM: &str = "stratum";
    /// `nut`: percent of charge the UPS's battery has.
    pub const CHARGE: &str = "charge";
    /// `nut`: minutes the UPS can carry its load on battery.
    pub const RUNTIME: &str = "runtime";
    /// `nut`: percent of what the UPS can deliver that is drawn.
    pub const LOAD: &str = "load";
    /// `hyperv`: how many of the virtual machines that count are running.
    pub const RUNNING: &str = "running";
    /// `hyperv`: how many virtual machines count.
    pub const VMS: &str = "vms";
    /// `eventlog`: how many events match (the hub judges it; counted up
    /// to `MAX_EVENT_COUNT`).
    pub const EVENTS: &str = "events";
    /// `process`: how many processes match.
    pub const PROCESSES: &str = "processes";
    /// `process`: percent of the machine's CPU they use together.
    pub const CPU: &str = "cpu";
    /// `process`: megabytes of memory they use together.
    pub const MEMORY: &str = "memory";
    /// `smart`: how many drives say they are failing.
    pub const FAILING: &str = "failing";
    /// `smart`: how many drives show a warning sign (the hub's Warning).
    /// `nut`, `hyperv`: how many such signs the UPS, the machines show.
    pub const WARNINGS: &str = "warnings";
    /// `smart`: how many drives report their health.
    pub const DRIVES: &str = "drives";
    /// `smart`: degrees Celsius of the hottest drive. Absent where none says.
    /// `temperature`: degrees Celsius of the hottest sensor.
    pub const TEMPERATURE: &str = "temperature";
    /// `temperature`: how many sensors count.
    pub const SENSORS: &str = "sensors";
    /// `smart`: percent of its rated life the most worn drive has used.
    /// Absent where none says.
    pub const WEAR: &str = "wear";
    /// `smart`: bad sectors of all drives together. Absent where none says.
    pub const BAD_SECTORS: &str = "bad_sectors";
    /// `interface`: megabits a second that came in since the report
    /// before. Absent in a probe's first report.
    pub const RECEIVED: &str = "received";
    /// `interface`: megabits a second that went out.
    pub const SENT: &str = "sent";
    /// `interface`: packets with an error, in or out, since the report
    /// before.
    pub const ERRORS: &str = "errors";
    /// `interface`: the speed of the link, in megabits a second. Absent
    /// where the system doesn't say (a virtual interface).
    pub const SPEED: &str = "speed";
    /// `updates`: how many updates wait to be installed.
    pub const UPDATES: &str = "updates";
    /// `updates`: how many of them are security updates.
    pub const SECURITY: &str = "security";
    /// `updates`: days security updates have waited, as far as the probe
    /// has seen (the hub's limit applies to it). Absent when none waits.
    pub const SECURITY_DAYS: &str = "security_days";
    /// `updates`: 1 when the machine must restart to finish an update,
    /// else 0. Absent where the system doesn't say.
    pub const REBOOT: &str = "reboot";
    /// `updates`: days a restart has been needed (the hub's limit applies
    /// to it). Absent when none is.
    pub const REBOOT_DAYS: &str = "reboot_days";
    /// `updates`: days since the system last looked for updates (the hub's
    /// limit applies to it). Absent where the system doesn't say.
    pub const LOOKED_DAYS: &str = "looked_days";

    pub const PERCENT: &str = "%";
    pub const GIGABYTES: &str = "GB";
    pub const SECONDS: &str = "s";
    pub const DAYS: &str = "d";
    pub const HOURS: &str = "h";
    pub const MINUTES: &str = "min";
    pub const MILLISECONDS: &str = "ms";
    pub const MEGABYTES: &str = "MB";
    pub const MEGABITS_PER_SECOND: &str = "Mbit/s";
    pub const CELSIUS: &str = "°C";
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
    fn an_interface_check_names_an_interface_of_either_system() {
        let interface = |name: &str| CheckSpec::Interface { name: name.into() };
        assert_eq!(
            serde_json::to_string(&interface("eth0")).unwrap(),
            r#"{"kind":"interface","name":"eth0"}"#
        );
        for name in [
            "eth0",
            "enp3s0",
            "br-4f1a",
            "Ethernet",
            "Wi-Fi",
            "vEthernet (Default Switch)",
            "Ethernet 2",
        ] {
            assert_eq!(interface(name).validate(), Ok(()), "{name}");
            assert_eq!(interface(name).target(), name);
            assert_eq!(interface(name).kind(), "interface");
        }
        for name in ["", " eth0", "../eth0", "a\\b", "eth0\n", &"n".repeat(257)] {
            let error = interface(name).validate().unwrap_err();
            assert!(error.0.contains("interface name"), "{name}: {}", error.0);
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

    #[test]
    fn a_smart_check_covers_every_drive_but_those_left_out() {
        let all = CheckSpec::Smart { ignore: vec![] };
        assert_eq!(serde_json::to_string(&all).unwrap(), r#"{"kind":"smart"}"#);
        assert_eq!(
            serde_json::from_str::<CheckSpec>(r#"{"kind":"smart"}"#).unwrap(),
            all
        );
        assert_eq!(
            (all.kind(), all.target().as_str()),
            ("smart", "disk health")
        );
        let some = CheckSpec::Smart {
            ignore: vec!["WDC WD40*".into(), "S3Z9NB0K".into()],
        };
        assert_eq!(some.validate(), Ok(()));
        assert_eq!(some.target(), "disk health but WDC WD40*, S3Z9NB0K");
        let blank = CheckSpec::Smart {
            ignore: vec![" ".into()],
        };
        assert!(blank.validate().unwrap_err().0.contains("can't be empty"));
        let many = CheckSpec::Smart {
            ignore: vec!["X".into(); MAX_IGNORED_DISKS + 1],
        };
        assert!(many.validate().is_err());
    }

    #[test]
    fn a_hyperv_check_names_the_machines_that_must_run_or_none() {
        let all = CheckSpec::Hyperv { vms: vec![] };
        let json = r#"{"kind":"hyperv"}"#;
        assert_eq!(serde_json::to_string(&all).unwrap(), json);
        assert_eq!(serde_json::from_str::<CheckSpec>(json).unwrap(), all);
        assert_eq!(
            (all.kind(), all.target().as_str()),
            ("hyperv", "virtual machines")
        );
        let some = CheckSpec::Hyperv {
            vms: vec!["web01".into(), "sql-*".into()],
        };
        assert_eq!(
            serde_json::to_string(&some).unwrap(),
            r#"{"kind":"hyperv","vms":["web01","sql-*"]}"#
        );
        assert_eq!(some.validate(), Ok(()));
        assert_eq!(some.target(), "web01, sql-*");
        let blank = CheckSpec::Hyperv {
            vms: vec![" ".into()],
        };
        assert!(blank.validate().unwrap_err().0.contains("can't be empty"));
        let many = CheckSpec::Hyperv {
            vms: vec!["x".into(); MAX_VM_NAMES + 1],
        };
        assert!(many.validate().unwrap_err().0.contains("more than 50"));
    }

    #[test]
    fn an_ldap_check_names_a_directory_server_and_how_it_is_encrypted() {
        let ldap = |port, security, verify| CheckSpec::Ldap {
            host: "dc01.corp.example.com".into(),
            port,
            security,
            verify,
        };
        // Not encrypted on the usual port, unless said.
        let plain = ldap(None, LdapSecurity::None, true);
        let json = r#"{"kind":"ldap","host":"dc01.corp.example.com"}"#;
        assert_eq!(serde_json::to_string(&plain).unwrap(), json);
        assert_eq!(serde_json::from_str::<CheckSpec>(json).unwrap(), plain);
        assert_eq!(
            (plain.kind(), plain.target().as_str()),
            ("ldap", "dc01.corp.example.com")
        );
        assert_eq!(plain.validate(), Ok(()));

        let typed = ldap(Some(3269), LdapSecurity::Tls, false);
        assert_eq!(
            serde_json::to_string(&typed).unwrap(),
            r#"{"kind":"ldap","host":"dc01.corp.example.com","port":3269,"security":"tls","verify":false}"#
        );
        assert_eq!(typed.target(), "dc01.corp.example.com:3269");
        assert_eq!(typed.validate(), Ok(()));
        for security in LdapSecurity::ALL {
            assert_eq!(LdapSecurity::parse(security.as_str()), Some(*security));
        }
        assert_eq!(LdapSecurity::Starttls.default_port(), 389);
        assert_eq!(LdapSecurity::Tls.default_port(), 636);

        assert!(ldap(Some(0), LdapSecurity::None, true).validate().is_err());
        let nowhere = CheckSpec::Ldap {
            host: "ldap://dc01".into(),
            port: None,
            security: LdapSecurity::None,
            verify: true,
        };
        assert!(nowhere.validate().is_err());
    }

    #[test]
    fn a_nut_check_names_a_ups_server_and_maybe_the_ups() {
        let nut = |host: &str, port, ups: &str| CheckSpec::Nut {
            host: host.into(),
            port,
            ups: ups.into(),
        };
        let plain = nut("nas.lan", None, "");
        let json = r#"{"kind":"nut","host":"nas.lan"}"#;
        assert_eq!(serde_json::to_string(&plain).unwrap(), json);
        assert_eq!(serde_json::from_str::<CheckSpec>(json).unwrap(), plain);
        assert_eq!((plain.kind(), plain.target().as_str()), ("nut", "nas.lan"));
        assert_eq!(plain.validate(), Ok(()));

        let named = nut("nas.lan", Some(3500), "rack");
        assert_eq!(
            serde_json::to_string(&named).unwrap(),
            r#"{"kind":"nut","host":"nas.lan","port":3500,"ups":"rack"}"#
        );
        assert_eq!(named.target(), "rack@nas.lan:3500");
        assert_eq!(nut("nas.lan", None, "ups").target(), "ups@nas.lan");
        assert_eq!(named.validate(), Ok(()));

        assert!(nut("", None, "ups").validate().is_err());
        assert!(nut("nas.lan", Some(0), "").validate().is_err());
        for name in ["my ups", "ups@nas", "a\"b"] {
            assert!(
                nut("nas.lan", None, name)
                    .validate()
                    .unwrap_err()
                    .0
                    .contains("one word"),
                "{name}"
            );
        }
    }

    #[test]
    fn an_ntp_check_names_a_time_server() {
        let ntp = |host: &str, port| CheckSpec::Ntp {
            host: host.into(),
            port,
        };
        let plain = ntp("pool.ntp.org", None);
        let json = r#"{"kind":"ntp","host":"pool.ntp.org"}"#;
        assert_eq!(serde_json::to_string(&plain).unwrap(), json);
        assert_eq!(serde_json::from_str::<CheckSpec>(json).unwrap(), plain);
        assert_eq!(
            (plain.kind(), plain.target().as_str()),
            ("ntp", "pool.ntp.org")
        );
        assert_eq!(plain.validate(), Ok(()));

        let other = ntp("dc01.lan", Some(1123));
        assert_eq!(
            serde_json::to_string(&other).unwrap(),
            r#"{"kind":"ntp","host":"dc01.lan","port":1123}"#
        );
        assert_eq!(other.target(), "dc01.lan:1123");
        assert!(ntp("", None).validate().is_err());
        assert!(ntp("ntp://pool", None).validate().is_err());
        assert!(ntp("pool.ntp.org", Some(0)).validate().is_err());
    }

    #[test]
    fn a_mail_check_names_its_protocol_and_how_it_is_encrypted() {
        let mail = |protocol, port, security, verify| CheckSpec::Mail {
            protocol,
            host: "mail.example.com".into(),
            port,
            security,
            verify,
        };
        // The usual: STARTTLS on the protocol's port, the certificate verified.
        let plain = mail(MailProtocol::Smtp, None, MailSecurity::Starttls, true);
        let json = r#"{"kind":"mail","protocol":"smtp","host":"mail.example.com"}"#;
        assert_eq!(serde_json::to_string(&plain).unwrap(), json);
        assert_eq!(serde_json::from_str::<CheckSpec>(json).unwrap(), plain);
        assert_eq!(
            (plain.kind(), plain.target().as_str()),
            ("mail", "mail.example.com:25 (SMTP)")
        );
        assert_eq!(plain.validate(), Ok(()));

        let typed = mail(MailProtocol::Pop3, Some(1995), MailSecurity::Tls, false);
        let json = r#"{"kind":"mail","protocol":"pop3","host":"mail.example.com","port":1995,"security":"tls","verify":false}"#;
        assert_eq!(serde_json::to_string(&typed).unwrap(), json);
        assert_eq!(serde_json::from_str::<CheckSpec>(json).unwrap(), typed);
        assert_eq!(typed.target(), "mail.example.com:1995 (POP3)");

        // The port follows how the connection is encrypted.
        for (protocol, security, target) in [
            (
                MailProtocol::Smtp,
                MailSecurity::Tls,
                "mail.example.com:465 (SMTP)",
            ),
            (
                MailProtocol::Imap,
                MailSecurity::Starttls,
                "mail.example.com:143 (IMAP)",
            ),
            (
                MailProtocol::Imap,
                MailSecurity::Tls,
                "mail.example.com:993 (IMAP)",
            ),
            (
                MailProtocol::Pop3,
                MailSecurity::None,
                "mail.example.com:110 (POP3)",
            ),
        ] {
            assert_eq!(mail(protocol, None, security, true).target(), target);
        }
        for protocol in MailProtocol::ALL {
            assert_eq!(MailProtocol::parse(protocol.as_str()), Some(*protocol));
        }
        for security in MailSecurity::ALL {
            assert_eq!(MailSecurity::parse(security.as_str()), Some(*security));
        }

        assert!(
            mail(MailProtocol::Smtp, Some(0), MailSecurity::Tls, true)
                .validate()
                .is_err()
        );
        let nowhere = CheckSpec::Mail {
            protocol: MailProtocol::Imap,
            host: "imap://mail".into(),
            port: None,
            security: MailSecurity::Starttls,
            verify: true,
        };
        assert!(nowhere.validate().is_err());
        assert!(
            serde_json::from_str::<CheckSpec>(r#"{"kind":"mail","protocol":"x400","host":"m"}"#)
                .is_err()
        );
    }

    #[test]
    fn a_temperature_check_counts_every_sensor_or_those_named() {
        let all = CheckSpec::Temperature { sensors: vec![] };
        assert_eq!(
            serde_json::to_string(&all).unwrap(),
            r#"{"kind":"temperature"}"#
        );
        assert_eq!(
            serde_json::from_str::<CheckSpec>(r#"{"kind":"temperature"}"#).unwrap(),
            all
        );
        assert_eq!(
            (all.kind(), all.target().as_str()),
            ("temperature", "hottest sensor")
        );
        let some = CheckSpec::Temperature {
            sensors: vec!["coretemp*".into(), "acpitz*".into()],
        };
        assert_eq!(some.validate(), Ok(()));
        assert_eq!(some.target(), "hottest of coretemp*, acpitz*");
        let blank = CheckSpec::Temperature {
            sensors: vec![" ".into()],
        };
        assert!(blank.validate().unwrap_err().0.contains("can't be empty"));
        let many = CheckSpec::Temperature {
            sensors: vec!["x".into(); MAX_SENSOR_NAMES + 1],
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
            CheckSpec::Database {
                engine: DbEngine::Postgres,
                host: "db".into(),
                port: None,
            },
            CheckSpec::Mail {
                protocol: MailProtocol::Smtp,
                host: "mail".into(),
                port: None,
                security: MailSecurity::default(),
                verify: true,
            },
            CheckSpec::Ldap {
                host: "dc01".into(),
                port: None,
                security: LdapSecurity::default(),
                verify: true,
            },
            CheckSpec::Ntp {
                host: "pool.ntp.org".into(),
                port: None,
            },
            CheckSpec::Nut {
                host: "nas".into(),
                port: None,
                ups: String::new(),
            },
            CheckSpec::snmp("10.0.0.2", vec![SnmpOid::of("1.3.6.1.2.1.1.3.0")]),
            CheckSpec::Cpu,
            CheckSpec::Memory,
            CheckSpec::Disk { path: None },
            CheckSpec::Disks { ignore: vec![] },
            CheckSpec::Smart { ignore: vec![] },
            CheckSpec::Temperature { sensors: vec![] },
            CheckSpec::Hyperv { vms: vec![] },
            file("/var/backups", "*.bak"),
            CheckSpec::Service {
                name: "nginx".into(),
            },
            CheckSpec::process("nginx"),
            CheckSpec::Interface {
                name: "eth0".into(),
            },
            CheckSpec::eventlog("System"),
            CheckSpec::updates(),
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
            expect: expect.map(str::to_string).into_iter().collect(),
            ..SnmpOid::default()
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
            r#"{"kind":"snmp","host":"switch.lan","port":1161,"version":"v3","user":"monitor","auth":"sha256","privacy":"aes128","oids":[{"oid":"1.3.6.1.2.1.2.2.1.8.1","name":"Uplink","expect":["1"]}]}"#
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
    fn a_value_says_how_it_is_read_and_its_json_leaves_out_the_rest() {
        let toner = SnmpOid {
            name: "Toner".into(),
            walk: true,
            rows: Some("1.3.6.1.2.1.43.11.1.1.6".into()),
            of: Some("1.3.6.1.2.1.43.11.1.1.8".into()),
            warn: Some(SnmpLimit::Under(20.0)),
            down: Some(SnmpLimit::Under(5.0)),
            ..SnmpOid::of("1.3.6.1.2.1.43.11.1.1.9")
        };
        assert_eq!(toner.validate(), Ok(()));
        let json = serde_json::to_string(&toner).unwrap();
        assert_eq!(
            json,
            r#"{"oid":"1.3.6.1.2.1.43.11.1.1.9","name":"Toner","walk":true,"rows":"1.3.6.1.2.1.43.11.1.1.6","of":"1.3.6.1.2.1.43.11.1.1.8","warn":{"under":20.0},"down":{"under":5.0}}"#
        );
        assert_eq!(serde_json::from_str::<SnmpOid>(&json).unwrap(), toner);

        // A number of the table is the value's; another value's isn't.
        assert_eq!(toner.row_label("Black"), "Toner: Black");
        assert!(toner.owns("Toner: Black"));
        assert!(!toner.owns("Toner level: Black"));
        let plain = SnmpOid {
            name: "Toner".into(),
            ..SnmpOid::of("1.3.6.1.2.1.1.3.0")
        };
        assert!(plain.owns("Toner"));
        assert!(!plain.owns("Toner: Black"));

        assert!(SnmpLimit::Under(20.0).reached(19.9));
        assert!(!SnmpLimit::Under(20.0).reached(20.0));
        assert!(SnmpLimit::Over(60.0).reached(60.5));
        assert!(!SnmpLimit::Over(60.0).reached(60.0));
    }

    #[test]
    fn a_value_that_makes_no_sense_is_refused_with_the_reason() {
        let base = || SnmpOid {
            name: "Fan".into(),
            ..SnmpOid::of("1.3.6.1.4.1.9.9.1")
        };
        let error = |value: SnmpOid| value.validate().unwrap_err().to_string();
        assert!(
            error(SnmpOid {
                rows: Some("1.3.6.1.2".into()),
                ..base()
            })
            .starts_with("Fan: only the rows of a table have names")
        );
        assert_eq!(
            error(SnmpOid {
                walk: true,
                rows: Some("ifName".into()),
                ..base()
            }),
            "Fan: \"ifName\" isn't an OID in numbers"
        );
        assert!(
            error(SnmpOid {
                rate: true,
                of: Some("1.3.6.1.2".into()),
                ..base()
            })
            .contains("a rate or a share")
        );
        for scale in [0.0, f64::NAN, f64::INFINITY] {
            assert!(
                error(SnmpOid {
                    scale: Some(scale),
                    ..base()
                })
                .contains("the factor")
            );
        }
        assert!(
            error(SnmpOid {
                unit: "degrees Celsius".into(),
                ..base()
            })
            .contains("a unit takes at most 12")
        );
        assert!(
            error(SnmpOid {
                expect: vec!["1".into()],
                not: vec!["2".into()],
                ..base()
            })
            .contains("not both")
        );
        assert!(
            error(SnmpOid {
                not: vec![" ".into()],
                ..base()
            })
            .starts_with("what Fan must be")
        );
        assert!(
            error(SnmpOid {
                names: BTreeMap::from([("1".to_string(), String::new())]),
                ..base()
            })
            .contains("a value and its meaning")
        );
        assert!(
            error(SnmpOid {
                warn: Some(SnmpLimit::Over(60.0)),
                down: Some(SnmpLimit::Under(5.0)),
                ..base()
            })
            .contains("both over or both under")
        );
        assert!(
            error(SnmpOid {
                warn: Some(SnmpLimit::Over(60.0)),
                down: Some(SnmpLimit::Over(50.0)),
                ..base()
            })
            .contains("after the warning")
        );
        assert!(
            error(SnmpOid {
                warn: Some(SnmpLimit::Under(5.0)),
                down: Some(SnmpLimit::Under(20.0)),
                ..base()
            })
            .contains("after the warning")
        );
        // The check refuses what one of its values refuses.
        let spec = v2c(vec![SnmpOid {
            scale: Some(0.0),
            ..base()
        }]);
        assert!(spec.validate().is_err());
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

#[cfg(test)]
mod database_tests {
    use super::*;

    #[test]
    fn a_database_check_names_its_engine_and_may_leave_out_the_port() {
        let plain = CheckSpec::Database {
            engine: DbEngine::Postgres,
            host: "db.lan".into(),
            port: None,
        };
        let json = serde_json::to_string(&plain).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"database","engine":"postgres","host":"db.lan"}"#
        );
        assert_eq!(serde_json::from_str::<CheckSpec>(&json).unwrap(), plain);
        assert_eq!(plain.kind(), "database");
        assert_eq!(plain.target(), "db.lan:5432 (PostgreSQL)");
        assert_eq!(plain.validate(), Ok(()));
        assert!(plain.secret_names().is_empty(), "it logs in nowhere");

        let other = CheckSpec::Database {
            engine: DbEngine::Mssql,
            host: "10.0.0.7".into(),
            port: Some(14330),
        };
        assert_eq!(
            serde_json::to_string(&other).unwrap(),
            r#"{"kind":"database","engine":"mssql","host":"10.0.0.7","port":14330}"#
        );
        assert_eq!(other.target(), "10.0.0.7:14330 (Microsoft SQL Server)");

        for (host, port) in [("", None), ("db lan", None), ("db.lan", Some(0))] {
            let wrong = CheckSpec::Database {
                engine: DbEngine::Mysql,
                host: host.into(),
                port,
            };
            assert!(wrong.validate().is_err(), "{host:?} {port:?}");
        }
        let unknown = r#"{"kind":"database","engine":"oracle","host":"db.lan"}"#;
        assert!(serde_json::from_str::<CheckSpec>(unknown).is_err());
    }

    #[test]
    fn every_engine_has_its_names_and_its_port() {
        assert_eq!(DbEngine::ALL.len(), 4);
        let mut ports = Vec::new();
        for engine in DbEngine::ALL {
            assert_eq!(DbEngine::parse(engine.as_str()), Some(*engine));
            assert!(!engine.label().is_empty());
            // The name in the JSON is the one `parse` reads.
            assert_eq!(
                serde_json::to_string(engine).unwrap(),
                format!("\"{}\"", engine.as_str())
            );
            ports.push(engine.default_port());
        }
        assert_eq!(ports, [5432, 3306, 1433, 27017]);
    }
}

#[cfg(test)]
mod updates_tests {
    use super::*;

    #[test]
    fn an_updates_check_has_its_usual_limits_unless_told_otherwise() {
        let usual: CheckSpec = serde_json::from_str(r#"{"kind":"updates"}"#).unwrap();
        assert_eq!(usual, CheckSpec::updates());
        assert_eq!(
            serde_json::to_string(&usual).unwrap(),
            r#"{"kind":"updates","security_days":0,"reboot_days":0,"stale_days":14}"#
        );
        assert_eq!(
            (usual.kind(), usual.target().as_str()),
            ("updates", "localhost")
        );

        // `null` turns a limit off, and stays off through the JSON.
        let quiet: CheckSpec = serde_json::from_str(
            r#"{"kind":"updates","security_days":7,"reboot_days":null,"stale_days":null}"#,
        )
        .unwrap();
        assert_eq!(
            quiet,
            CheckSpec::Updates {
                security_days: Some(7),
                reboot_days: None,
                stale_days: None,
            }
        );
        let again: CheckSpec =
            serde_json::from_str(&serde_json::to_string(&quiet).unwrap()).unwrap();
        assert_eq!(again, quiet);
        assert_eq!(quiet.validate(), Ok(()));

        let long = CheckSpec::Updates {
            security_days: Some(MAX_UPDATE_DAYS),
            reboot_days: Some(MAX_UPDATE_DAYS + 1),
            stale_days: None,
        };
        assert!(long.validate().unwrap_err().0.contains("0-365 days"));
    }
}

#[cfg(test)]
mod process_tests {
    use super::*;

    fn process(name: &str, command: &str, min: u32, max: Option<u32>) -> CheckSpec {
        CheckSpec::Process {
            name: name.into(),
            command: command.into(),
            min,
            max,
        }
    }

    #[test]
    fn a_process_check_names_a_program_and_how_many_may_run() {
        let plain = CheckSpec::process("sqlservr");
        let json = r#"{"kind":"process","name":"sqlservr"}"#;
        assert_eq!(serde_json::to_string(&plain).unwrap(), json);
        assert_eq!(serde_json::from_str::<CheckSpec>(json).unwrap(), plain);
        assert_eq!(
            (plain.kind(), plain.target().as_str()),
            ("process", "sqlservr")
        );

        let exact = process("java", "app.jar", 2, Some(2));
        let json = r#"{"kind":"process","name":"java","command":"app.jar","min":2,"max":2}"#;
        assert_eq!(serde_json::to_string(&exact).unwrap(), json);
        assert_eq!(serde_json::from_str::<CheckSpec>(json).unwrap(), exact);
        assert_eq!(exact.target(), "java (app.jar)");

        for fine in [
            plain,
            exact,
            process("backup-*.exe", "", 1, None),
            // Must not run at all.
            process("teamviewer", "", 0, Some(0)),
            process(
                "My Agent",
                "--config C:\\agent.toml",
                1,
                Some(MAX_PROCESS_COUNT),
            ),
        ] {
            assert_eq!(fine.validate(), Ok(()), "{fine:?}");
        }
        for (wrong, reason) in [
            (process("", "", 1, None), "process name"),
            (process("  ", "", 1, None), "process name"),
            (process("/usr/sbin/nginx", "", 1, None), "not its path"),
            (process("C:\\x\\a.exe", "", 1, None), "not its path"),
            (process("a\nb", "", 1, None), "not its path"),
            (process(&"n".repeat(257), "", 1, None), "not its path"),
            (process("java", &"c".repeat(257), 1, None), "at most 256"),
            (process("java", "a\tb", 1, None), "at most 256"),
            (process("nginx", "", 3, Some(2)), "fewest not above"),
            (process("nginx", "", 10_001, None), "0-10000"),
            (process("nginx", "", 1, Some(10_001)), "0-10000"),
        ] {
            let error = wrong.validate().expect_err(&format!("{wrong:?}"));
            assert!(error.0.contains(reason), "{wrong:?}: {error}");
        }
    }
}

#[cfg(test)]
mod eventlog_tests {
    use super::*;

    #[test]
    fn an_eventlog_check_names_a_log_and_what_counts_in_it() {
        let plain = CheckSpec::eventlog("System");
        let json = r#"{"kind":"eventlog","log":"System"}"#;
        assert_eq!(serde_json::to_string(&plain).unwrap(), json);
        assert_eq!(serde_json::from_str::<CheckSpec>(json).unwrap(), plain);
        assert_eq!(
            (plain.kind(), plain.target().as_str()),
            ("eventlog", "System")
        );
        assert_eq!(plain.validate(), Ok(()));

        let disk = CheckSpec::Eventlog {
            log: "System".into(),
            source: "disk".into(),
            ids: vec![7, 51, 153],
            level: EventLevel::Warning,
            minutes: 1440,
            count: 3,
            wanted: false,
            warning: true,
        };
        let json = r#"{"kind":"eventlog","log":"System","source":"disk","ids":[7,51,153],"level":"warning","minutes":1440,"count":3,"warning":true}"#;
        assert_eq!(serde_json::to_string(&disk).unwrap(), json);
        assert_eq!(serde_json::from_str::<CheckSpec>(json).unwrap(), disk);
        assert_eq!(disk.target(), "System (disk)");
        assert_eq!(disk.validate(), Ok(()));

        let with = |change: &dyn Fn(&mut CheckSpec)| {
            let mut spec = CheckSpec::eventlog("Microsoft-Windows-Backup");
            change(&mut spec);
            spec.validate().map_err(|e| e.0)
        };
        let set = |f: fn(&mut String, &mut String, &mut Vec<u32>, &mut u32, &mut u32)| {
            move |spec: &mut CheckSpec| {
                if let CheckSpec::Eventlog {
                    log,
                    source,
                    ids,
                    minutes,
                    count,
                    ..
                } = spec
                {
                    f(log, source, ids, minutes, count);
                }
            }
        };
        assert_eq!(with(&|_| {}), Ok(()));
        assert_eq!(
            with(&set(|log, source, _, minutes, count| {
                *log = "Microsoft-Windows-TaskScheduler/Operational".into();
                *source = "Microsoft-Windows-TaskScheduler".into();
                *minutes = MAX_EVENT_MINUTES;
                *count = MAX_EVENT_COUNT;
            })),
            Ok(())
        );
        for (change, reason) in [
            (set(|log, _, _, _, _| log.clear()) as _, "log"),
            (
                // What would end the text the log is asked with.
                set(|log, _, _, _, _| *log = "System' or '1'='1".into()),
                "names as Windows shows them",
            ),
            (
                set(|_, source, _, _, _| *source = "disk\"]".into()),
                "names as Windows shows them",
            ),
            (
                set(|_, source, _, _, _| *source = "s".repeat(257)),
                "names as Windows shows them",
            ),
            (
                set(|_, _, ids, _, _| *ids = vec![1; MAX_EVENT_IDS + 1]),
                "at most 20 event IDs",
            ),
            (set(|_, _, ids, _, _| *ids = vec![70_000]), "each 0-65535"),
            (set(|_, _, _, minutes, _| *minutes = 0), "1-44640 minutes"),
            (
                set(|_, _, _, minutes, _| *minutes = MAX_EVENT_MINUTES + 1),
                "1-44640 minutes",
            ),
            (set(|_, _, _, _, count| *count = 0), "1-1000"),
            (set(|_, _, _, _, count| *count = 1001), "1-1000"),
        ] {
            let change: Box<dyn Fn(&mut CheckSpec)> = Box::new(change);
            let error = with(&*change).unwrap_err();
            assert!(error.contains(reason), "{error}");
        }
        assert_eq!(EventLevel::parse("warning"), Some(EventLevel::Warning));
        assert_eq!(EventLevel::ALL.len(), 4);
    }
}
