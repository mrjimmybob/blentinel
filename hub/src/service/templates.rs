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
use common::checks::{self, CheckSpec};
use rand::RngCore;
use rand::rngs::OsRng;
use serde_json::{Value, json};

use crate::model::{
    CheckRequest, CompanyScope, NewTemplateRequest, TemplateCheck, TemplateInfo, TemplateParameter,
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
    }
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
    for parameter in &template.parameters {
        let value = values
            .get(&parameter.key)
            .map(|v| v.trim())
            .unwrap_or_default();
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
            Ok(CheckRequest::new(name, spec))
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
    scope: &CompanyScope,
    probe_id: &str,
    template_id: &str,
    values: &BTreeMap<String, String>,
    by: &str,
    now: DateTime<Utc>,
) -> ProbeResult<Applied> {
    scoped_record(store, scope, probe_id).await?;
    let template = get(store, scope, template_id).await?;
    let requests = instantiate(&template, values).map_err(ProbeError::Invalid)?;
    let (probe, added, skipped) =
        crate::service::checks::add_checks(store, scope, probe_id, &requests, by, now).await?;
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
        assert!(templates.len() >= 4);
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
