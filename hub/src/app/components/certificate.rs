//! "HTTPS certificate" on the admin page (M2-14, D-62): the certificate the
//! hub serves, and getting it from Let's Encrypt or another ACME authority.

use leptos::prelude::*;

#[cfg(not(feature = "ssr"))]
use crate::app::http::{fetch_json, post_for, post_json};
#[cfg(not(feature = "ssr"))]
use crate::app::ui::input_value;
use crate::app::ui::{Me, any, nothing};
#[cfg(not(feature = "ssr"))]
use crate::model::CertificateSaved;
use crate::model::{
    AcmeAuthority, AcmeChallenge, AcmeSettings, CertificateInfo, CertificateOverview, Permission,
};

/// Where Let's Encrypt publishes its terms of service.
const LETS_ENCRYPT_TERMS: &str = "https://letsencrypt.org/repository/";

#[component]
pub(crate) fn CertificateSection() -> impl IntoView {
    let me = Me::current();
    let can_manage = Signal::derive(move || me.can(Permission::ManageData));
    let refresh = RwSignal::new(0u32);
    let error = RwSignal::new(String::new());
    let notice = RwSignal::new(String::new());
    let busy = RwSignal::new(false);

    let overview: LocalResource<Result<CertificateOverview, String>> =
        LocalResource::new(move || {
            refresh.track();
            async move {
                #[cfg(not(feature = "ssr"))]
                {
                    fetch_json::<CertificateOverview>("/api/admin/certificate").await
                }
                #[cfg(feature = "ssr")]
                {
                    Err(String::new())
                }
            }
        });

    // Both can take a while: the authority checks the hub's names first.
    let save = move |settings: AcmeSettings| {
        busy.set(true);
        error.set(String::new());
        notice.set(String::new());
        let body = serde_json::to_string(&settings).unwrap_or_default();
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            match post_for::<CertificateSaved>("/api/admin/certificate", &body).await {
                Ok(CertificateSaved { error: Some(e), .. }) => {
                    error.set(format!("Saved, but no certificate yet: {e}"))
                }
                Ok(_) => notice.set("Saved.".into()),
                Err(e) => error.set(e),
            }
            #[cfg(feature = "ssr")]
            let _ = body;
            busy.set(false);
            refresh.update(|n| *n += 1);
        });
    };
    let renew = move || {
        busy.set(true);
        error.set(String::new());
        notice.set(String::new());
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            match post_json("/api/admin/certificate/renew", "{}").await {
                Ok(()) => notice.set("A new certificate is in use.".into()),
                Err(e) => error.set(e),
            }
            busy.set(false);
            refresh.update(|n| *n += 1);
        });
    };

    view! {
        <div class="admin-section certificate">
            <h2>"HTTPS certificate"</h2>
            {move || -> AnyView {
                match overview.get() {
                    Some(Ok(data)) if !data.tls_enabled => any(view! {
                        <p class="info-text">
                            "The hub doesn't serve HTTPS itself ([server.tls] enabled = false): \
                             the proxy in front of it handles certificates."
                        </p>
                    }),
                    Some(Ok(data)) => any(view! {
                        <Status data=data.clone()/>
                        <Show when=move || can_manage.get()>
                            <AcmeForm data=data.clone() busy error on_save=save on_renew=renew/>
                        </Show>
                    }),
                    Some(Err(e)) => any(view! { <div class="error">{format!("Error: {e}")}</div> }),
                    None => any(view! { <div class="loading">"Loading…"</div> }),
                }
            }}
            {move || {
                let e = error.get();
                if e.is_empty() { nothing() } else { any(view! { <div class="error-message">{e}</div> }) }
            }}
            {move || {
                let n = notice.get();
                if n.is_empty() { nothing() } else { any(view! { <div class="success-message">{n}</div> }) }
            }}
        </div>
    }
}

/// A certificate in one line.
fn describe(cert: &CertificateInfo) -> String {
    let by = match cert.self_signed {
        true => "Self-signed".to_string(),
        false => format!("Issued by {}", cert.issuer),
    };
    format!(
        "{by}, for {}; valid until {}.",
        cert.names.join(", "),
        cert.not_after
    )
}

/// What the hub serves, and how getting it from the authority went.
#[component]
fn Status(data: CertificateOverview) -> impl IntoView {
    let current = data
        .certificate
        .as_ref()
        .map_or_else(|| "No certificate yet.".to_string(), describe);
    let next = data
        .next_certificate
        .as_ref()
        .map(|cert| format!(" After the key rotation: {}", describe(cert)));
    let renews = data
        .next_attempt_at
        .as_ref()
        .map(|at| format!(" The hub asks the authority again at {at}."));
    let failed = data
        .last_error
        .as_ref()
        .zip(data.last_attempt_at.as_ref())
        .map(|(error, at)| {
            view! {
                <div class="storage-warning-banner certificate-failed">
                    {format!("Asking for a certificate at {at} failed: {error}")}
                </div>
            }
        });
    view! {
        <p class="certificate-status">{current}{next}{renews}</p>
        {failed}
    }
}

/// The ACME settings, "Save" and "Renew now".
#[component]
fn AcmeForm(
    data: CertificateOverview,
    busy: RwSignal<bool>,
    error: RwSignal<String>,
    on_save: impl Fn(AcmeSettings) + Copy + Send + Sync + 'static,
    on_renew: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let saved = data.settings.clone();
    let enabled = RwSignal::new(saved.enabled);
    let domains = RwSignal::new(saved.domains.join(", "));
    let email = RwSignal::new(saved.email.clone());
    let authority = RwSignal::new(saved.authority);
    let directory_url = RwSignal::new(saved.directory_url.clone());
    let ca_pem = RwSignal::new(saved.ca_pem.clone());
    let challenge = RwSignal::new(saved.challenge);
    let terms = RwSignal::new(saved.terms_agreed);
    let https_port = data.https_port;
    let http_port = data.http_port;

    #[cfg(not(feature = "ssr"))]
    let (on_enabled, on_domains, on_email, on_authority, on_url, on_pem, on_challenge, on_terms) = (
        move |ev| enabled.set(event_target_checked(&ev)),
        move |ev| domains.set(input_value(&ev)),
        move |ev| email.set(input_value(&ev)),
        move |ev| authority.set(AcmeAuthority::parse(&event_target_value(&ev)).unwrap_or_default()),
        move |ev| directory_url.set(input_value(&ev)),
        move |ev| ca_pem.set(event_target_value(&ev)),
        move |ev| challenge.set(AcmeChallenge::parse(&event_target_value(&ev)).unwrap_or_default()),
        move |ev| terms.set(event_target_checked(&ev)),
    );
    #[cfg(feature = "ssr")]
    let (on_enabled, on_domains, on_email, on_authority, on_url, on_pem, on_challenge, on_terms) = (
        |_| {},
        |_| {},
        |_| {},
        |_| {},
        |_| {},
        |_| {},
        |_| {},
        |_| {},
    );

    let submit = move |_| {
        let settings = AcmeSettings {
            enabled: enabled.get(),
            domains: domains
                .get()
                .split(|c: char| c == ',' || c.is_whitespace())
                .map(str::to_string)
                .collect(),
            email: email.get(),
            authority: authority.get(),
            directory_url: directory_url.get(),
            ca_pem: ca_pem.get(),
            challenge: challenge.get(),
            terms_agreed: terms.get(),
        }
        .normalized();
        match settings.validate() {
            Ok(()) => on_save(settings),
            Err(e) => error.set(format!("Not saved: {e}.")),
        }
    };
    let port_hint = move || match challenge.get() {
        AcmeChallenge::TlsAlpn if https_port == 443 => {
            "The authority connects to port 443, where the hub serves HTTPS.".to_string()
        }
        AcmeChallenge::TlsAlpn => format!(
            "The authority connects to port 443: forward it to the hub's HTTPS port, {https_port}."
        ),
        AcmeChallenge::Http => match http_port {
            Some(80) => "The authority connects to port 80, the hub's plain-HTTP port.".into(),
            Some(port) => format!(
                "The authority connects to port 80: forward it to the hub's plain-HTTP port, {port}."
            ),
            None => "This hub has no plain-HTTP port: choose TLS-ALPN.".into(),
        },
    };
    let terms_link = move || match authority.get() {
        AcmeAuthority::Custom => None,
        _ => Some(view! {
            " (" <a href=LETS_ENCRYPT_TERMS target="_blank" rel="noopener">"Let's Encrypt's"</a> ")"
        }),
    };

    view! {
        <div class="acme-form">
            <label>
                <input type="checkbox" name="acme_enabled" prop:checked=move || enabled.get() on:change=on_enabled/>
                " Get the certificate from an ACME authority (Let's Encrypt), renewed automatically"
            </label>
            <label>
                "Names the hub is reached by "
                <input type="text" name="acme_domains" placeholder="hub.example.com"
                    prop:value=move || domains.get() on:input=on_domains/>
            </label>
            <label>
                "Contact e-mail (optional) "
                <input type="email" name="acme_email" prop:value=move || email.get() on:input=on_email/>
            </label>
            <label>
                "Authority "
                <select name="acme_authority" prop:value=move || authority.get().as_str() on:change=on_authority>
                    <option value=AcmeAuthority::LetsEncrypt.as_str()>"Let's Encrypt"</option>
                    <option value=AcmeAuthority::LetsEncryptStaging.as_str()>"Let's Encrypt staging (test certificates)"</option>
                    <option value=AcmeAuthority::Custom.as_str()>"Another ACME authority"</option>
                </select>
            </label>
            <Show when=move || authority.get() == AcmeAuthority::Custom>
                <label>
                    "Directory URL "
                    <input type="url" name="acme_directory" placeholder="https://ca.example.com/acme/directory"
                        prop:value=move || directory_url.get() on:input=on_url/>
                </label>
                <label>
                    "Its root certificate, if not a public one (PEM) "
                    <textarea name="acme_ca_pem" rows="3" prop:value=move || ca_pem.get() on:input=on_pem></textarea>
                </label>
            </Show>
            <label>
                "Validation "
                <select name="acme_challenge" prop:value=move || challenge.get().as_str() on:change=on_challenge>
                    <option value=AcmeChallenge::TlsAlpn.as_str()>"TLS-ALPN (port 443)"</option>
                    <option value=AcmeChallenge::Http.as_str()>"HTTP (port 80)"</option>
                </select>
            </label>
            <p class="info-text">{port_hint}</p>
            <label>
                <input type="checkbox" name="acme_terms" prop:checked=move || terms.get() on:change=on_terms/>
                " I accept the authority's terms of service" {terms_link}
            </label>
            <div class="acme-actions">
                <button class="btn btn-primary btn-sm" name="acme_save" disabled=busy on:click=submit>
                    {move || if busy.get() { "Working…" } else { "Save" }}
                </button>
                " "
                <Show when=move || saved.enabled>
                    <button class="btn btn-secondary btn-sm" name="acme_renew" disabled=busy
                        on:click=move |_| on_renew()>"Renew now"</button>
                </Show>
            </div>
            <p class="info-text">
                "The certificate is issued for the hub's own key, so probes notice nothing: no rotation, \
                 no enrolling again. Turned off, the hub goes back to a self-signed certificate with the same key."
            </p>
        </div>
    }
}
