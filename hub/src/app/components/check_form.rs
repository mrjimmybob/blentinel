//! The add/edit dialog for a check (M2-05). The form shows the settings of
//! the chosen kind, and checks them with the same code as the hub and the
//! probe (`common::checks`) before anything is sent.

use common::checks::{
    CHECK_KINDS, CheckSpec, DEFAULT_PING_COUNT, DEFAULT_TLS_PORT, DnsRecord, HttpHeader,
    HttpMethod, HttpOptions,
};
use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::post_for;
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::model::{
    AlertAfter, CheckInfo, CheckRequest, Limit, MetricLimits, ResolvedLimits, ThresholdLevel,
    ThresholdMetric,
};

/// A kind as the UI names it.
pub(crate) fn kind_label(kind: &str) -> &'static str {
    match kind {
        "ping" => "Ping",
        "http" => "Web page (HTTP)",
        "tcp" => "TCP port",
        "tls" => "TLS certificate",
        "dns" => "DNS name",
        "cpu" => "CPU usage",
        "memory" => "Memory usage",
        "disk" => "Disk usage (one disk)",
        "disks" => "Disk usage (every disk, found by the probe)",
        "file" => "File or folder (a backup that must be recent)",
        "service" => "Service (a Windows service, a systemd unit)",
        "uptime" => "Uptime",
        _ => "Unknown",
    }
}

/// The form's text fields. Only those of the chosen kind are shown and used.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CheckFields {
    pub kind: String,
    pub host: String,
    pub port: String,
    pub url: String,
    pub path: String,
    /// `tls`, `http`: accept a certificate a client wouldn't.
    pub unverified: bool,
    /// `http`: `get` or `head` (empty: `get`).
    pub method: String,
    /// `http`: the status codes expected (empty: 200-299).
    pub status: String,
    /// `http`: text the page must, and must not, contain.
    pub contains: String,
    pub absent: String,
    /// `http`: a redirect is the answer.
    pub no_redirects: bool,
    /// `http`: the headers to send, one `Name: value` a line.
    pub headers: String,
    /// `ping`: how many echoes (empty: 4).
    pub count: String,
    /// `ping`: the TCP port to try without an answer (empty: none).
    pub tcp_port: String,
    /// `service`: the service's name.
    pub service: String,
    /// `dns`: the record type, as in the JSON (empty: `a`).
    pub record: String,
    /// `dns`: the server to ask (empty: the machine's).
    pub server: String,
    /// `dns`: the answers that are fine, separated by commas (empty: any).
    pub expect: String,
    /// `disks`: the mount points to leave out, separated by commas.
    pub ignore: String,
    /// `file`: the names that count in a folder (empty: every file).
    pub pattern: String,
    /// `file`: the subfolders too.
    pub recursive: bool,
    /// `file`: the smallest size that is fine, in MB (empty: any).
    pub min_mb: String,
}

const BYTES_PER_MB: f64 = 1024.0 * 1024.0;

/// A size typed in megabytes (`500`, `0.5`), in bytes; empty: none.
fn parse_min_mb(text: &str) -> Result<Option<u64>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse::<f64>()
        .ok()
        .filter(|mb| mb.is_finite() && *mb > 0.0 && *mb <= 1e9)
        .map(|mb| Some((mb * BYTES_PER_MB).round() as u64))
        .ok_or_else(|| format!("{text}: the smallest size is a number of MB, e.g. 500 or 0.5"))
}

fn min_mb_text(bytes: Option<u64>) -> String {
    bytes
        // Not rounded: dividing by a power of two is exact, so the text gives
        // back the same bytes (rounding it made 512 bytes 524 on the next save).
        .map(|bytes| format!("{}", bytes as f64 / BYTES_PER_MB))
        .unwrap_or_default()
}

/// Headers as typed: one `Name: value` a line; empty lines don't count.
fn parse_headers(text: &str) -> Result<Vec<HttpHeader>, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (name, value) = line
                .split_once(':')
                .ok_or_else(|| format!("\"{line}\": a header is written Name: value"))?;
            Ok(HttpHeader {
                name: name.trim().to_string(),
                value: value.trim().to_string(),
            })
        })
        .collect()
}

fn headers_text(headers: &[HttpHeader]) -> String {
    headers
        .iter()
        .map(|h| format!("{}: {}", h.name, h.value))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Which inputs a kind needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Inputs {
    pub host: bool,
    pub port: bool,
    pub url: bool,
    pub path: bool,
    pub unverified: bool,
    /// The `http` settings beyond the URL.
    pub http: bool,
    /// The `file` settings.
    pub file: bool,
    /// The `ping` settings beyond the host.
    pub ping: bool,
    /// The `disks` setting.
    pub disks: bool,
    /// The `dns` settings.
    pub dns: bool,
    /// The `service` setting.
    pub service: bool,
}

/// A list typed with commas between its items; empty items don't count.
fn comma_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

impl CheckFields {
    pub fn from_spec(spec: &CheckSpec) -> Self {
        let mut fields = Self {
            kind: spec.kind().to_string(),
            ..Self::default()
        };
        match spec {
            CheckSpec::Ping {
                host,
                count,
                tcp_port,
            } => {
                fields.host = host.clone();
                if *count != DEFAULT_PING_COUNT {
                    fields.count = count.to_string();
                }
                fields.tcp_port = tcp_port.map(|p| p.to_string()).unwrap_or_default();
            }
            CheckSpec::Http { url, options } => {
                fields.url = url.clone();
                fields.method = options.method.as_str().to_string();
                fields.status = options.status.clone();
                fields.contains = options.contains.clone();
                fields.absent = options.absent.clone();
                fields.no_redirects = !options.follow_redirects;
                fields.headers = headers_text(&options.headers);
                fields.unverified = !options.verify;
            }
            CheckSpec::Tcp { host, port } => {
                fields.host = host.clone();
                fields.port = port.to_string();
            }
            CheckSpec::Tls { host, port, verify } => {
                fields.host = host.clone();
                fields.port = port.to_string();
                fields.unverified = !verify;
            }
            CheckSpec::Disk { path } => fields.path = path.clone().unwrap_or_default(),
            CheckSpec::Disks { ignore } => fields.ignore = ignore.join(", "),
            CheckSpec::Dns {
                name,
                record,
                server,
                expect,
            } => {
                fields.host = name.clone();
                fields.record = record.as_str().to_string();
                fields.server = server.clone().unwrap_or_default();
                fields.expect = expect.join(", ");
            }
            CheckSpec::File {
                path,
                pattern,
                recursive,
                min_bytes,
            } => {
                fields.path = path.clone();
                fields.pattern = pattern.clone();
                fields.recursive = *recursive;
                fields.min_mb = min_mb_text(*min_bytes);
            }
            CheckSpec::Service { name } => fields.service = name.clone(),
            CheckSpec::Cpu | CheckSpec::Memory | CheckSpec::Uptime => {}
        }
        fields
    }

    pub fn inputs(&self) -> Inputs {
        match self.kind.as_str() {
            "ping" => Inputs {
                host: true,
                ping: true,
                ..Inputs::default()
            },
            "tcp" => Inputs {
                host: true,
                port: true,
                ..Inputs::default()
            },
            "tls" => Inputs {
                host: true,
                port: true,
                unverified: true,
                ..Inputs::default()
            },
            "http" => Inputs {
                url: true,
                http: true,
                unverified: true,
                ..Inputs::default()
            },
            "disk" => Inputs {
                path: true,
                ..Inputs::default()
            },
            "disks" => Inputs {
                disks: true,
                ..Inputs::default()
            },
            "dns" => Inputs {
                dns: true,
                ..Inputs::default()
            },
            "service" => Inputs {
                service: true,
                ..Inputs::default()
            },
            "file" => Inputs {
                file: true,
                ..Inputs::default()
            },
            _ => Inputs::default(),
        }
    }

    /// The spec these fields describe, if it's valid (`CheckSpec::validate`).
    pub fn to_spec(&self) -> Result<CheckSpec, String> {
        let text = |value: &str| value.trim().to_string();
        let port = |default: Option<u16>| match (self.port.trim(), default) {
            ("", Some(default)) => Ok(default),
            (port, _) => port
                .parse()
                .map_err(|_| "the port must be 1-65535".to_string()),
        };
        let spec = match self.kind.as_str() {
            "ping" => CheckSpec::Ping {
                host: text(&self.host),
                count: match self.count.trim() {
                    "" => DEFAULT_PING_COUNT,
                    count => count
                        .parse()
                        .map_err(|_| format!("{count}: how many echoes is a number"))?,
                },
                tcp_port: match self.tcp_port.trim() {
                    "" => None,
                    port => Some(
                        port.parse()
                            .map_err(|_| "the TCP port must be 1-65535".to_string())?,
                    ),
                },
            },
            "http" => CheckSpec::Http {
                url: text(&self.url),
                options: HttpOptions {
                    method: match self.method.as_str() {
                        "" => HttpMethod::Get,
                        method => HttpMethod::parse(method)
                            .ok_or_else(|| "the method is GET or HEAD".to_string())?,
                    },
                    status: text(&self.status),
                    // As typed: spaces can be part of what to look for.
                    contains: self.contains.clone(),
                    absent: self.absent.clone(),
                    follow_redirects: !self.no_redirects,
                    headers: parse_headers(&self.headers)?,
                    verify: !self.unverified,
                },
            },
            "tcp" => CheckSpec::Tcp {
                host: text(&self.host),
                port: port(None)?,
            },
            "tls" => CheckSpec::Tls {
                host: text(&self.host),
                port: port(Some(DEFAULT_TLS_PORT))?,
                verify: !self.unverified,
            },
            "cpu" => CheckSpec::Cpu,
            "memory" => CheckSpec::Memory,
            "disk" => CheckSpec::Disk {
                path: Some(text(&self.path)).filter(|p| !p.is_empty()),
            },
            "disks" => CheckSpec::Disks {
                ignore: comma_list(&self.ignore),
            },
            "dns" => CheckSpec::Dns {
                name: text(&self.host),
                record: match self.record.as_str() {
                    "" => DnsRecord::default(),
                    record => DnsRecord::parse(record)
                        .ok_or_else(|| "choose the record type".to_string())?,
                },
                server: Some(text(&self.server)).filter(|s| !s.is_empty()),
                expect: comma_list(&self.expect),
            },
            "file" => CheckSpec::File {
                path: text(&self.path),
                pattern: text(&self.pattern),
                recursive: self.recursive,
                min_bytes: parse_min_mb(&self.min_mb)?,
            },
            "service" => CheckSpec::Service {
                name: text(&self.service),
            },
            "uptime" => CheckSpec::Uptime,
            _ => return Err("choose what to check".into()),
        };
        spec.validate().map_err(|e| e.to_string())?;
        Ok(spec)
    }
}

/// A count typed in the form: empty is the default (`None`).
fn optional_count(text: &str) -> Result<Option<u32>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse()
        .map(Some)
        .map_err(|_| format!("{text} isn't a whole number"))
}

/// When a check alerts, as typed: failed and good results in a row.
pub(crate) fn alert_after(fail_after: &str, recover_after: &str) -> Result<AlertAfter, String> {
    let alert_after = AlertAfter {
        fail_after: optional_count(fail_after)?,
        recover_after: optional_count(recover_after)?,
    };
    alert_after.validate()?;
    Ok(alert_after)
}

/// A check's own limits as typed, for its kind's metric (none for a kind
/// without thresholds, whatever was typed).
pub(crate) fn check_limits(kind: &str, warning: &str, down: &str) -> Result<MetricLimits, String> {
    match ThresholdMetric::of_kind(kind) {
        Some(metric) => MetricLimits::from_text(metric, warning, down),
        None => Ok(MetricLimits::default()),
    }
}

/// The request for a check called `name` with these fields, checked like
/// the hub checks it. `depends_on`: the ID of the check it is behind ("": none).
pub(crate) fn check_request(
    name: &str,
    fields: &CheckFields,
    enabled: bool,
    alert_after: AlertAfter,
    limits: MetricLimits,
    depends_on: &str,
) -> Result<CheckRequest, String> {
    common::checks::validate_name(name.trim()).map_err(|e| e.to_string())?;
    Ok(CheckRequest {
        name: name.trim().to_string(),
        spec: fields.to_spec()?,
        enabled,
        alert_after,
        limits,
        depends_on: Some(depends_on.trim().to_string()).filter(|d| !d.is_empty()),
    })
}

/// The request that enables a disabled `check`, or disables it: the whole
/// check with only `enabled` flipped. What a request leaves out is its
/// default, so sending less reset when it alerts, its limits and what it is
/// behind.
pub(crate) fn switched(check: &CheckInfo) -> CheckRequest {
    CheckRequest {
        name: check.name.clone(),
        spec: check.spec.clone(),
        enabled: !check.enabled,
        alert_after: check.alert_after,
        limits: check.limits,
        depends_on: check.depends_on.clone(),
    }
}

/// A count as the form shows it (empty: the default).
fn count_text(count: Option<u32>) -> String {
    count.map(|n| n.to_string()).unwrap_or_default()
}

/// Add a check to `probe_id` (`editing` = `None`) or change one. `checks`:
/// the probe's checks, to choose the one it is behind (D-68).
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))] // handlers are browser-only
pub(crate) fn CheckDialog(
    probe_id: String,
    editing: Option<CheckInfo>,
    checks: Vec<CheckInfo>,
    open: RwSignal<bool>,
    saved: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let title = if editing.is_some() {
        "Edit check"
    } else {
        "Add check"
    };
    let name = RwSignal::new(editing.as_ref().map(|c| c.name.clone()).unwrap_or_default());
    let enabled = RwSignal::new(editing.as_ref().is_none_or(|c| c.enabled));
    let current = editing.as_ref().map(|c| c.alert_after).unwrap_or_default();
    let fail_after = RwSignal::new(count_text(current.fail_after));
    let recover_after = RwSignal::new(count_text(current.recover_after));
    let (own_warning, own_down) = editing
        .as_ref()
        .map(|c| c.limits.texts())
        .unwrap_or_default();
    let warning_at = RwSignal::new(own_warning);
    let down_at = RwSignal::new(own_down);
    let depends_on = RwSignal::new(
        editing
            .as_ref()
            .and_then(|c| c.depends_on.clone())
            .unwrap_or_default(),
    );
    // What it can be behind: the probe's other checks (the hub refuses loops).
    let parents: Vec<(String, String)> = checks
        .into_iter()
        .filter(|c| editing.as_ref().is_none_or(|e| e.id != c.id))
        .map(|c| (c.id, c.name))
        .collect();
    // What applies when left empty: the levels above (D-64), if the hub said.
    let in_force = editing.as_ref().and_then(|c| c.limits_in_force);
    let inherited = move |pick: fn(&ResolvedLimits) -> Limit, metric: ThresholdMetric| {
        in_force
            .map(|l| pick(&l))
            .filter(|l| l.from != ThresholdLevel::Check)
            .map_or_else(|| "inherited".to_string(), |l| l.text(metric))
    };
    let fields = RwSignal::new(editing.as_ref().map_or_else(
        || CheckFields {
            kind: "ping".into(),
            ..CheckFields::default()
        },
        |c| CheckFields::from_spec(&c.spec),
    ));
    let error = RwSignal::new(String::new());
    let url = match &editing {
        Some(check) => format!("/api/checks/{}", check.id),
        None => format!("/api/probes/{probe_id}/checks"),
    };
    let url = StoredValue::new(url);

    let save = move || {
        let kind = fields.with_untracked(|f| f.kind.clone());
        let request = match alert_after(&fail_after.get_untracked(), &recover_after.get_untracked())
            .and_then(|alert_after| {
                let limits =
                    check_limits(&kind, &warning_at.get_untracked(), &down_at.get_untracked())?;
                check_request(
                    &name.get_untracked(),
                    &fields.get_untracked(),
                    enabled.get_untracked(),
                    alert_after,
                    limits,
                    &depends_on.get_untracked(),
                )
            }) {
            Ok(request) => request,
            Err(e) => {
                error.set(e);
                return;
            }
        };
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                let body = serde_json::to_string(&request).unwrap_or_default();
                match post_for::<CheckInfo>(&url.get_value(), &body).await {
                    Ok(_) => {
                        open.set(false);
                        saved();
                    }
                    Err(e) => error.set(e),
                }
            }
            #[cfg(feature = "ssr")]
            let _ = request;
        });
    };

    // One text input bound to a field of `fields`.
    let input = move |id: &'static str,
                      label: &'static str,
                      placeholder: &'static str,
                      get: fn(&CheckFields) -> String,
                      set: fn(&mut CheckFields, String)| {
        #[cfg(not(feature = "ssr"))]
        let on_input = move |ev| fields.update(|f| set(f, input_value(&ev)));
        #[cfg(feature = "ssr")]
        let on_input = move |_| {
            let _ = set;
        };
        view! {
            <div class="form-group">
                <label for=id>{label}</label>
                <input id=id name=id type="text" placeholder=placeholder
                    prop:value=move || fields.with(get) on:input=on_input/>
            </div>
        }
    };
    #[cfg(not(feature = "ssr"))]
    let on_depends = move |ev| depends_on.set(input_value(&ev));
    #[cfg(feature = "ssr")]
    let on_depends = |_| {};
    #[cfg(not(feature = "ssr"))]
    let on_record = move |ev| fields.update(|f| f.record = input_value(&ev));
    #[cfg(feature = "ssr")]
    let on_record = |_| {};
    #[cfg(not(feature = "ssr"))]
    let on_recursive = move |ev| fields.update(|f| f.recursive = event_target_checked(&ev));
    #[cfg(feature = "ssr")]
    let on_recursive = |_| {};
    #[cfg(not(feature = "ssr"))]
    let (on_unverified, on_no_redirects, on_method, on_headers) = (
        move |ev| fields.update(|f| f.unverified = event_target_checked(&ev)),
        move |ev| fields.update(|f| f.no_redirects = event_target_checked(&ev)),
        move |ev| fields.update(|f| f.method = input_value(&ev)),
        move |ev| fields.update(|f| f.headers = event_target_value(&ev)),
    );
    #[cfg(feature = "ssr")]
    let (on_unverified, on_no_redirects, on_method, on_headers) = (|_| {}, |_| {}, |_| {}, |_| {});
    // What not verifying means depends on the kind.
    let unverified_label = move || {
        if fields.with(|f| f.kind == "tls") {
            " Only watch the expiry date: accept a certificate that is self-signed, from a private authority, or for another name"
        } else {
            " Accept any certificate over HTTPS (self-signed, a private authority, expired, another name)"
        }
    };
    #[cfg(not(feature = "ssr"))]
    let (on_name, on_kind, on_enabled, on_fail, on_recover, on_warning, on_down) = (
        move |ev| name.set(input_value(&ev)),
        move |ev| fields.update(|f| f.kind = input_value(&ev)),
        move |ev| enabled.set(event_target_checked(&ev)),
        move |ev| fail_after.set(input_value(&ev)),
        move |ev| recover_after.set(input_value(&ev)),
        move |ev| warning_at.set(input_value(&ev)),
        move |ev| down_at.set(input_value(&ev)),
    );
    #[cfg(feature = "ssr")]
    let (on_name, on_kind, on_enabled, on_fail, on_recover, on_warning, on_down) =
        (|_| {}, |_| {}, |_| {}, |_| {}, |_| {}, |_| {}, |_| {});
    let metric = move || fields.with(|f| ThresholdMetric::of_kind(&f.kind));
    let default = AlertAfter::DEFAULT.to_string();
    let max = AlertAfter::MAX.to_string();
    let shown = move || fields.with(CheckFields::inputs);

    view! {
        <div class="modal-overlay" on:click=move |_| open.set(false)>
            <div class="modal check-dialog" on:click=|e| e.stop_propagation()>
                <h3>{title}</h3>
                <div class="form-group">
                    <label for="check_name">"Name"</label>
                    <input id="check_name" name="check_name" type="text" placeholder="e.g. Core switch"
                        prop:value=move || name.get() on:input=on_name/>
                </div>
                <div class="form-group">
                    <label for="check_kind">"What to check"</label>
                    <select id="check_kind" name="check_kind" prop:value=move || fields.with(|f| f.kind.clone()) on:change=on_kind>
                        {CHECK_KINDS.iter().map(|kind| view! {
                            <option value=*kind>{kind_label(kind)}</option>
                        }).collect_view()}
                    </select>
                </div>
                <Show when=move || shown().host>
                    {input("check_host", "Host (an IP address, or a name the probe's machine resolves)", "192.168.1.1, printer.lan", |f| f.host.clone(), |f, v| f.host = v)}
                </Show>
                <Show when=move || shown().ping>
                    {input("check_count", "Echoes each time (empty: 4)", "1-20", |f| f.count.clone(), |f, v| f.count = v)}
                    {input("check_tcp_port", "Without an answer, try this TCP port (a firewall that drops pings; empty: don't)", "e.g. 443", |f| f.tcp_port.clone(), |f, v| f.tcp_port = v)}
                </Show>
                <Show when=move || shown().port>
                    {input("check_port", "Port (a TLS certificate: empty is 443)", "e.g. 443", |f| f.port.clone(), |f, v| f.port = v)}
                </Show>
                <Show when=move || shown().url>
                    {input("check_url", "URL", "https://example.com", |f| f.url.clone(), |f, v| f.url = v)}
                </Show>
                <Show when=move || shown().http>
                    <div class="form-group">
                        <label for="check_method">"Method"</label>
                        <select id="check_method" name="check_method"
                            prop:value=move || fields.with(|f| if f.method.is_empty() { "get".to_string() } else { f.method.clone() })
                            on:change=on_method>
                            <option value="get">"GET (the whole page)"</option>
                            <option value="head">"HEAD (headers only)"</option>
                        </select>
                    </div>
                    {input("check_status", "Status codes that are fine (empty: 200-299)", "e.g. 200, 301-308, 401", |f| f.status.clone(), |f, v| f.status = v)}
                    {input("check_contains", "The page must contain", "e.g. Sign in", |f| f.contains.clone(), |f, v| f.contains = v)}
                    {input("check_absent", "The page must not contain", "e.g. Maintenance", |f| f.absent.clone(), |f, v| f.absent = v)}
                    <div class="form-group">
                        <label for="check_headers">"Headers to send (one a line; not for passwords or tokens: everyone who sees the check sees them)"</label>
                        <textarea id="check_headers" name="check_headers" rows="2" placeholder="Accept-Language: en"
                            prop:value=move || fields.with(|f| f.headers.clone()) on:input=on_headers></textarea>
                    </div>
                    <div class="form-group form-check">
                        <label>
                            <input type="checkbox" name="check_no_redirects"
                                prop:checked=move || fields.with(|f| f.no_redirects) on:change=on_no_redirects/>
                            " Don't follow redirects (the redirect is the answer: expect its 3xx status)"
                        </label>
                    </div>
                </Show>
                <Show when=move || shown().service>
                    {input("check_service", "Service on the probe's machine. Windows: its service name, not its display name (Spooler, not Print Spooler). Linux: the systemd unit", "Spooler, MSSQLSERVER, nginx, backup.timer", |f| f.service.clone(), |f, v| f.service = v)}
                </Show>
                <Show when=move || shown().dns>
                    {input("check_dns_name", "Name to look up", "intranet.example.com", |f| f.host.clone(), |f, v| f.host = v)}
                    <div class="form-group">
                        <label for="check_record">"Record"</label>
                        <select id="check_record" name="check_record"
                            prop:value=move || fields.with(|f| if f.record.is_empty() { "a".to_string() } else { f.record.clone() })
                            on:change=on_record>
                            {DnsRecord::ALL.into_iter().map(|record| view! {
                                <option value=record.as_str()>{record.label()}</option>
                            }).collect_view()}
                        </select>
                    </div>
                    {input("check_server", "DNS server to ask (an IP address; empty: the ones the probe's machine uses)", "e.g. 192.168.1.1", |f| f.server.clone(), |f, v| f.server = v)}
                    {input("check_expect", "Answers that are fine (separated by commas; empty: any answer)", "192.168.1.20, 192.168.1.21", |f| f.expect.clone(), |f, v| f.expect = v)}
                </Show>
                <Show when=move || shown().disks>
                    {input("check_ignore", "Disks to leave out (separated by commas; empty: none). Removable, read-only and network disks never count", "E:, /mnt/scratch, /mnt/*", |f| f.ignore.clone(), |f, v| f.ignore = v)}
                </Show>
                <Show when=move || shown().file>
                    {input("check_file_path", "File or folder on the probe's machine (a folder: its newest file counts)", "/var/backups, D:\\Backups, \\\\nas\\backups", |f| f.path.clone(), |f, v| f.path = v)}
                    {input("check_pattern", "In a folder, only files named (empty: every file)", "*.bak, backup-*.zip", |f| f.pattern.clone(), |f, v| f.pattern = v)}
                    {input("check_min_mb", "The newest file must be at least (MB; empty: any size)", "e.g. 500", |f| f.min_mb.clone(), |f, v| f.min_mb = v)}
                    <div class="form-group form-check">
                        <label>
                            <input type="checkbox" name="check_recursive"
                                prop:checked=move || fields.with(|f| f.recursive) on:change=on_recursive/>
                            " Look in its subfolders too"
                        </label>
                    </div>
                </Show>
                <Show when=move || shown().path>
                    {input("check_path", "Disk (empty: the system disk)", "/, /data, C:", |f| f.path.clone(), |f, v| f.path = v)}
                </Show>
                <Show when=move || shown().unverified>
                    <div class="form-group form-check">
                        <label>
                            <input type="checkbox" name="check_unverified"
                                prop:checked=move || fields.with(|f| f.unverified) on:change=on_unverified/>
                            {unverified_label}
                        </label>
                    </div>
                </Show>
                {move || metric().map(|metric| view! {
                    <div class="form-group check-limits">
                        <label>
                            {format!("{} warning at ", metric.label())}
                            <input type="text" name="check_warning_at" size="6"
                                placeholder=inherited(|l| l.warning, metric)
                                prop:value=move || warning_at.get() on:input=on_warning/>
                            ", down at "
                            <input type="text" name="check_down_at" size="6"
                                placeholder=inherited(|l| l.down, metric)
                                prop:value=move || down_at.get() on:input=on_down/>
                            {format!(
                                " ({}; empty: the site's, company's or hub's; \"off\": none)",
                                match metric {
                                    ThresholdMetric::Tls => "days left or fewer",
                                    ThresholdMetric::FileAge => "hours old or more",
                                    other => other.unit().trim(),
                                },
                            )}
                        </label>
                    </div>
                })}
                <div class="form-group alert-after">
                    <label>
                        "Alert after "
                        <input type="number" name="check_fail_after" min="1" max=max.clone()
                            placeholder=default.clone() prop:value=move || fail_after.get() on:input=on_fail/>
                        " failed results in a row; recovered after "
                        <input type="number" name="check_recover_after" min="1" max=max
                            placeholder=default prop:value=move || recover_after.get() on:input=on_recover/>
                        " good ones"
                    </label>
                </div>
                <div class="form-group">
                    <label for="check_depends_on">"Behind (while it is down, this check is Unknown and doesn't alert)"</label>
                    <select id="check_depends_on" name="check_depends_on"
                        prop:value=move || depends_on.get() on:change=on_depends>
                        <option value="">"Nothing"</option>
                        {parents.into_iter().map(|(id, name)| view! { <option value=id>{name}</option> }).collect_view()}
                    </select>
                </div>
                <div class="form-group form-check">
                    <label>
                        <input type="checkbox" name="check_enabled" prop:checked=move || enabled.get() on:change=on_enabled/>
                        " Enabled (sent to the probe)"
                    </label>
                </div>
                <div class="login-error">{error}</div>
                <div class="modal-actions">
                    <button class="btn btn-secondary" on:click=move |_| open.set(false)>"Cancel"</button>
                    <button class="btn btn-primary" on:click=move |_| save()>"Save"</button>
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(kind: &str) -> CheckFields {
        CheckFields {
            kind: kind.into(),
            ..CheckFields::default()
        }
    }

    #[test]
    fn every_kind_has_a_label_and_round_trips_through_the_form() {
        let specs = [
            CheckSpec::ping("10.0.0.1"),
            CheckSpec::Ping {
                host: "10.0.0.2".into(),
                count: 8,
                tcp_port: Some(3389),
            },
            CheckSpec::http("https://example.com"),
            CheckSpec::Http {
                url: "https://example.com/login".into(),
                options: HttpOptions {
                    method: HttpMethod::Head,
                    status: "200, 401".into(),
                    contains: String::new(),
                    absent: String::new(),
                    follow_redirects: false,
                    headers: vec![HttpHeader {
                        name: "Accept".into(),
                        value: "text/html".into(),
                    }],
                    verify: false,
                },
            },
            CheckSpec::Tcp {
                host: "db".into(),
                port: 5432,
            },
            CheckSpec::Tls {
                host: "example.com".into(),
                port: 8443,
                verify: false,
            },
            CheckSpec::Cpu,
            CheckSpec::Memory,
            CheckSpec::Disk {
                path: Some("/data".into()),
            },
            CheckSpec::Disk { path: None },
            CheckSpec::Disks { ignore: vec![] },
            CheckSpec::Disks {
                ignore: vec!["E:".into(), "/mnt/*".into()],
            },
            CheckSpec::Service {
                name: "MSSQL$SQLEXPRESS".into(),
            },
            CheckSpec::Dns {
                name: "example.com".into(),
                record: DnsRecord::A,
                server: None,
                expect: vec![],
            },
            CheckSpec::Dns {
                name: "example.com".into(),
                record: DnsRecord::Mx,
                server: Some("192.168.1.1".into()),
                expect: vec!["10 mail.example.com".into(), "20 backup.example.com".into()],
            },
            CheckSpec::File {
                path: "D:\\Backups".into(),
                pattern: "*.bak".into(),
                recursive: true,
                min_bytes: Some(524_288_000),
            },
            CheckSpec::File {
                path: "/srv/dump.sql".into(),
                pattern: String::new(),
                recursive: false,
                min_bytes: Some(512),
            },
            CheckSpec::Uptime,
        ];
        for spec in specs {
            assert_ne!(kind_label(spec.kind()), "Unknown");
            assert_eq!(CheckFields::from_spec(&spec).to_spec(), Ok(spec));
        }
    }

    #[test]
    fn a_kind_shows_only_its_inputs() {
        assert_eq!(
            fields("tcp").inputs(),
            Inputs {
                host: true,
                port: true,
                ..Inputs::default()
            }
        );
        assert_eq!(fields("cpu").inputs(), Inputs::default());
    }

    #[test]
    fn values_are_trimmed_and_checked_like_the_hub_checks_them() {
        let mut tcp = fields("tcp");
        tcp.host = " db ".into();
        tcp.port = "http".into();
        assert!(tcp.to_spec().unwrap_err().contains("port"));
        tcp.port = " 5432 ".into();
        assert_eq!(
            tcp.to_spec(),
            Ok(CheckSpec::Tcp {
                host: "db".into(),
                port: 5432
            })
        );

        // A DNS name: an A record from the machine's servers unless typed.
        let mut dns = fields("dns");
        dns.host = " intranet.example.com ".into();
        assert_eq!(
            dns.to_spec(),
            Ok(CheckSpec::Dns {
                name: "intranet.example.com".into(),
                record: DnsRecord::A,
                server: None,
                expect: vec![],
            })
        );
        dns.server = "dns.example.com".into();
        assert!(dns.to_spec().unwrap_err().contains("DNS server"));
        dns.server = "192.168.1.1".into();
        dns.record = "srv".into();
        assert!(dns.to_spec().unwrap_err().contains("record type"));

        // Every disk: the ones to leave out are a list.
        let mut disks = fields("disks");
        assert_eq!(disks.to_spec(), Ok(CheckSpec::Disks { ignore: vec![] }));
        disks.ignore = " E: ,, /mnt/* ".into();
        assert_eq!(
            disks.to_spec(),
            Ok(CheckSpec::Disks {
                ignore: vec!["E:".into(), "/mnt/*".into()]
            })
        );

        // A ping: four echoes and no TCP port unless typed.
        let mut ping = fields("ping");
        ping.host = "10.0.0.1".into();
        assert_eq!(ping.to_spec(), Ok(CheckSpec::ping("10.0.0.1")));
        ping.count = " 10 ".into();
        ping.tcp_port = "443".into();
        assert_eq!(
            ping.to_spec(),
            Ok(CheckSpec::Ping {
                host: "10.0.0.1".into(),
                count: 10,
                tcp_port: Some(443),
            })
        );
        ping.count = "50".into();
        assert!(ping.to_spec().unwrap_err().contains("1-20"));
        ping.count = "many".into();
        assert!(ping.to_spec().unwrap_err().contains("a number"));

        // A file: sizes are typed in MB.
        let mut file = fields("file");
        file.path = " /var/backups ".into();
        file.min_mb = "0.5".into();
        assert_eq!(
            file.to_spec(),
            Ok(CheckSpec::File {
                path: "/var/backups".into(),
                pattern: String::new(),
                recursive: false,
                min_bytes: Some(524_288),
            })
        );
        file.min_mb = "big".into();
        assert!(file.to_spec().unwrap_err().contains("number of MB"));
        file.min_mb.clear();
        file.path = "backups".into();
        assert!(file.to_spec().unwrap_err().contains("full one"));

        // A web page: a plain GET unless the form says more.
        let mut http = fields("http");
        http.url = " https://example.com ".into();
        assert_eq!(http.to_spec(), Ok(CheckSpec::http("https://example.com")));
        http.contains = "Sign in ".into();
        http.headers = "Accept-Language: en\n\n X-Site : branch 7 ".into();
        let Ok(CheckSpec::Http { options, .. }) = http.to_spec() else {
            panic!("{:?}", http.to_spec());
        };
        assert_eq!(options.contains, "Sign in ", "as typed");
        let sent: Vec<_> = options
            .headers
            .iter()
            .map(|h| (h.name.as_str(), h.value.as_str()))
            .collect();
        assert_eq!(sent, [("Accept-Language", "en"), ("X-Site", "branch 7")]);
        http.headers = "no colon".into();
        assert!(http.to_spec().unwrap_err().contains("Name: value"));
        http.headers.clear();
        http.status = "2xx".into();
        assert!(http.to_spec().unwrap_err().contains("status code"));
        http.status = common::checks::DEFAULT_HTTP_STATUS.into();
        http.method = "head".into();
        assert!(
            http.to_spec().unwrap_err().contains("HEAD"),
            "text needs a body"
        );

        // A certificate is on port 443 unless the form says otherwise, and verified.
        let mut tls = fields("tls");
        tls.host = "example.com".into();
        assert_eq!(
            tls.to_spec(),
            Ok(CheckSpec::Tls {
                host: "example.com".into(),
                port: 443,
                verify: true
            })
        );
        tls.port = "https".into();
        assert!(tls.to_spec().unwrap_err().contains("port"));
        let mut tcp = fields("tcp");
        tcp.host = "db".into();
        assert!(tcp.to_spec().is_err(), "a TCP check needs its port");

        let mut ping = fields("ping");
        ping.host = "router lan".into();
        assert!(ping.to_spec().unwrap_err().contains("IP address"));
        ping.host = "router.lan".into();
        assert_eq!(ping.to_spec(), Ok(CheckSpec::ping("router.lan")), "a name");
        assert!(fields("snmp").to_spec().is_err());
    }

    #[test]
    fn a_request_needs_a_name() {
        let cpu = fields("cpu");
        let none = MetricLimits::default();
        assert!(check_request("  ", &cpu, true, AlertAfter::default(), none, "").is_err());
        let request = check_request(" CPU ", &cpu, false, AlertAfter::default(), none, "").unwrap();
        assert_eq!((request.name.as_str(), request.enabled), ("CPU", false));
        assert_eq!(request.depends_on, None, "empty: behind nothing");
        let behind = check_request("CPU", &cpu, true, AlertAfter::default(), none, " gw ");
        assert_eq!(behind.unwrap().depends_on.as_deref(), Some("gw"));
    }

    /// Regression test: Disable and Enable sent only the name, spec and flag,
    /// so a check lost when it alerts and its own limits.
    #[test]
    fn switching_a_check_on_or_off_keeps_the_rest_of_it() {
        use crate::model::Setting;
        let check = CheckInfo {
            id: "c".into(),
            probe_id: "p".into(),
            name: "Disk".into(),
            spec: CheckSpec::Disk { path: None },
            target: String::new(),
            enabled: true,
            alert_after: AlertAfter {
                fail_after: Some(3),
                recover_after: None,
            },
            limits: MetricLimits {
                warning: Some(Setting::At(80.0)),
                down: None,
            },
            limits_in_force: None,
            depends_on: Some("gw".into()),
            created_at: String::new(),
            updated_at: String::new(),
            updated_by: None,
        };
        let request = switched(&check);
        assert!(!request.enabled);
        assert_eq!(request.alert_after, check.alert_after);
        assert_eq!(request.limits, check.limits);
        assert_eq!(request.depends_on.as_deref(), Some("gw"));
    }

    #[test]
    fn when_it_alerts_is_optional_and_checked() {
        assert_eq!(
            alert_after("", " "),
            Ok(AlertAfter::default()),
            "empty: the default"
        );
        assert_eq!(
            alert_after("3", ""),
            Ok(AlertAfter {
                fail_after: Some(3),
                recover_after: None,
            })
        );
        assert!(alert_after("0", "").is_err());
        assert!(alert_after("", "11").is_err());
        assert!(alert_after("two", "").unwrap_err().contains("whole number"));
    }

    #[test]
    fn a_checks_limits_are_typed_for_its_kind() {
        use crate::model::Setting;
        assert_eq!(
            check_limits("disk", "85", " off "),
            Ok(MetricLimits {
                warning: Some(Setting::At(85.0)),
                down: Some(Setting::Off),
            })
        );
        assert_eq!(check_limits("ping", "", ""), Ok(MetricLimits::default()));
        assert!(check_limits("cpu", "120", "").is_err(), "over 100 %");
        assert!(
            check_limits("disk", "95", "90")
                .unwrap_err()
                .contains("below")
        );
        assert!(check_limits("ping", "fast", "").is_err());
        assert!(check_limits("tls", "30", "14").is_ok(), "days left fall");
        assert!(check_limits("file", "26", "50").is_ok(), "hours");
        assert!(check_limits("file", "50", "26").is_err());
        assert_eq!(
            check_limits("service", "1", "2"),
            Ok(MetricLimits::default()),
            "running or not: nothing to limit"
        );
        assert!(
            check_limits("tls", "14", "30")
                .unwrap_err()
                .contains("above")
        );
        assert_eq!(
            check_limits("uptime", "5", "9"),
            Ok(MetricLimits::default()),
            "no thresholds for its kind"
        );
    }
}
