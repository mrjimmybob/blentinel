//! API types and domain rules shared by the server (`ssr`) and the browser
//! (`hydrate`). This is the **single definition** of every JSON shape the hub
//! API returns, and of the rules the UI and server must agree on.
//!
//! Compiled for both targets, so it may only depend on `serde`: no database,
//! no async runtime, no browser APIs.

use std::collections::BTreeSet;

use common::checks::{CheckSpec, Metric};
use serde::{Deserialize, Serialize};

// ===========================================================================
// Severity
// ===========================================================================

/// How bad things are for a probe or a company.
///
/// Variants are declared **worst first**, so the derived `Ord` sorts
/// `Critical < Degraded < Healthy` and a plain ascending sort lists problems first.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// An active probe reports at least one device down: a known outage.
    Critical,
    /// A probe stopped reporting: its data is stale, so the state is unknown.
    Degraded,
    #[default]
    Healthy,
}

impl Severity {
    /// A probe's severity. An expired probe is `Degraded` whatever its (stale)
    /// device counts say; only a live probe can prove an outage.
    pub fn for_probe(probe_active: bool, devices_down: i64) -> Self {
        if !probe_active {
            Severity::Degraded
        } else if devices_down > 0 {
            Severity::Critical
        } else {
            Severity::Healthy
        }
    }

    /// A company is as bad as its worst probe. `active_devices_down` counts
    /// only devices reported down by *active* probes.
    pub fn for_company(active_devices_down: i64, expired_probes: i64) -> Self {
        if active_devices_down > 0 {
            Severity::Critical
        } else if expired_probes > 0 {
            Severity::Degraded
        } else {
            Severity::Healthy
        }
    }

    /// Lowercase name, as serialised (`"critical"`, `"degraded"`, `"healthy"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Critical => "critical",
            Severity::Degraded => "degraded",
            Severity::Healthy => "healthy",
        }
    }
}

// ===========================================================================
// Identity helpers
// ===========================================================================

/// First 8 characters of a probe ID (the hex of its public key), for logs and
/// compact display.
pub fn short_probe_id(probe_id: &str) -> &str {
    probe_id.get(..8).unwrap_or(probe_id)
}

// ===========================================================================
// API types (JSON shapes)
// ===========================================================================

/// `GET /api/dashboard/companies` (also the archive viewer).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DashboardCompany {
    pub company_id: String,
    /// Display name (the company record's; the ID until someone sets one).
    pub name: String,
    pub total_probes: i64,
    pub active_probes: i64,
    pub expired_probes: i64,
    pub devices_up: i64,
    /// All devices down in each probe's latest report, including stale ones
    /// from expired probes. Use `severity` to judge the company.
    pub devices_down: i64,
    pub severity: Severity,
    /// ISO 8601; `None` if no reports yet.
    pub last_report: Option<String>,
}

/// `GET /api/company/{company_id}/probes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompanyProbe {
    pub probe_id: String,
    /// Name from the hub's probe whitelist; empty if unnamed.
    pub probe_name: String,
    pub hostname: Option<String>,
    /// The site's display name (its record's), from the probe's latest report.
    pub site: Option<String>,
    /// `"active"` or `"expired"`.
    pub status: String,
    pub severity: Severity,
    pub last_seen_at: Option<String>,
    pub devices_up: i64,
    pub devices_down: i64,
}

impl CompanyProbe {
    pub fn is_active(&self) -> bool {
        self.status == "active"
    }

    /// The whitelist name, or the short probe ID followed by `…` if unnamed.
    pub fn display_name(&self) -> String {
        if self.probe_name.is_empty() {
            format!("{}…", short_probe_id(&self.probe_id))
        } else {
            self.probe_name.clone()
        }
    }
}

/// `GET /api/probe/{probe_id}/devices`: one check's result from the probe's
/// latest report. Checks deleted since then are left out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeDevice {
    pub check_id: String,
    pub name: String,
    /// The check's kind (`ping`, `http`, …).
    pub kind: String,
    pub target: String,
    /// `"Up"` or `"Down"`.
    pub status: String,
    pub message: Option<String>,
    pub latency_ms: Option<i64>,
    /// The measured values, headline first.
    pub metrics: Vec<Metric>,
}

impl ProbeDevice {
    /// The headline metric, as `42.5%` (`None` without metrics).
    pub fn headline(&self) -> Option<String> {
        self.metrics
            .first()
            .map(|m| format!("{:.1}{}", m.value, m.unit))
    }
}

/// `GET /api/company/{company_id}/uptime`: one time bucket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UptimeBucket {
    /// `"YYYY-MM-DD HH:MM"`, UTC.
    pub bucket: String,
    pub up_count: i64,
    pub down_count: i64,
}

/// `GET /api/admin/probes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminProbe {
    pub probe_id: String,
    pub company_id: String,
    pub status: String,
    pub last_seen_at: Option<String>,
    pub first_seen_at: Option<String>,
}

/// `GET /api/admin/archives`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveMetadata {
    pub id: i64,
    pub filename: String,
    pub created_at: String,
    pub cutoff_date: String,
    pub size_mb: i64,
}

/// `GET /api/admin/storage-info`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorageInfo {
    pub current_db_size_mb: u64,
    pub warn_threshold_mb: u64,
    pub retention_days: u32,
    pub last_archive_date: Option<String>,
    pub last_archive_size_mb: Option<i64>,
    pub warning: bool,
}

/// `GET /api/silences`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertSilence {
    pub id: i64,
    /// `check` ([`SILENCE_CHECK`]): `scope_id` is a check ID.
    pub scope_type: String,
    pub scope_id: String,
    /// The company the silenced check belongs to (`None` if it was deleted).
    pub company_id: Option<String>,
    pub reason: String,
    pub created_at: String,
    pub expires_at: Option<String>,
}

/// [`AlertSilence::scope_type`] of a silence on one check.
pub const SILENCE_CHECK: &str = "check";

// ===========================================================================
// Users, roles and permissions (proposal 0001)
// ===========================================================================

/// What a user is. Global roles see every company; the others will be limited
/// to their assigned companies once scoping is enforced (M1-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Everything, including managing Owners. At least one active Owner exists.
    Owner,
    /// Everything except managing Owners.
    Admin,
    /// Views data and silences alerts.
    Technician,
    /// Read-only (internal staff).
    Viewer,
    /// Read-only view of their own companies (needs M1-04).
    Client,
}

impl Role {
    pub const ALL: [Role; 5] = [
        Role::Owner,
        Role::Admin,
        Role::Technician,
        Role::Viewer,
        Role::Client,
    ];

    /// Lowercase name, as serialised and stored.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Technician => "technician",
            Role::Viewer => "viewer",
            Role::Client => "client",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Role::ALL.into_iter().find(|r| r.as_str() == value)
    }

    /// Display name ("Owner", …).
    pub fn label(self) -> &'static str {
        match self {
            Role::Owner => "Owner",
            Role::Admin => "Admin",
            Role::Technician => "Technician",
            Role::Viewer => "Viewer",
            Role::Client => "Client",
        }
    }

    /// Owner and Admin always see every company.
    pub fn is_global(self) -> bool {
        matches!(self, Role::Owner | Role::Admin)
    }
}

/// Something a request needs to be allowed to do. Every API route declares
/// one (`crate::routes`); the UI uses the same check to hide what it can't do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    /// Dashboards, company and probe views, uptime, archives, silence list.
    ViewData,
    /// Create and remove alert silences.
    Silence,
    /// The admin page: storage size, archive list, company and probe listings.
    ViewAdmin,
    /// Delete data, remove probes, create archives.
    ManageData,
    /// Create and edit users (only Owners edit Owners, see `service::users`).
    ManageUsers,
    /// Edit companies and sites: names, contacts, notes, addresses (within
    /// the user's company scope). Archiving is `ManageData`.
    ManageCompanies,
}

impl Permission {
    pub const ALL: [Permission; 6] = [
        Permission::ViewData,
        Permission::Silence,
        Permission::ViewAdmin,
        Permission::ManageData,
        Permission::ManageUsers,
        Permission::ManageCompanies,
    ];
}

/// The single authorization rule: may `role` do `permission`?
pub fn authorize(role: Role, permission: Permission) -> bool {
    use Permission::*;
    match permission {
        ViewData => true,
        Silence | ManageCompanies => {
            matches!(role, Role::Owner | Role::Admin | Role::Technician)
        }
        ViewAdmin | ManageData | ManageUsers => role.is_global(),
    }
}

/// A user as the API shows it (`/api/auth/me`, `/api/users`). Never includes
/// the password hash.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub role: Role,
    /// Which companies the user sees (ignored for Owner and Admin, who see
    /// all: use [`UserInfo::effective_scope`]).
    pub scope: CompanyScope,
    pub is_active: bool,
    /// Must set a new password before doing anything else.
    pub must_change_password: bool,
    pub created_at: String,
    pub last_login_at: Option<String>,
}

impl UserInfo {
    pub fn can(&self, permission: Permission) -> bool {
        authorize(self.role, permission)
    }

    /// The companies this user may see and act on. Owner and Admin: all.
    pub fn effective_scope(&self) -> CompanyScope {
        if self.role.is_global() {
            CompanyScope::All
        } else {
            self.scope.clone()
        }
    }
}

/// Which companies a user sees (D-45). One person monitoring their own
/// company never needs more than `All`, the default; an IT provider can give
/// each technician, viewer or customer login exactly their companies.
/// JSON: `{"mode": "all"}`, `{"mode": "only", "companies": [...]}`,
/// `{"mode": "except", "companies": [...]}`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", content = "companies", rename_all = "lowercase")]
pub enum CompanyScope {
    /// Every company, including ones registered later.
    #[default]
    All,
    /// Only these companies.
    Only(BTreeSet<String>),
    /// Every company except these, including ones registered later.
    #[serde(rename = "except")]
    AllExcept(BTreeSet<String>),
}

impl CompanyScope {
    pub fn allows(&self, company_id: &str) -> bool {
        match self {
            CompanyScope::All => true,
            CompanyScope::Only(ids) => ids.contains(company_id),
            CompanyScope::AllExcept(ids) => !ids.contains(company_id),
        }
    }

    /// Stored form: (`all` | `only` | `except`, listed companies).
    pub fn mode(&self) -> &'static str {
        match self {
            CompanyScope::All => "all",
            CompanyScope::Only(_) => "only",
            CompanyScope::AllExcept(_) => "except",
        }
    }

    pub fn companies(&self) -> Vec<String> {
        match self {
            CompanyScope::All => Vec::new(),
            CompanyScope::Only(ids) | CompanyScope::AllExcept(ids) => ids.iter().cloned().collect(),
        }
    }

    /// Build from the stored form; an unknown mode is `None`.
    pub fn from_parts(mode: &str, companies: impl IntoIterator<Item = String>) -> Option<Self> {
        let ids = companies.into_iter().collect();
        match mode {
            "all" => Some(CompanyScope::All),
            "only" => Some(CompanyScope::Only(ids)),
            "except" => Some(CompanyScope::AllExcept(ids)),
            _ => None,
        }
    }
}

/// `POST /api/auth/login`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// `GET /api/setup`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetupStatus {
    /// No user exists yet: the setup wizard must create the first Owner.
    pub needed: bool,
}

/// `POST /api/setup`: create the first Owner with the setup code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupRequest {
    pub code: String,
    pub username: String,
    pub display_name: String,
    pub password: String,
}

/// `POST /api/me/password`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// `POST /api/users`. Without a password, the hub generates a temporary one
/// (returned once) that must be changed at first login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub display_name: String,
    #[serde(default)]
    pub email: Option<String>,
    pub role: Role,
    /// Defaults to all companies.
    #[serde(default)]
    pub scope: CompanyScope,
}

/// `POST /api/users/{id}`: the editable fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateUserRequest {
    pub display_name: String,
    #[serde(default)]
    pub email: Option<String>,
    pub role: Role,
    #[serde(default)]
    pub scope: CompanyScope,
    pub is_active: bool,
}

// ===========================================================================
// Companies and sites (proposal 0002 §4.1, D-44)
// ===========================================================================

/// A company: `GET /api/companies`, and the head of `CompanyDetail`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompanyInfo {
    /// The `company_id` probes send. Immutable: reports are stored under it.
    pub id: String,
    pub name: String,
    pub notes: Option<String>,
    pub contact_name: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    /// Registered by a report and never edited yet.
    pub is_new: bool,
    /// Hidden from the dashboard (its probes can still report).
    pub archived: bool,
    pub created_at: String,
    pub site_count: i64,
    pub probe_count: i64,
}

/// A physical location of a company.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SiteInfo {
    /// The `site` probes send. Immutable.
    pub id: String,
    pub name: String,
    pub address: Option<String>,
    pub notes: Option<String>,
    pub is_new: bool,
}

/// `GET /api/companies/{id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompanyDetail {
    pub company: CompanyInfo,
    pub sites: Vec<SiteInfo>,
}

impl CompanyInfo {
    /// Without the provider's internal notes and contacts (what a Client sees).
    pub fn without_internal_fields(self) -> Self {
        Self {
            notes: None,
            contact_name: None,
            contact_email: None,
            contact_phone: None,
            ..self
        }
    }
}

impl CompanyDetail {
    /// Without internal notes and contacts, on the company and its sites.
    pub fn without_internal_fields(self) -> Self {
        Self {
            company: self.company.without_internal_fields(),
            sites: self
                .sites
                .into_iter()
                .map(|site| SiteInfo {
                    notes: None,
                    ..site
                })
                .collect(),
        }
    }
}

/// `POST /api/companies/{id}`: the editable fields.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateCompanyRequest {
    pub name: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub contact_name: Option<String>,
    #[serde(default)]
    pub contact_email: Option<String>,
    #[serde(default)]
    pub contact_phone: Option<String>,
}

/// `POST /api/companies/{id}/sites/{site}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateSiteRequest {
    pub name: String,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// `POST /api/companies`: a new company. `id` is its permanent key (a slug
/// such as `acme-ltd`); the UI derives it from the name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateCompanyRequest {
    pub id: String,
    pub name: String,
}

/// `POST /api/companies/{id}/sites`: a new site of a company.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateSiteRequest {
    pub id: String,
    pub name: String,
}

/// A slug for `name`: lowercase ASCII letters and digits joined by `-`
/// ("ACME Ltd." → `acme-ltd`). The UI suggests it; the hub validates IDs with
/// [`is_valid_slug`].
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').chars().take(64).collect()
}

/// A company or site ID: 1–64 of `a-z 0-9 - _ .`, starting with a letter or digit.
pub fn is_valid_slug(id: &str) -> bool {
    let valid =
        |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.');
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(valid)
        && id.starts_with(|c: char| c.is_ascii_alphanumeric())
}

/// `POST /api/companies/{id}/archive`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveCompanyRequest {
    pub archived: bool,
}

// ===========================================================================
// Probes and enrollment codes (M2-01, D-47)
// ===========================================================================

/// A probe the hub knows (`GET /api/probes`): added by an enrollment code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeInfo {
    /// Hex of its Ed25519 public key.
    pub id: String,
    pub name: String,
    pub company_id: String,
    pub company_name: String,
    pub site_id: String,
    pub site_name: String,
    /// `active`, or `revoked` (its reports are refused).
    pub state: String,
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub probe_version: Option<String>,
    pub enrolled_at: String,
    /// Its last report, if any.
    pub last_seen_at: Option<String>,
    /// Seconds between two reports.
    pub interval_secs: u32,
    /// The last update it refused or rolled back, and why (D-55).
    #[serde(default)]
    pub update_failed: Option<common::models::UpdateFailure>,
}

/// An enrollment code that can still be used (`GET /api/enrollment-codes`).
/// The code itself is shown only once, when created.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnrollmentCodeInfo {
    pub id: String,
    pub company_id: String,
    pub site_id: String,
    pub probe_name: Option<String>,
    pub created_by: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    /// `None`: any number of probes until it expires or is revoked.
    pub max_uses: Option<i64>,
    pub uses: i64,
}

/// `POST /api/enrollment-codes`: "Add probe".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateCodeRequest {
    pub company_id: String,
    pub site_id: String,
    /// The probe's name; empty: its hostname.
    #[serde(default)]
    pub probe_name: Option<String>,
    /// How many probes may enroll with it; omitted or `null`: any number
    /// until it expires or is revoked.
    #[serde(default)]
    pub max_uses: Option<u32>,
    /// How long it stays valid (default 24 hours).
    #[serde(default = "default_day")]
    pub valid_hours: u32,
    /// Where probes reach the hub, e.g. `https://hub.example.com:3443`.
    pub hub_url: String,
}

fn default_day() -> u32 {
    24
}

/// The answer to "Add probe": the code, shown only now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreatedCode {
    pub code: String,
    pub info: EnrollmentCodeInfo,
    /// Where probes reach the hub (in the code too): the wizard's download links.
    pub hub_url: String,
}

/// A probe binary the hub offers (`GET /api/probe-downloads`, M2-09, D-53).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeDownload {
    /// Rust target triple, e.g. `x86_64-unknown-linux-musl`.
    pub target: String,
    /// `linux` or `windows`.
    pub os: String,
    /// `x86_64`, `arm64`, `armv7`.
    pub arch: String,
    pub size: u64,
    /// Hex SHA-256 of the file: the one-line commands check the download with it.
    pub sha256: String,
    /// The release it comes from (D-55).
    pub version: String,
}

/// A signed probe release on the hub (D-55).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeRelease {
    pub version: String,
    pub released: String,
    /// The protocol version its probes speak.
    pub protocol: u16,
    /// e.g. `linux x86_64`, in the hub's platform order.
    pub platforms: Vec<String>,
}

/// The admin page's "Probe updates" (`GET /api/admin/probe-updates`, D-55).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeUpdates {
    /// The release probes are offered; `None`: no updates.
    pub approved: Option<String>,
    pub approved_by: Option<String>,
    pub approved_at: Option<String>,
    /// Newest first.
    pub releases: Vec<ProbeRelease>,
    /// The hub trusts a release key (it was built with one): without one it
    /// can't take a release.
    pub has_release_key: bool,
    /// Every probe, with its version and last failed update.
    pub probes: Vec<ProbeInfo>,
}

/// Approve a release for every probe, or none (`POST /api/admin/probe-updates/approve`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApproveRequest {
    /// `None`: stop offering updates.
    pub version: Option<String>,
}

impl ProbeDownload {
    /// Where to download it from a hub reached at `hub_url`.
    pub fn url(&self, hub_url: &str) -> String {
        format!(
            "{}/download/probe/{}",
            hub_url.trim_end_matches('/'),
            self.target
        )
    }

    /// The one command that downloads it, checks its SHA-256 and installs it
    /// with `code`: a shell line for Linux (as a user who may `sudo`), a
    /// PowerShell line for Windows (an elevated window).
    ///
    /// The certificate isn't checked for the download (`-k`, the PowerShell
    /// equivalent): the SHA-256, shown on the hub's own HTTPS page, is what
    /// proves the file is the hub's. The probe then pins the hub itself.
    ///
    /// Both fail closed: the shell line is one `&&` chain; the PowerShell
    /// line stops at any error (`$ErrorActionPreference='Stop'`: otherwise a
    /// failing statement is skipped and the next one runs) and runs the probe
    /// only inside `if (hash matches)`. The hash uses .NET directly, so it
    /// needs no module (Windows PowerShell 5.1 and PowerShell 7 alike).
    pub fn install_command(&self, hub_url: &str, code: &str) -> String {
        let url = self.url(hub_url);
        if self.os == "windows" {
            format!(
                "$ErrorActionPreference='Stop'; $f=\"$env:TEMP\\blentinel-probe.exe\"; \
                 $o=@{{UseBasicParsing=$true}}; \
                 if ($PSVersionTable.PSVersion.Major -ge 6) {{ $o.SkipCertificateCheck=$true }} \
                 else {{ [Net.ServicePointManager]::SecurityProtocol='Tls12'; \
                 [Net.ServicePointManager]::ServerCertificateValidationCallback={{$true}} }}; \
                 Invoke-WebRequest '{url}' -OutFile $f @o; \
                 $h=[BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([IO.File]::ReadAllBytes($f))).Replace('-',''); \
                 if ($h -eq '{sha}') {{ & $f install --code {code} }} else {{ throw 'checksum mismatch' }}",
                sha = self.sha256.to_uppercase(),
            )
        } else {
            format!(
                "curl -fsSLk -o probe '{url}' && echo '{sha}  probe' | sha256sum -c - \
                 && chmod +x probe && sudo ./probe install --code {code}",
                sha = self.sha256,
            )
        }
    }

    /// "Linux x86_64", "Windows x86_64".
    pub fn label(&self) -> String {
        let os = if self.os == "windows" {
            "Windows"
        } else {
            "Linux"
        };
        format!("{os} {}", self.arch)
    }
}

/// `POST /api/probes/{id}`: rename, move to another site of its company,
/// set how often it reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateProbeRequest {
    pub name: String,
    pub site_id: String,
    /// Seconds between two reports (`common::checks::MIN_INTERVAL_SECS`..=`MAX_INTERVAL_SECS`).
    pub interval_secs: u32,
}

// ===========================================================================
// Checks (M2-04, D-49)
// ===========================================================================

/// A check as the API shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckInfo {
    pub id: String,
    pub probe_id: String,
    pub name: String,
    /// What it checks; `kind` is inside (`{"kind": "tcp", "host": …}`).
    pub spec: CheckSpec,
    /// For people: `10.0.0.1`, `db:5432`, a URL.
    pub target: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
    pub updated_by: Option<String>,
}

/// `POST /api/probes/{id}/checks` and `POST /api/checks/{id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckRequest {
    pub name: String,
    pub spec: CheckSpec,
    /// Default `true`.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// `POST /api/probes/{id}/checks/copy`: copy another probe's checks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopyChecksRequest {
    pub from_probe_id: String,
}

/// Does the probe run what the hub wants it to?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSync {
    /// It runs the current checks.
    InSync,
    /// It gets them with the answer to its next report.
    Pending,
    /// It refused them; see `ProbeChecks::config_error`.
    Rejected,
}

/// `GET /api/probes/{id}/checks`: a probe's checks and whether it runs them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeChecks {
    pub probe_id: String,
    pub interval_secs: u32,
    /// The revision the probe should run.
    pub config_revision: String,
    /// The revision the probe last said it runs (`None` before its first report).
    pub applied_revision: Option<String>,
    pub sync: ConfigSync,
    /// Why the probe refused the current config.
    pub config_error: Option<String>,
    /// Every check of the probe, disabled ones too, by name.
    pub checks: Vec<CheckInfo>,
}

// ===========================================================================
// Audit log (M1-06, D-46)
// ===========================================================================

/// One entry of `GET /api/audit`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    /// RFC 3339.
    pub at: String,
    /// Who did it; `None` for the hub itself or the server console (`hub user`).
    pub username: Option<String>,
    pub actor_ip: Option<String>,
    /// What happened, e.g. `user.create`, `silence.delete`, `login.failed`.
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub company_id: Option<String>,
    /// Extra facts, as JSON text (e.g. the fields that changed).
    pub details: Option<String>,
}

/// Filters of `GET /api/audit` (all optional), newest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AuditQuery {
    /// Exact username (case-insensitive).
    pub user: Option<String>,
    /// Action or action prefix: `user` matches `user.create`, `user.update`, …
    pub action: Option<String>,
    /// Target ID or company ID.
    pub target: Option<String>,
    /// Entries at or after this RFC 3339 time.
    pub from: Option<String>,
    /// Entries before this RFC 3339 time.
    pub to: Option<String>,
    /// Page: entries with a smaller ID than this.
    pub before_id: Option<i64>,
    /// At most this many (default 200, at most 1000).
    pub limit: Option<u32>,
}

/// A generated temporary password, shown once (create, reset).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemporaryPassword {
    pub user: UserInfo,
    pub temporary_password: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_matrix() {
        // (role, [ViewData, Silence, ViewAdmin, ManageData, ManageUsers, ManageCompanies])
        let expected = [
            (Role::Owner, [true, true, true, true, true, true]),
            (Role::Admin, [true, true, true, true, true, true]),
            (Role::Technician, [true, true, false, false, false, true]),
            (Role::Viewer, [true, false, false, false, false, false]),
            (Role::Client, [true, false, false, false, false, false]),
        ];
        for (role, allowed) in expected {
            for (perm, want) in Permission::ALL.into_iter().zip(allowed) {
                assert_eq!(authorize(role, perm), want, "{role:?} {perm:?}");
            }
        }
    }

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn scopes_allow_all_only_or_all_except() {
        assert!(CompanyScope::All.allows("anything"));
        let only = CompanyScope::Only(set(&["c1", "c3"]));
        assert!(only.allows("c1") && !only.allows("c2"));
        let except = CompanyScope::AllExcept(set(&["c8"]));
        assert!(
            except.allows("c9") && !except.allows("c8"),
            "later companies included"
        );
    }

    #[test]
    fn scopes_have_a_stable_json_and_stored_form() {
        let except = CompanyScope::AllExcept(set(&["c8"]));
        let json = serde_json::to_value(&except).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "mode": "except", "companies": ["c8"] })
        );
        assert_eq!(
            serde_json::from_value::<CompanyScope>(json).unwrap(),
            except
        );
        assert_eq!(
            serde_json::from_value::<CompanyScope>(serde_json::json!({ "mode": "all" })).unwrap(),
            CompanyScope::All
        );
        for scope in [CompanyScope::All, CompanyScope::Only(set(&["a"])), except] {
            assert_eq!(
                CompanyScope::from_parts(scope.mode(), scope.companies()),
                Some(scope)
            );
        }
        assert_eq!(CompanyScope::from_parts("some", Vec::new()), None);
    }

    #[test]
    fn owners_and_admins_always_see_every_company() {
        let user = |role| UserInfo {
            id: "u".into(),
            username: "u".into(),
            display_name: "u".into(),
            email: None,
            role,
            scope: CompanyScope::Only(set(&["c1"])),
            is_active: true,
            must_change_password: false,
            created_at: String::new(),
            last_login_at: None,
        };
        assert_eq!(user(Role::Admin).effective_scope(), CompanyScope::All);
        assert_eq!(user(Role::Owner).effective_scope(), CompanyScope::All);
        assert_eq!(
            user(Role::Viewer).effective_scope(),
            CompanyScope::Only(set(&["c1"]))
        );
    }

    #[test]
    fn slugs_are_made_from_names_and_validated() {
        assert_eq!(slugify("ACME Ltd."), "acme-ltd");
        assert_eq!(slugify("  Warehouse #2 (north) "), "warehouse-2-north");
        assert_eq!(slugify("Ñandú"), "and");
        for ok in ["acme", "acme-ltd", "hq_1", "site.2"] {
            assert!(is_valid_slug(ok), "{ok}");
        }
        for bad in ["", "Acme", "-acme", "a b", "a/b", &"a".repeat(65)] {
            assert!(!is_valid_slug(bad), "{bad}");
        }
    }

    #[test]
    fn roles_round_trip_through_their_names() {
        for role in Role::ALL {
            assert_eq!(Role::parse(role.as_str()), Some(role));
            assert_eq!(
                serde_json::to_string(&role).unwrap(),
                format!("\"{}\"", role.as_str())
            );
        }
        assert_eq!(Role::parse("root"), None);
    }

    #[test]
    fn expired_probe_is_degraded_even_with_devices_down() {
        assert_eq!(Severity::for_probe(false, 3), Severity::Degraded);
        assert_eq!(Severity::for_probe(false, 0), Severity::Degraded);
    }

    #[test]
    fn active_probe_is_critical_only_with_devices_down() {
        assert_eq!(Severity::for_probe(true, 1), Severity::Critical);
        assert_eq!(Severity::for_probe(true, 0), Severity::Healthy);
    }

    #[test]
    fn company_with_a_live_outage_is_critical_even_if_another_probe_expired() {
        // Previously the card showed amber here and hid the outage.
        assert_eq!(Severity::for_company(1, 1), Severity::Critical);
    }

    #[test]
    fn company_with_only_stale_problems_is_degraded() {
        assert_eq!(Severity::for_company(0, 1), Severity::Degraded);
        assert_eq!(Severity::for_company(0, 0), Severity::Healthy);
    }

    #[test]
    fn severity_sorts_worst_first_and_serialises_lowercase() {
        let mut all = vec![Severity::Healthy, Severity::Critical, Severity::Degraded];
        all.sort();
        assert_eq!(
            all,
            [Severity::Critical, Severity::Degraded, Severity::Healthy]
        );
        assert_eq!(
            serde_json::to_string(&Severity::Degraded).unwrap(),
            r#""degraded""#
        );
        assert_eq!(Severity::Critical.as_str(), "critical");
    }

    #[test]
    fn short_probe_id_takes_eight_chars_and_tolerates_short_ids() {
        assert_eq!(short_probe_id("0123456789abcdef"), "01234567");
        assert_eq!(short_probe_id("abc"), "abc");
    }

    #[test]
    fn a_device_headlines_its_first_metric() {
        let mut device = ProbeDevice {
            check_id: "c1".into(),
            name: "Disk".into(),
            kind: "disk".into(),
            target: "/".into(),
            status: "Up".into(),
            message: None,
            latency_ms: None,
            metrics: vec![
                Metric::new("usage", 42.26, "%"),
                Metric::new("free", 9.0, "GB"),
            ],
        };
        assert_eq!(device.headline().as_deref(), Some("42.3%"));
        device.metrics.clear();
        assert_eq!(device.headline(), None);
    }

    fn download(os: &str, target: &str) -> ProbeDownload {
        ProbeDownload {
            target: target.into(),
            os: os.into(),
            arch: "x86_64".into(),
            size: 7,
            sha256: "ab12".repeat(16),
            version: "0.1.0".into(),
        }
    }

    #[test]
    fn the_linux_command_downloads_checks_and_installs() {
        let linux = download("linux", "x86_64-unknown-linux-musl");
        let command = linux.install_command("https://hub.example.com:3443/", "BLT1-ABC");
        assert_eq!(
            command,
            format!(
                "curl -fsSLk -o probe 'https://hub.example.com:3443/download/probe/x86_64-unknown-linux-musl' \
                 && echo '{}  probe' | sha256sum -c - && chmod +x probe && sudo ./probe install --code BLT1-ABC",
                "ab12".repeat(16)
            )
        );
        assert_eq!(linux.label(), "Linux x86_64");
    }

    #[test]
    fn the_windows_command_runs_the_probe_only_when_the_hash_matches() {
        let windows = download("windows", "x86_64-pc-windows-msvc");
        let command = windows.install_command("http://hub:3000", "BLT1-ABC");
        // Any error stops the line (a failed download or hash can't be skipped)…
        assert!(
            command.starts_with("$ErrorActionPreference='Stop';"),
            "{command}"
        );
        // …and the probe runs only inside the branch where the hash matched.
        assert!(
            command.contains(&format!(
                "if ($h -eq '{}') {{ & $f install --code BLT1-ABC }} else {{ throw 'checksum mismatch' }}",
                "AB12".repeat(16)
            )),
            "{command}"
        );
        assert_eq!(command.matches("& $f").count(), 1, "run in one place only");
        assert!(command.contains("'http://hub:3000/download/probe/x86_64-pc-windows-msvc'"));
        assert!(!command.contains('\n'), "one line");
    }

    #[test]
    fn config_sync_serialises_in_snake_case() {
        assert_eq!(
            serde_json::to_string(&ConfigSync::InSync).unwrap(),
            r#""in_sync""#
        );
    }

    #[test]
    fn display_name_falls_back_to_the_short_id() {
        let mut probe = CompanyProbe {
            probe_id: "0123456789abcdef".into(),
            probe_name: String::new(),
            hostname: None,
            site: None,
            status: "active".into(),
            severity: Severity::Healthy,
            last_seen_at: None,
            devices_up: 0,
            devices_down: 0,
        };
        assert_eq!(probe.display_name(), "01234567…");
        probe.probe_name = "hq".into();
        assert_eq!(probe.display_name(), "hq");
    }
}
