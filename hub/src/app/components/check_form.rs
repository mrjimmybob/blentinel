//! The add/edit dialog for a check (M2-05). The form shows the settings of
//! the chosen kind, and checks them with the same code as the hub and the
//! probe (`common::checks`) before anything is sent.

use common::checks::{CHECK_KINDS, CheckSpec};
use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::post_for;
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::model::{CheckInfo, CheckRequest};

/// A kind as the UI names it.
pub(crate) fn kind_label(kind: &str) -> &'static str {
    match kind {
        "ping" => "Ping",
        "http" => "Web page (HTTP)",
        "tcp" => "TCP port",
        "cpu" => "CPU usage",
        "memory" => "Memory usage",
        "disk" => "Disk usage",
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
}

/// Which inputs a kind needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Inputs {
    pub host: bool,
    pub port: bool,
    pub url: bool,
    pub path: bool,
}

impl CheckFields {
    pub fn from_spec(spec: &CheckSpec) -> Self {
        let mut fields = Self {
            kind: spec.kind().to_string(),
            ..Self::default()
        };
        match spec {
            CheckSpec::Ping { host } => fields.host = host.clone(),
            CheckSpec::Http { url } => fields.url = url.clone(),
            CheckSpec::Tcp { host, port } => {
                fields.host = host.clone();
                fields.port = port.to_string();
            }
            CheckSpec::Disk { path } => fields.path = path.clone().unwrap_or_default(),
            CheckSpec::Cpu | CheckSpec::Memory | CheckSpec::Uptime => {}
        }
        fields
    }

    pub fn inputs(&self) -> Inputs {
        match self.kind.as_str() {
            "ping" => Inputs {
                host: true,
                ..Inputs::default()
            },
            "tcp" => Inputs {
                host: true,
                port: true,
                ..Inputs::default()
            },
            "http" => Inputs {
                url: true,
                ..Inputs::default()
            },
            "disk" => Inputs {
                path: true,
                ..Inputs::default()
            },
            _ => Inputs::default(),
        }
    }

    /// The spec these fields describe, if it's valid (`CheckSpec::validate`).
    pub fn to_spec(&self) -> Result<CheckSpec, String> {
        let text = |value: &str| value.trim().to_string();
        let spec = match self.kind.as_str() {
            "ping" => CheckSpec::Ping {
                host: text(&self.host),
            },
            "http" => CheckSpec::Http {
                url: text(&self.url),
            },
            "tcp" => CheckSpec::Tcp {
                host: text(&self.host),
                port: self
                    .port
                    .trim()
                    .parse()
                    .map_err(|_| "the port must be 1-65535".to_string())?,
            },
            "cpu" => CheckSpec::Cpu,
            "memory" => CheckSpec::Memory,
            "disk" => CheckSpec::Disk {
                path: Some(text(&self.path)).filter(|p| !p.is_empty()),
            },
            "uptime" => CheckSpec::Uptime,
            _ => return Err("choose what to check".into()),
        };
        spec.validate().map_err(|e| e.to_string())?;
        Ok(spec)
    }
}

/// The request for a check called `name` with these fields, checked like
/// the hub checks it.
pub(crate) fn check_request(
    name: &str,
    fields: &CheckFields,
    enabled: bool,
) -> Result<CheckRequest, String> {
    common::checks::validate_name(name.trim()).map_err(|e| e.to_string())?;
    Ok(CheckRequest {
        name: name.trim().to_string(),
        spec: fields.to_spec()?,
        enabled,
    })
}

/// Add a check to `probe_id` (`editing` = `None`) or change one.
#[component]
#[cfg_attr(feature = "ssr", allow(unused_variables))] // handlers are browser-only
pub(crate) fn CheckDialog(
    probe_id: String,
    editing: Option<CheckInfo>,
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
        let request = match check_request(
            &name.get_untracked(),
            &fields.get_untracked(),
            enabled.get_untracked(),
        ) {
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
    let (on_name, on_kind, on_enabled) = (
        move |ev| name.set(input_value(&ev)),
        move |ev| fields.update(|f| f.kind = input_value(&ev)),
        move |ev| enabled.set(event_target_checked(&ev)),
    );
    #[cfg(feature = "ssr")]
    let (on_name, on_kind, on_enabled) = (|_| {}, |_| {}, |_| {});
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
                    {input("check_host", "Host", "IP address or name", |f| f.host.clone(), |f, v| f.host = v)}
                </Show>
                <Show when=move || shown().port>
                    {input("check_port", "Port", "e.g. 443", |f| f.port.clone(), |f, v| f.port = v)}
                </Show>
                <Show when=move || shown().url>
                    {input("check_url", "URL", "https://example.com", |f| f.url.clone(), |f, v| f.url = v)}
                </Show>
                <Show when=move || shown().path>
                    {input("check_path", "Disk (empty: the system disk)", "/, /data, C:", |f| f.path.clone(), |f, v| f.path = v)}
                </Show>
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
            CheckSpec::Ping {
                host: "10.0.0.1".into(),
            },
            CheckSpec::Http {
                url: "https://example.com".into(),
            },
            CheckSpec::Tcp {
                host: "db".into(),
                port: 5432,
            },
            CheckSpec::Cpu,
            CheckSpec::Memory,
            CheckSpec::Disk {
                path: Some("/data".into()),
            },
            CheckSpec::Disk { path: None },
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

        let mut ping = fields("ping");
        ping.host = "router.lan".into();
        assert!(ping.to_spec().unwrap_err().contains("IP address"));
        assert!(fields("snmp").to_spec().is_err());
    }

    #[test]
    fn a_request_needs_a_name() {
        let cpu = fields("cpu");
        assert!(check_request("  ", &cpu, true).is_err());
        let request = check_request(" CPU ", &cpu, false).unwrap();
        assert_eq!((request.name.as_str(), request.enabled), ("CPU", false));
    }
}
