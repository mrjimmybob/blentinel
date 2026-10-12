//! Check templates (M5-12, proposal 0006 §5, D-85): a named set of checks
//! added to a probe in one go.
//!
//! Two kinds. **Built-in** ones are code ([`builtin`]): they may ask for a
//! value (`{host}`) that is put into each check's name and settings.
//! **Saved** ones are records, made from a probe's checks as they are.
//! Applying a template copies its checks to the probe: they are then that
//! probe's own, and changing the template later changes nothing there.
//!
//! Templates are hub-wide, not a company's. A saved one shows what the probe
//! it came from checks, so saved templates are for users who see every
//! company: only they see, apply, save and delete them. A user limited to
//! some companies has the built-in ones.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use common::checks::{self, CheckSpec, Secrets, SnmpOid, secret};
use rand::RngCore;
use rand::rngs::OsRng;
use serde_json::{Value, json};

use crate::model::{
    ApplyTemplateRequest, CheckRequest, CompanyScope, NewTemplateRequest, TemplateCheck,
    TemplateInfo, TemplateParameter,
};
use crate::service::probes::{ProbeError, ProbeResult, scoped_record};
use crate::storage::{CheckRecord, ProbeRecord, Storage, TemplateRecord};

/// The IDs of built-in templates start so; saved ones are random hex.
pub const BUILTIN_PREFIX: &str = "builtin:";
const MAX_NAME_CHARS: usize = 80;
const MAX_DESCRIPTION_CHARS: usize = 300;
const MAX_VALUE_CHARS: usize = 300;

fn check(name: &str, spec: Value) -> TemplateCheck {
    TemplateCheck {
        name: name.to_string(),
        spec,
    }
}

fn parameter(key: &str, label: &str, example: &str) -> TemplateParameter {
    TemplateParameter {
        key: key.to_string(),
        label: label.to_string(),
        example: example.to_string(),
        secret: false,
    }
}

/// What every SNMP template asks for: the device, and the community that
/// opens it (a secret of the check, D-86).
fn device() -> Vec<TemplateParameter> {
    vec![
        parameter("host", "Address of the device", "192.168.1.50"),
        TemplateParameter {
            secret: true,
            ..parameter(secret::COMMUNITY, "SNMP community (v2c)", "public")
        },
    ]
}

/// An `snmp` check of the device `{host}`, its values written as the check
/// form shows them ([`common::snmp`]): a person can read and change them.
fn snmp(name: &str, lines: &[&str]) -> TemplateCheck {
    let oids: Vec<SnmpOid> = lines
        .iter()
        // A line that can't be read stays out (a test reads them all).
        .filter_map(|line| common::snmp::parse_value(line).ok())
        .collect();
    debug_assert_eq!(oids.len(), lines.len(), "{name}");
    check(
        name,
        json!({ "kind": "snmp", "host": "{host}", "oids": oids }),
    )
}

fn built_in(
    id: &str,
    name: &str,
    description: &str,
    parameters: Vec<TemplateParameter>,
    checks: Vec<TemplateCheck>,
) -> TemplateInfo {
    TemplateInfo {
        id: format!("{BUILTIN_PREFIX}{id}"),
        name: name.to_string(),
        description: description.to_string(),
        builtin: true,
        parameters,
        checks,
        created_by: None,
    }
}

/// The templates that come with the hub.
pub fn builtin() -> Vec<TemplateInfo> {
    vec![
        built_in(
            "machine",
            "Machine baseline",
            "What every server or workstation deserves: CPU, memory, every fixed disk, and its uptime.",
            vec![],
            vec![
                check("CPU", json!({ "kind": "cpu" })),
                check("Memory", json!({ "kind": "memory" })),
                check("Disks", json!({ "kind": "disks" })),
                check("Uptime", json!({ "kind": "uptime" })),
            ],
        ),
        built_in(
            "website",
            "Website",
            "A site as its visitors meet it: the page answers, its certificate is accepted and not about to expire, and its name resolves.",
            vec![parameter("host", "Host name", "www.example.com")],
            vec![
                check(
                    "{host} page",
                    json!({ "kind": "http", "url": "https://{host}" }),
                ),
                check(
                    "{host} certificate",
                    json!({ "kind": "tls", "host": "{host}", "port": 443 }),
                ),
                check("{host} DNS", json!({ "kind": "dns", "name": "{host}" })),
            ],
        ),
        built_in(
            "backup",
            "Backup freshness",
            "Did the last backup write a file? The newest file of a folder, judged by its age (warning after 26 hours, down after 50, unless you set others).",
            vec![parameter("path", "Backup folder", "D:\\Backups")],
            vec![check(
                "Backup in {path}",
                json!({ "kind": "file", "path": "{path}" }),
            )],
        ),
        built_in(
            "domain-controller",
            "Domain controller",
            "A Windows domain controller, for the probe installed on it: its directory, DNS, logon and Kerberos services, its LDAP port, and that the domain's name resolves.",
            vec![parameter("domain", "Domain name", "corp.example.com")],
            vec![
                check(
                    "Active Directory",
                    json!({ "kind": "service", "name": "NTDS" }),
                ),
                check("DNS server", json!({ "kind": "service", "name": "DNS" })),
                check("Netlogon", json!({ "kind": "service", "name": "Netlogon" })),
                check("Kerberos", json!({ "kind": "service", "name": "Kdc" })),
                check(
                    "LDAP port",
                    json!({ "kind": "tcp", "host": "127.0.0.1", "port": 389 }),
                ),
                check("{domain} DNS", json!({ "kind": "dns", "name": "{domain}" })),
            ],
        ),
        // SNMP devices (M5-09, D-87): the OIDs are those of the standard and
        // the makers' MIBs. Limits are a start: each is the check's to change.
        built_in(
            "snmp-printer",
            "Printer (SNMP)",
            "A network printer: what it is doing, how much each toner, drum or other supply has left (warning under 15 %, down under 3 %), the paper in its trays and the pages it has printed.",
            device(),
            vec![snmp(
                "{host} printer",
                &[
                    "Status = 1.3.6.1.2.1.25.3.5.1.1 ; walk ; means 1=other, 2=unknown, 3=idle, 4=printing, 5=warming up",
                    "Supply = 1.3.6.1.2.1.43.11.1.1.9 ; rows 1.3.6.1.2.1.43.11.1.1.6 ; of 1.3.6.1.2.1.43.11.1.1.8 ; warn < 15 ; down < 3",
                    "Paper = 1.3.6.1.2.1.43.8.2.1.10 ; rows 1.3.6.1.2.1.43.8.2.1.13 ; of 1.3.6.1.2.1.43.8.2.1.9",
                    "Pages = 1.3.6.1.2.1.43.10.2.1.4 ; walk",
                ],
            )],
        ),
        built_in(
            "snmp-switch",
            "Switch or router ports (SNMP)",
            "Every port of a switch, a router or a firewall: how many are up, and the errors each one counts per second (warning over 1). For a port that must be up, add a line to the check: Uplink = 1.3.6.1.2.1.2.2.1.8.<port number> = 1.",
            device(),
            vec![snmp(
                "{host} ports",
                &[
                    "Port = 1.3.6.1.2.1.2.2.1.8 ; rows 1.3.6.1.2.1.31.1.1.1.1 ; means 1=up, 2=down, 3=testing, 4=unknown, 5=dormant, 6=not present, 7=lower layer down",
                    "Errors in = 1.3.6.1.2.1.2.2.1.14 ; rows 1.3.6.1.2.1.31.1.1.1.1 ; per second ; unit /s ; warn > 1",
                    "Errors out = 1.3.6.1.2.1.2.2.1.20 ; rows 1.3.6.1.2.1.31.1.1.1.1 ; per second ; unit /s ; warn > 1",
                ],
            )],
        ),
        built_in(
            "snmp-synology",
            "Synology NAS (SNMP)",
            "A Synology DiskStation or RackStation: its system, power and fans, its temperature, every disk (state and temperature), every volume (not degraded, not crashed) and the space it has left (warning under 15 %, down under 5 %).",
            device(),
            vec![snmp(
                "{host} NAS",
                &[
                    "System = 1.3.6.1.4.1.6574.1.1.0 = normal ; means 1=normal, 2=failed",
                    "Power = 1.3.6.1.4.1.6574.1.3.0 = normal ; means 1=normal, 2=failed",
                    "System fan = 1.3.6.1.4.1.6574.1.4.1.0 = normal ; means 1=normal, 2=failed",
                    "CPU fan = 1.3.6.1.4.1.6574.1.4.2.0 = normal ; means 1=normal, 2=failed",
                    "Temperature = 1.3.6.1.4.1.6574.1.2.0 ; unit °C ; warn > 55 ; down > 65",
                    "Disk = 1.3.6.1.4.1.6574.2.1.1.5 ; rows 1.3.6.1.4.1.6574.2.1.1.2 ; means 1=normal, 2=initialized, 3=not initialized, 4=system partition failed, 5=crashed ; not system partition failed | crashed",
                    "Disk temperature = 1.3.6.1.4.1.6574.2.1.1.6 ; rows 1.3.6.1.4.1.6574.2.1.1.2 ; unit °C ; warn > 50 ; down > 60",
                    "RAID = 1.3.6.1.4.1.6574.3.1.1.3 ; rows 1.3.6.1.4.1.6574.3.1.1.2 ; means 1=normal, 2=repairing, 3=migrating, 4=expanding, 5=deleting, 6=creating, 7=syncing, 8=checking parity, 9=assembling, 10=canceling, 11=degraded, 12=crashed, 13=scrubbing ; not degraded | crashed",
                    "Free space = 1.3.6.1.4.1.6574.3.1.1.4 ; rows 1.3.6.1.4.1.6574.3.1.1.2 ; of 1.3.6.1.4.1.6574.3.1.1.5 ; warn < 15 ; down < 5",
                    "DSM update = 1.3.6.1.4.1.6574.1.5.4.0 ; means 1=available, 2=up to date, 3=connecting, 4=disconnected, 5=other",
                ],
            )],
        ),
        built_in(
            "snmp-qnap",
            "QNAP NAS (SNMP)",
            "A QNAP NAS (its NAS-MIB): system and CPU temperature, every disk (state, SMART and temperature), every volume (ready) and the space it has left (warning under 15 %, down under 5 %), and its fans.",
            device(),
            vec![snmp(
                "{host} NAS",
                &[
                    "System temperature = 1.3.6.1.4.1.24681.1.3.6.0 ; unit °C ; warn > 55 ; down > 65",
                    "CPU temperature = 1.3.6.1.4.1.24681.1.3.5.0 ; unit °C ; warn > 75 ; down > 90",
                    "Disk = 1.3.6.1.4.1.24681.1.3.11.1.4 ; rows 1.3.6.1.4.1.24681.1.3.11.1.2 ; means 0=ready, -4=unknown, -5=no disk, -6=invalid, -9=read/write error ; not invalid | read/write error",
                    "Disk SMART = 1.3.6.1.4.1.24681.1.3.11.1.7 ; rows 1.3.6.1.4.1.24681.1.3.11.1.2 ; not warning | abnormal",
                    "Disk temperature = 1.3.6.1.4.1.24681.1.3.11.1.3 ; rows 1.3.6.1.4.1.24681.1.3.11.1.2 ; unit °C ; warn > 50 ; down > 60",
                    "Volume = 1.3.6.1.4.1.24681.1.3.17.1.6 = Ready ; rows 1.3.6.1.4.1.24681.1.3.17.1.2",
                    "Free space = 1.3.6.1.4.1.24681.1.3.17.1.5 ; rows 1.3.6.1.4.1.24681.1.3.17.1.2 ; of 1.3.6.1.4.1.24681.1.3.17.1.4 ; warn < 15 ; down < 5",
                    "Fan = 1.3.6.1.4.1.24681.1.3.15.1.3 ; rows 1.3.6.1.4.1.24681.1.3.15.1.2 ; unit RPM",
                ],
            )],
        ),
        built_in(
            "snmp-apc",
            "APC UPS (SNMP)",
            "An APC UPS with a network card: its battery (state, charge, minutes it can carry the load, temperature, whether it must be replaced), whether it runs on battery, its load and the voltage it gets.",
            device(),
            vec![snmp(
                "{host} UPS",
                &[
                    "Battery = 1.3.6.1.4.1.318.1.1.1.2.1.1.0 = normal ; means 1=unknown, 2=normal, 3=low, 4=faulty",
                    "Charge = 1.3.6.1.4.1.318.1.1.1.2.2.1.0 ; unit % ; warn < 50 ; down < 20",
                    "Runtime = 1.3.6.1.4.1.318.1.1.1.2.2.3.0 ; / 60 ; unit min ; warn < 15 ; down < 5",
                    "Battery temperature = 1.3.6.1.4.1.318.1.1.1.2.2.2.0 ; unit °C ; warn > 40 ; down > 50",
                    "Replace battery = 1.3.6.1.4.1.318.1.1.1.2.2.4.0 = no ; means 1=no, 2=yes",
                    "Output = 1.3.6.1.4.1.318.1.1.1.4.1.1.0 ; means 1=unknown, 2=online, 3=on battery, 4=on smart boost, 5=timed sleeping, 6=software bypass, 7=off, 8=rebooting, 9=switched bypass, 10=hardware failure bypass, 11=sleeping until power returns, 12=on smart trim ; not on battery | off | hardware failure bypass",
                    "Load = 1.3.6.1.4.1.318.1.1.1.4.2.3.0 ; unit % ; warn > 80 ; down > 95",
                    "Input voltage = 1.3.6.1.4.1.318.1.1.1.3.2.1.0 ; unit V",
                ],
            )],
        ),
    ]
}

fn saved(record: TemplateRecord) -> TemplateInfo {
    TemplateInfo {
        id: record.id,
        name: record.name,
        description: record.description,
        builtin: false,
        parameters: vec![],
        checks: record.checks,
        created_by: record.created_by,
    }
}

/// The templates a user with `scope` has: the built-in ones, then (for one
/// who sees every company) the saved ones by name.
pub async fn list(store: &dyn Storage, scope: &CompanyScope) -> ProbeResult<Vec<TemplateInfo>> {
    let mut templates = builtin();
    if matches!(scope, CompanyScope::All) {
        templates.extend(store.templates().await?.into_iter().map(saved));
    }
    Ok(templates)
}

/// A template a user with `scope` has; a saved one is unknown to a user
/// limited to some companies.
async fn get(store: &dyn Storage, scope: &CompanyScope, id: &str) -> ProbeResult<TemplateInfo> {
    if id.starts_with(BUILTIN_PREFIX) {
        return builtin()
            .into_iter()
            .find(|t| t.id == id)
            .ok_or(ProbeError::NotFound);
    }
    if !matches!(scope, CompanyScope::All) {
        return Err(ProbeError::NotFound);
    }
    store
        .template(id)
        .await?
        .map(saved)
        .ok_or(ProbeError::NotFound)
}

/// `value` with every `{key}` replaced, in every text it holds.
fn fill(value: &Value, values: &BTreeMap<String, String>) -> Value {
    match value {
        Value::String(text) => Value::String(fill_text(text, values)),
        Value::Array(items) => Value::Array(items.iter().map(|v| fill(v, values)).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, v)| (key.clone(), fill(v, values)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn fill_text(text: &str, values: &BTreeMap<String, String>) -> String {
    values.iter().fold(text.to_string(), |text, (key, value)| {
        text.replace(&format!("{{{key}}}"), value)
    })
}

/// The checks of `template` with `values` for its parameters, each one a
/// check the hub accepts. `Err`: what is missing or wrong, for the person.
pub fn instantiate(
    template: &TemplateInfo,
    values: &BTreeMap<String, String>,
) -> Result<Vec<CheckRequest>, String> {
    let mut given = BTreeMap::new();
    // What was typed for a secret is kept apart: it goes to the checks that
    // need a secret of that name, never into a name or a setting.
    let mut secrets = BTreeMap::new();
    for parameter in &template.parameters {
        let typed = values.get(&parameter.key).map_or("", String::as_str);
        if parameter.secret {
            if typed.is_empty() {
                return Err(format!("{} is needed", parameter.label));
            }
            secrets.insert(parameter.key.clone(), typed.to_string());
            continue;
        }
        let value = typed.trim();
        if value.is_empty() {
            return Err(format!(
                "{} is needed (e.g. {})",
                parameter.label, parameter.example
            ));
        }
        if value.chars().count() > MAX_VALUE_CHARS {
            return Err(format!("{} is too long", parameter.label));
        }
        given.insert(parameter.key.clone(), value.to_string());
    }
    template
        .checks
        .iter()
        .map(|check| {
            let name = fill_text(&check.name, &given);
            let spec: CheckSpec = serde_json::from_value(fill(&check.spec, &given))
                .map_err(|e| format!("{name}: {e}"))?;
            checks::validate_name(&name)
                .and_then(|()| spec.validate())
                .map_err(|e| format!("{name}: {e}"))?;
            let own = Secrets(
                secrets
                    .iter()
                    .filter(|(key, _)| spec.secret_names().contains(&key.as_str()))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            );
            let mut request = CheckRequest::new(name, spec);
            if !own.is_empty() {
                request
                    .spec
                    .validate_secrets(&own)
                    .map_err(|e| format!("{}: {e}", request.name))?;
                request.secrets = Some(own);
            }
            Ok(request)
        })
        .collect()
}

fn hub_wide(scope: &CompanyScope) -> ProbeResult<()> {
    if matches!(scope, CompanyScope::All) {
        Ok(())
    } else {
        Err(ProbeError::Forbidden(
            "saved templates are shared by every company: they are for users who see all companies"
                .into(),
        ))
    }
}

/// Save the checks of a probe as a template: each check's name and what it
/// checks (not its thresholds, nor what it is behind).
pub async fn create_from_probe(
    store: &dyn Storage,
    scope: &CompanyScope,
    request: &NewTemplateRequest,
    by: &str,
    now: DateTime<Utc>,
) -> ProbeResult<TemplateInfo> {
    hub_wide(scope)?;
    let name = request.name.trim();
    let description = request.description.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(ProbeError::Invalid(format!(
            "a template needs a name of at most {MAX_NAME_CHARS} characters"
        )));
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(ProbeError::Invalid(format!(
            "the description takes at most {MAX_DESCRIPTION_CHARS} characters"
        )));
    }
    if builtin().iter().any(|t| t.name.eq_ignore_ascii_case(name)) {
        return Err(ProbeError::Exists(format!("a template named {name}")));
    }
    scoped_record(store, scope, &request.from_probe_id).await?;
    let checks: Vec<TemplateCheck> = store
        .probe_checks(&request.from_probe_id)
        .await?
        .into_iter()
        .map(|check| {
            Ok(TemplateCheck {
                name: check.name,
                spec: serde_json::to_value(&check.spec)?,
            })
        })
        .collect::<Result<_, serde_json::Error>>()
        .map_err(|e| ProbeError::Storage(e.into()))?;
    if checks.is_empty() {
        return Err(ProbeError::Invalid(
            "this probe has no checks to save".into(),
        ));
    }
    let mut id = [0u8; 8];
    OsRng.fill_bytes(&mut id);
    let record = TemplateRecord {
        id: hex::encode(id),
        name: name.to_string(),
        description: description.to_string(),
        checks,
        created_at: now,
        created_by: Some(by.to_string()),
    };
    if !store.create_template(&record).await? {
        return Err(ProbeError::Exists(format!("a template named {name}")));
    }
    Ok(saved(record))
}

/// Delete a saved template. Checks made from it stay.
pub async fn delete(
    store: &dyn Storage,
    scope: &CompanyScope,
    id: &str,
) -> ProbeResult<TemplateInfo> {
    hub_wide(scope)?;
    if id.starts_with(BUILTIN_PREFIX) {
        return Err(ProbeError::Invalid(
            "a built-in template can't be deleted".into(),
        ));
    }
    let template = get(store, scope, id).await?;
    store.delete_template(id).await?;
    Ok(template)
}

/// What applying a template did.
pub struct Applied {
    pub template: TemplateInfo,
    pub probe: ProbeRecord,
    pub added: Vec<CheckRecord>,
    /// Checks the probe already had (the same settings): left as they are.
    pub skipped: usize,
}

/// Add the checks of a template to a probe of one of the user's companies.
pub async fn apply(
    store: &dyn Storage,
    key: &crate::secrets::DataKey,
    scope: &CompanyScope,
    probe_id: &str,
    request: &ApplyTemplateRequest,
    by: &str,
    now: DateTime<Utc>,
) -> ProbeResult<Applied> {
    scoped_record(store, scope, probe_id).await?;
    let template = get(store, scope, &request.template_id).await?;
    let requests = instantiate(&template, &request.values).map_err(ProbeError::Invalid)?;
    let (probe, added, skipped) =
        crate::service::checks::add_checks(store, key, scope, probe_id, &requests, by, now).await?;
    Ok(Applied {
        template,
        probe,
        added,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snmp_templates() -> Vec<TemplateInfo> {
        builtin()
            .into_iter()
            .filter(|t| t.id.starts_with("builtin:snmp-"))
            .collect()
    }

    #[test]
    fn the_snmp_templates_ask_for_the_device_and_its_community() {
        let templates = snmp_templates();
        assert_eq!(templates.len(), 5);
        for template in &templates {
            let keys: Vec<(&str, bool)> = template
                .parameters
                .iter()
                .map(|p| (p.key.as_str(), p.secret))
                .collect();
            assert_eq!(
                keys,
                [("host", false), ("community", true)],
                "{}",
                template.name
            );
            // Every line of it is read, and reads back as it is written.
            for check in &template.checks {
                let spec: CheckSpec = serde_json::from_value(check.spec.clone()).unwrap();
                let CheckSpec::Snmp { oids, .. } = &spec else {
                    panic!("{}: an snmp check", template.name);
                };
                assert!(oids.len() >= 3, "{}", template.name);
                let text = common::snmp::values_text(oids);
                assert_eq!(&common::snmp::parse_values(&text).unwrap(), oids);
            }
        }

        let printer = &templates[0];
        let typed = |host: &str, community: &str| {
            BTreeMap::from([
                ("host".to_string(), host.to_string()),
                ("community".to_string(), community.to_string()),
            ])
        };
        let checks = instantiate(printer, &typed("10.0.0.9", " s3cret ")).unwrap();
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "10.0.0.9 printer");
        assert!(checks[0].enabled);
        // The community is the check's secret, as typed, and nowhere else.
        let secrets = checks[0].secrets.as_ref().unwrap();
        assert_eq!(secrets.get("community"), Some(" s3cret "));
        assert!(
            !serde_json::to_string(&checks[0].spec)
                .unwrap()
                .contains("s3cret")
        );
        assert!(!checks[0].name.contains("s3cret"));

        assert_eq!(
            instantiate(printer, &typed("10.0.0.9", "")).unwrap_err(),
            "SNMP community (v2c) is needed"
        );
        let long = "x".repeat(300);
        assert!(
            instantiate(printer, &typed("10.0.0.9", &long))
                .unwrap_err()
                .starts_with("10.0.0.9 printer: the community takes at most")
        );
    }

    fn examples(template: &TemplateInfo) -> BTreeMap<String, String> {
        template
            .parameters
            .iter()
            .map(|p| (p.key.clone(), p.example.clone()))
            .collect()
    }

    #[test]
    fn every_built_in_template_makes_checks_the_hub_accepts() {
        let templates = builtin();
        assert!(templates.len() >= 9);
        for template in &templates {
            assert!(template.id.starts_with(BUILTIN_PREFIX));
            assert!(!template.description.is_empty());
            let checks = instantiate(template, &examples(template))
                .unwrap_or_else(|e| panic!("{}: {e}", template.name));
            assert_eq!(checks.len(), template.checks.len());
            // No placeholder is left in a name or a setting.
            for check in &checks {
                let json = serde_json::to_string(&check.spec).unwrap();
                for parameter in &template.parameters {
                    let placeholder = format!("{{{}}}", parameter.key);
                    assert!(!check.name.contains(&placeholder), "{}", check.name);
                    assert!(!json.contains(&placeholder), "{json}");
                }
            }
        }
        // IDs and names are told apart.
        let mut ids: Vec<&str> = templates.iter().map(|t| t.id.as_str()).collect();
        ids.dedup();
        assert_eq!(ids.len(), templates.len());
    }

    #[test]
    fn a_templates_value_goes_into_names_and_settings() {
        let website = builtin().into_iter().find(|t| t.name == "Website").unwrap();
        let values = BTreeMap::from([("host".to_string(), " shop.example.com ".to_string())]);
        let checks = instantiate(&website, &values).unwrap();
        assert_eq!(checks[0].name, "shop.example.com page");
        assert_eq!(checks[0].spec, CheckSpec::http("https://shop.example.com"));
        assert!(
            matches!(&checks[1].spec, CheckSpec::Tls { host, port: 443, .. } if host == "shop.example.com")
        );
        assert!(checks.iter().all(|c| c.enabled && c.depends_on.is_none()));
    }

    #[test]
    fn a_missing_or_unusable_value_is_refused_with_the_reason() {
        let website = builtin().into_iter().find(|t| t.name == "Website").unwrap();
        let missing = instantiate(&website, &BTreeMap::new()).unwrap_err();
        assert_eq!(missing, "Host name is needed (e.g. www.example.com)");
        let blank = BTreeMap::from([("host".to_string(), "  ".to_string())]);
        assert!(instantiate(&website, &blank).is_err());
        // The check's own rules apply to what was typed.
        let wrong = BTreeMap::from([("host".to_string(), "not a host name".to_string())]);
        let error = instantiate(&website, &wrong).unwrap_err();
        assert!(error.starts_with("not a host name page: "), "{error}");
        // A value nobody asked for is ignored.
        let machine = &builtin()[0];
        let extra = BTreeMap::from([("host".to_string(), "x".to_string())]);
        assert_eq!(instantiate(machine, &extra).unwrap().len(), 4);
    }
}
