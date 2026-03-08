use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{Outlet, ParentRoute, Route, Router, Routes},
    hooks::use_params_map,
    ParamSegment, StaticSegment,
};

// ===========================================================================
// Shared data types
// ===========================================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DashboardCompany {
    pub company_id: String,
    pub total_probes: i64,
    pub active_probes: i64,
    pub expired_probes: i64,
    pub devices_up: i64,
    pub devices_down: i64,
    pub last_report: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompanyProbe {
    pub probe_id: String,
    pub probe_name: String,
    pub hostname: Option<String>,
    pub site: Option<String>,
    pub status: String,
    pub last_seen_at: Option<String>,
    pub devices_up: i64,
    pub devices_down: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProbeDevice {
    pub name: String,
    pub resource_type: String,
    pub target: String,
    pub status: String,
    pub message: Option<String>,
    pub latency_ms: Option<i64>,
    pub metric_value: Option<f64>,
    pub metric_unit: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UptimeBucket {
    pub bucket: String,
    pub up_count: i64,
    pub down_count: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AdminProbe {
    pub probe_id: String,
    pub company_id: String,
    pub status: String,
    pub last_seen_at: Option<String>,
    pub first_seen_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ArchiveMetadata {
    pub id: i64,
    pub filename: String,
    pub created_at: String,
    pub cutoff_date: String,
    pub size_mb: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StorageInfo {
    pub current_db_size_mb: u64,
    pub warn_threshold_mb: u64,
    pub retention_days: u32,
    pub last_archive_date: Option<String>,
    pub last_archive_size_mb: Option<i64>,
    pub warning: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AlertSilence {
    pub id: i64,
    pub scope_type: String,
    pub scope_id: String,
    pub reason: String,
    pub created_at: String,
    pub expires_at: Option<String>,
}

// ===========================================================================
// Helpers
// ===========================================================================

/// Type-erase a view so different branches can be returned from the same closure.
fn any(v: impl IntoView) -> AnyView {
    v.into_any()
}

/// Severity rank for dashboard ordering: 0 = CRITICAL, 1 = DEGRADED, 2 = HEALTHY.
fn company_severity_rank(c: &DashboardCompany) -> u8 {
    if c.devices_down > 0 {
        0
    } else if c.expired_probes > 0 {
        1
    } else {
        2
    }
}

/// Extract the value from an input event
fn event_target_value(ev: &leptos::ev::Event) -> String {
    use leptos::wasm_bindgen::JsCast;
    let target = ev.target().expect("Event should have a target");
    let input = target.dyn_ref::<leptos::web_sys::HtmlInputElement>();
    let select = target.dyn_ref::<leptos::web_sys::HtmlSelectElement>();

    if let Some(input) = input {
        input.value()
    } else if let Some(select) = select {
        select.value()
    } else {
        String::new()
    }
}

#[cfg(not(feature = "ssr"))]
async fn fetch_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    use gloo_net::http::Request;
    let resp = Request::get(url)
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;
    if resp.status() == 401 {
        return Err("UNAUTHORIZED".to_string());
    }
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json::<T>()
        .await
        .map_err(|e| format!("JSON error: {}", e))
}

#[cfg(not(feature = "ssr"))]
async fn post_json(url: &str, body: &str) -> Result<u16, String> {
    use gloo_net::http::Request;
    let resp = Request::post(url)
        .header("Content-Type", "application/json")
        .body(body)
        .map_err(|e| format!("Body error: {}", e))?
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;
    Ok(resp.status())
}

#[cfg(not(feature = "ssr"))]
async fn post_text(url: &str, body: &str) -> Result<u16, String> {
    use gloo_net::http::Request;
    let resp = Request::post(url)
        .header("Content-Type", "text/plain")
        .body(body)
        .map_err(|e| format!("Body error: {}", e))?
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;
    Ok(resp.status())
}

// ===========================================================================
// shell()
// ===========================================================================

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en" data-theme="light">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone() />
                <HydrationScripts options/>
                <MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

// ===========================================================================
// App
// ===========================================================================

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/hub.css"/>
        <Title text="Blentinel Hub"/>

        <Router>
            <Routes fallback=|| view! { <div class="empty-state">"Page not found."</div> }>
                <Route path=StaticSegment("login") view=LoginPage/>
                <ParentRoute path=StaticSegment("") view=AuthLayout>
                    <Route path=StaticSegment("") view=DashboardPage/>
                    <Route path=StaticSegment("admin") view=AdminPage/>
                    <Route path=(StaticSegment("company"), ParamSegment("company_id")) view=CompanyDetailPage/>
                    <Route path=(StaticSegment("archive"), ParamSegment("archive_id")) view=ArchiveViewerPage/>
                </ParentRoute>
            </Routes>
        </Router>
    }
}

// ===========================================================================
// LoginPage
// ===========================================================================

#[component]
fn LoginPage() -> impl IntoView {
    let token = RwSignal::new(String::new());
    let error = RwSignal::new(String::new());
    let submitting = RwSignal::new(false);

    let do_login = move || {
        let t = token.get();
        if t.trim().is_empty() {
            error.set("Please enter your token.".to_string());
            return;
        }

        submitting.set(true);
        error.set(String::new());

        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                match post_text("/api/login", t.trim()).await {
                    Ok(200) => {
                        leptos::web_sys::window()
                            .unwrap()
                            .location()
                            .set_href("/")
                            .unwrap_or(());
                    }
                    Ok(_) => {
                        error.set("Invalid token. Please try again.".to_string());
                    }
                    Err(e) => {
                        error.set(format!("Error: {}", e));
                    }
                }
            }
            submitting.set(false);
        });
    };

    let handle_click = move |_: _| {
        do_login();
    };
    #[cfg(not(feature = "ssr"))]
    let on_input = move |ev| {
        token.set(event_target_value(&ev));
    };
    #[cfg(feature = "ssr")]
    let on_input = |_| {};
    view! {
        <div class="login-page">
            <div class="login-card">
                <h1>"Blentinel Hub"</h1>
                <p>"Enter your admin token to access the dashboard."</p>
                <input
                    type="text"
                    placeholder="Admin token"
                    prop:value=move || token.get()
                    on:input=on_input
                />
                <button type="button" class="btn btn-primary" disabled=submitting on:click=handle_click>
                    {move || if submitting.get() { "Logging in…" } else { "Log In" }}
                </button>
                <div class="login-error">{error}</div>
            </div>
        </div>
    }
}

// ===========================================================================
// AuthLayout
// ===========================================================================

#[component]
fn AuthLayout() -> impl IntoView {
    let companies: LocalResource<Result<Vec<DashboardCompany>, String>> =
        LocalResource::new(|| async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<Vec<DashboardCompany>>("/api/dashboard/companies").await
            }
            #[cfg(feature = "ssr")]
            {
                Ok(vec![])
            }
        });

    Effect::new(move |_| {
        if let Some(Err(ref e)) = companies.get() {
            if e == "UNAUTHORIZED" {
                #[cfg(not(feature = "ssr"))]
                {
                    leptos::web_sys::window()
                        .unwrap()
                        .location()
                        .set_href("/login")
                        .unwrap_or(());
                }
            }
        }
    });

    view! {
        <Header/>
        <div class="main-content">
            <Outlet/>
        </div>
    }
}

// ===========================================================================
// Header
// ===========================================================================

#[component]
fn Header() -> impl IntoView {
    let theme = RwSignal::new("light".to_string());

    // DB size resource - fetches on mount and can be manually refreshed
    let db_size_info: LocalResource<Result<StorageInfo, String>> =
        LocalResource::new(|| async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<StorageInfo>("/api/admin/storage-info").await
            }
            #[cfg(feature = "ssr")]
            {
                Ok(StorageInfo {
                    current_db_size_mb: 0,
                    warn_threshold_mb: 1000,
                    retention_days: 90,
                    last_archive_date: None,
                    last_archive_size_mb: None,
                    warning: false,
                })
            }
        });

    // On client mount: read persisted theme preference
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            let win = leptos::web_sys::window().unwrap();
            if let Ok(Some(storage)) = win.local_storage() {
                if let Ok(Some(saved)) = storage.get_item("blentinel_theme") {
                    theme.set(saved.clone());
                    leptos::web_sys::window()
                        .unwrap()
                        .document()
                        .unwrap()
                        .document_element()
                        .unwrap()
                        .set_attribute("data-theme", &saved)
                        .unwrap_or(());
                }
            }
        }
    });

    let toggle_theme = move |_: _| {
        let next = if theme.get() == "dark" {
            "light"
        } else {
            "dark"
        };
        theme.set(next.to_string());
        #[cfg(not(feature = "ssr"))]
        {
            let win = leptos::web_sys::window().unwrap();
            win.document()
                .unwrap()
                .document_element()
                .unwrap()
                .set_attribute("data-theme", next)
                .unwrap_or(());
            if let Ok(Some(storage)) = win.local_storage() {
                storage.set_item("blentinel_theme", next).unwrap_or(());
            }
        }
    };

    let handle_logout = move |_: _| {
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                let _ = post_text("/api/logout", "").await;
                leptos::web_sys::window()
                    .unwrap()
                    .location()
                    .set_href("/login")
                    .unwrap_or(());
            }
        });
    };

    view! {
        <header class="header">
            <div class="header-left">
                <img class="logo-img" src="/logo.png" alt="Blentinel"
                     onerror="this.style.display='none'"/>
                <span class="logo-text">"Blentinel"</span>
                <ul class="nav-links">
                    <li><a href="/">"Dashboard"</a></li>
                    <li><a href="/admin">"Admin"</a></li>
                </ul>
            </div>
            <div class="header-right">
                // DB size indicator
                {move || -> AnyView {
                    match db_size_info.get() {
                        Some(Ok(info)) => {
                            let size_class = if info.warning { "db-size-warning" } else { "db-size-normal" };
                            let tooltip = if info.warning {
                                "Database exceeds retention threshold"
                            } else {
                                "Current database size"
                            };
                            any(view! {
                                <span class={format!("db-size-indicator {}", size_class)} title=tooltip>
                                    {format!("DB: {} MB", info.current_db_size_mb)}
                                </span>
                            })
                        }
                        Some(Err(_)) => any(view! { <></> }),
                        None => any(view! { <></> }),
                    }
                }}
                <button class="btn btn-ghost" on:click=toggle_theme>
                    {move || if theme.get() == "dark" { "☀️ Light" } else { "🌙 Dark" }}
                </button>
                <button class="btn btn-ghost" on:click=handle_logout>"Logout"</button>
            </div>
        </header>
    }
}

// ===========================================================================
// DashboardPage
// ===========================================================================

#[component]
fn DashboardPage() -> impl IntoView {
    let companies: LocalResource<Result<Vec<DashboardCompany>, String>> =
        LocalResource::new(|| async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<Vec<DashboardCompany>>("/api/dashboard/companies").await
            }
            #[cfg(feature = "ssr")]
            {
                Ok(vec![])
            }
        });

    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            use leptos::wasm_bindgen::closure::Closure;
            use leptos::wasm_bindgen::JsCast;
            let cb = Closure::wrap(Box::new(move || {
                companies.refetch();
            }) as Box<dyn FnMut()>);
            let func = cb
                .as_ref()
                .unchecked_ref::<leptos::web_sys::js_sys::Function>();
            leptos::web_sys::window()
                .unwrap()
                .set_interval_with_callback_and_timeout_and_arguments_0(func, 15_000)
                .unwrap();
            cb.forget();
        }
    });

    view! {
        <Suspense fallback=|| view! { <div class="loading">"Loading…"</div> }>
            {move || -> AnyView {
                let data = companies.get();
                let (companies_count, probes, active, up, down) = match &data {
                    Some(Ok(list)) => {
                        let cc = list.len() as i64;
                        let p: i64 = list.iter().map(|c| c.total_probes).sum();
                        let a: i64 = list.iter().map(|c| c.active_probes).sum();
                        let u: i64 = list.iter().map(|c| c.devices_up).sum();
                        let d: i64 = list.iter().map(|c| c.devices_down).sum();
                        (cc, p, a, u, d)
                    }
                    _ => (0, 0, 0, 0, 0),
                };

                let cards: AnyView = match data {
                    Some(Ok(list)) if !list.is_empty() => {
                        let mut sorted = list;
                        sorted.sort_by(|a, b| {
                            let sev = company_severity_rank(a).cmp(&company_severity_rank(b));
                            if sev != std::cmp::Ordering::Equal { return sev; }
                            let recency = b.last_report.cmp(&a.last_report);
                            if recency != std::cmp::Ordering::Equal { return recency; }
                            a.company_id.cmp(&b.company_id)
                        });
                        any(view! {
                            <div class="company-grid">
                                <For
                                    each=move || sorted.clone()
                                    key=|c: &DashboardCompany| c.company_id.clone()
                                    children=|company: DashboardCompany| view! { <CompanyCard company=company/> }
                                />
                            </div>
                        })
                    },
                    Some(Ok(_)) => any(view! {
                        <div class="empty-state">"No companies found. Deploy a probe to get started."</div>
                    }),
                    Some(Err(ref e)) if e == "UNAUTHORIZED" => any(view! { <div/> }),
                    Some(Err(e)) => any(view! {
                        <div class="error">{format!("Error: {}", e)}</div>
                    }),
                    None => any(view! {
                        <div class="loading">"Loading…"</div>
                    }),
                };

                any(view! {
                    <>
                        <div class="summary-stats">
                            <div class="stat-card">
                                <div class="stat-label">"Companies"</div>
                                <div class="stat-value">{companies_count}</div>
                            </div>
                            <div class="stat-card">
                                <div class="stat-label">"Probes"</div>
                                <div class="stat-value">{probes}</div>
                            </div>
                            <div class="stat-card">
                                <div class="stat-label">"Active"</div>
                                <div class="stat-value" style="color: var(--color-up)">{format!("{}/{}", active, probes)}</div>
                            </div>
                            <div class="stat-card">
                                <div class="stat-label">"Devices Up"</div>
                                <div class="stat-value" style="color: var(--color-up)">{up}</div>
                            </div>
                            <div class="stat-card">
                                <div class="stat-label">"Devices Down"</div>
                                <div class="stat-value" style="color: var(--color-down)">{down}</div>
                            </div>
                        </div>
                        {cards}
                    </>
                })
            }}
        </Suspense>
    }
}

// ===========================================================================
// CompanyCard
// ===========================================================================

/// Format an ISO 8601 timestamp string (e.g. "2026-03-08T01:27:26.522816772Z")
/// into "YYYY-MM-DD hh:mm:ss.sss" for display.
fn fmt_ts(ts: &str) -> String {
    let s = ts.trim_end_matches('Z');
    if let Some((date, time)) = s.split_once('T') {
        let time_fmt = if let Some((secs, frac)) = time.split_once('.') {
            let ms: String = frac.chars().take(3).collect();
            format!("{}.{}", secs, ms)
        } else {
            time.to_string()
        };
        format!("{} {}", date, time_fmt)
    } else {
        ts.to_string()
    }
}

#[component]
fn CompanyCard(company: DashboardCompany) -> impl IntoView {
    // Expired overrides device counts — stale data cannot indicate CRITICAL.
    let (border_class, dot_class) = if company.expired_probes > 0 {
        ("card-border-amber", "status-dot degraded")
    } else if company.devices_down > 0 {
        ("card-border-red", "status-dot critical")
    } else {
        ("card-border-green", "status-dot healthy")
    };

    let href = format!("/company/{}", company.company_id);
    let last_report_display = company
        .last_report
        .as_deref()
        .map(fmt_ts)
        .unwrap_or_else(|| "Never".to_string());

    view! {
        <a href=href class=format!("company-card {}", border_class)>
            <div class="card-title">{company.company_id.clone()}</div>
            <div class="card-stats">
                <span class="cs-label">"Active"</span>
                <span class="cs-value cs-up">{company.active_probes}</span>
                <span class="cs-label">"Expired"</span>
                <span class=format!("cs-value {}", if company.expired_probes > 0 { "cs-down" } else { "" })>{company.expired_probes}</span>
                <span class="cs-label">"Devices Up"</span>
                <span class="cs-value cs-up">{company.devices_up}</span>
                <span class="cs-label">"Devices Down"</span>
                <span class="cs-value cs-down">{company.devices_down}</span>
            </div>
            <div class="card-footer">
                <span class=dot_class></span>
                {"Last report: "}{last_report_display}
            </div>
        </a>
    }
}

// ===========================================================================
// CompanyDetailPage
// ===========================================================================

#[component]
fn CompanyDetailPage() -> impl IntoView {
    let params = use_params_map();
    let company_id = Signal::derive(move || params.get().get("company_id").unwrap_or_default());

    // Range selector state — defaults to "24h"
    let selected_range = RwSignal::new("24h".to_string());

    // ---------------------------------------------------------------------------
    // Silence modal state — lifted here so the modal renders at page root,
    // outside all <table> elements.  Signals are passed down to ProbeRow.
    // ---------------------------------------------------------------------------
    let silence_modal_open = RwSignal::new(false);
    let silence_device = RwSignal::new(None::<ProbeDevice>);
    let silence_company_id = RwSignal::new(String::new());
    let silence_probe_id = RwSignal::new(String::new());
    let silence_reason = RwSignal::new(String::new());
    let silence_duration = RwSignal::new(String::from("24"));

    // Shared silences signal — all probe rows read from this
    let silences = RwSignal::new(Vec::<AlertSilence>::new());

    // Fetch active silences once on mount
    Effect::new(move |_| {
        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                if let Ok(sils) = fetch_json::<Vec<AlertSilence>>("/api/silences").await {
                    silences.set(sils);
                }
            }
        });
    });

    let probes: LocalResource<Result<Vec<CompanyProbe>, String>> = LocalResource::new(move || {
        let cid = company_id.get();
        async move {
            if cid.is_empty() {
                return Ok(vec![]);
            }
            #[cfg(not(feature = "ssr"))]
            {
                let url = format!("/api/company/{}/probes", cid);
                fetch_json::<Vec<CompanyProbe>>(&url).await
            }
            #[cfg(feature = "ssr")]
            {
                Ok(vec![])
            }
        }
    });

    // Uptime resource reacts to selected_range — Leptos re-runs when the signal changes
    let uptime: LocalResource<Result<Vec<UptimeBucket>, String>> = LocalResource::new(move || {
        let cid = company_id.get();

        async move {
            if cid.is_empty() {
                return Ok(vec![]);
            }

            #[cfg(not(feature = "ssr"))]
            {
                let range = selected_range.get();
                let url = format!("/api/company/{}/uptime?range={}", cid, range);
                fetch_json::<Vec<UptimeBucket>>(&url).await
            }

            #[cfg(feature = "ssr")]
            {
                Ok(vec![])
            }
        }
    });

    // Poll every 15 seconds so probe status and uptime update live.
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            use leptos::wasm_bindgen::closure::Closure;
            use leptos::wasm_bindgen::JsCast;
            let cb = Closure::wrap(Box::new(move || {
                probes.refetch();
                uptime.refetch();
            }) as Box<dyn FnMut()>);
            let func = cb
                .as_ref()
                .unchecked_ref::<leptos::web_sys::js_sys::Function>();
            leptos::web_sys::window()
                .unwrap()
                .set_interval_with_callback_and_timeout_and_arguments_0(func, 15_000)
                .unwrap();
            cb.forget();
        }
    });

    // Dynamic title based on selected range
    let chart_title = Signal::derive(move || match selected_range.get().as_str() {
        "7d" => "Uptime \u{2014} Last 7 Days".to_string(),
        "30d" => "Uptime \u{2014} Last 30 Days".to_string(),
        "all" => "Uptime \u{2014} All Data".to_string(),
        _ => "Uptime \u{2014} Last 24 Hours".to_string(),
    });

    view! {
        <div class="breadcrumb">
            <a href="/">"Dashboard"</a>
            <span class="sep">"›"</span>
            {move || company_id.get()}
        </div>

        <Suspense fallback=|| view! { <div class="loading">"Loading…"</div> }>
            {move || -> AnyView {
                match uptime.get() {
                    Some(Ok(buckets)) => any(view! {
                        <UptimeChart buckets=buckets title=chart_title selected_range=selected_range/>
                    }),
                    _ => any(view! { <div/> }),
                }
            }}

            <div class="section-header">
                <h2>"Probes"</h2>
            </div>

            {move || -> AnyView {
                match probes.get() {
                    Some(Ok(list)) if !list.is_empty() => {
                        let probe_count = list.len();
                        any(view! {
                            <div class="probe-table-wrap">
                                <table class="probe-table">
                                    <thead>
                                        <tr>
                                            <th>"Probe"</th>
                                            <th>"Hostname"</th>
                                            <th>"Site"</th>
                                            <th>"Status"</th>
                                            <th>"Up"</th>
                                            <th>"Down"</th>
                                            <th>"Last Seen"</th>
                                            <th></th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        <For
                                            each=move || list.clone()
                                            key=|p: &CompanyProbe| p.probe_id.clone()
                                            children=move |probe: CompanyProbe| {
                                                let cid = company_id.get();
                                                view! {
                                                    <ProbeRow
                                                        probe=probe
                                                        company_id=cid
                                                        probe_count=probe_count
                                                        silences=silences
                                                        silence_modal_open=silence_modal_open
                                                        silence_device=silence_device
                                                        silence_company_id=silence_company_id
                                                        silence_probe_id=silence_probe_id
                                                    />
                                                }
                                            }
                                        />
                                    </tbody>
                                </table>
                            </div>
                        })
                    },
                    Some(Ok(_)) => any(view! {
                        <div class="empty-state">"No probes found for this company."</div>
                    }),
                    Some(Err(e)) => any(view! {
                        <div class="error">{format!("Error: {}", e)}</div>
                    }),
                    None => any(view! {
                        <div class="loading">"Loading…"</div>
                    }),
                }
            }}
        </Suspense>

        // -------------------------------------------------------------------
        // Silence modal — rendered at page root, outside all tables.
        // Uses the existing .modal-overlay / .modal CSS classes.
        // -------------------------------------------------------------------
        <Show when=move || silence_modal_open.get()>
            {move || {
                let dev = silence_device.get();
                let dev_name = dev.as_ref().map(|d| d.name.clone()).unwrap_or_default();

                view! {
                    <div class="modal-overlay" on:click=move |_| silence_modal_open.set(false)>
                        <div class="modal" on:click=|e| e.stop_propagation()>
                            <h3>"Silence Alerts"</h3>
                            <p>"Device: " <strong>{dev_name}</strong></p>

                            <div class="form-group">
                                <label>"Reason:"</label>
                                <input
                                    type="text"
                                    placeholder="e.g., Planned maintenance"
                                    prop:value=silence_reason
                                    on:input=move |ev| {
                                        silence_reason.set(event_target_value(&ev));
                                    }
                                />
                            </div>

                            <div class="form-group">
                                <label>"Duration:"</label>
                                <select
                                    prop:value=silence_duration
                                    on:change=move |ev| {
                                        silence_duration.set(event_target_value(&ev));
                                    }
                                >
                                    <option value="1">"1 hour"</option>
                                    <option value="24" selected>"24 hours"</option>
                                    <option value="168">"7 days"</option>
                                    <option value="forever">"Forever"</option>
                                </select>
                            </div>

                            <div class="modal-actions">
                                <button class="btn btn-secondary" on:click=move |_| silence_modal_open.set(false)>
                                    "Cancel"
                                </button>
                                <button class="btn btn-primary" on:click=move |_| {
                                    // Validate reason before submitting
                                    if silence_reason.get().trim().is_empty() {
                                        return;
                                    }

                                    leptos::task::spawn_local(async move {
                                        #[cfg(not(feature = "ssr"))]
                                        {
                                            let Some(dev) = silence_device.get() else { return; };
                                            let cid = silence_company_id.get();
                                            let pid = silence_probe_id.get();
                                            let reason = silence_reason.get();

                                            let resource_key = format!("{}:{}:{}:{}",
                                                cid, pid, dev.name, dev.target
                                            );

                                            let duration_str = silence_duration.get();
                                            let duration_hours: Option<u32> = if duration_str == "forever" {
                                                None
                                            } else {
                                                duration_str.parse().ok()
                                            };

                                            let body = serde_json::json!({
                                                "resource_key": resource_key,
                                                "reason": reason,
                                                "duration_hours": duration_hours
                                            });

                                            match post_json("/api/silence", &body.to_string()).await {
                                                Ok(200) => {
                                                    // Success: close modal, update silences locally
                                                    silence_modal_open.set(false);
                                                    // Push a new AlertSilence into the local signal
                                                    // (we don't know the server-assigned ID, so use 0 as placeholder;
                                                    //  the next page load will fetch the real list)
                                                    silences.update(|list| {
                                                        list.push(AlertSilence {
                                                            id: 0,
                                                            scope_type: "resource".to_string(),
                                                            scope_id: resource_key.clone(),
                                                            reason: reason.clone(),
                                                            created_at: String::new(),
                                                            expires_at: None,
                                                        });
                                                    });
                                                }
                                                Ok(status) => {
                                                    // Keep modal open on failure
                                                    eprintln!("Silence failed with status {}", status);
                                                }
                                                Err(e) => {
                                                    // Keep modal open on error
                                                    eprintln!("Silence error: {}", e);
                                                }
                                            }
                                        }
                                    });
                                }>
                                    "Silence"
                                </button>
                            </div>
                        </div>
                    </div>
                }
            }}
        </Show>
    }
}

// ===========================================================================
// ProbeRow
// ===========================================================================

#[component]
fn ProbeRow(
    probe: CompanyProbe,
    company_id: String,
    probe_count: usize,
    silences: RwSignal<Vec<AlertSilence>>,
    silence_modal_open: RwSignal<bool>,
    silence_device: RwSignal<Option<ProbeDevice>>,
    silence_company_id: RwSignal<String>,
    silence_probe_id: RwSignal<String>,
) -> impl IntoView {
    let is_single = probe_count == 1;
    let expanded = RwSignal::new(is_single);
    let cached_devices = RwSignal::new(Vec::<ProbeDevice>::new());
    let fetched = RwSignal::new(false);

    let probe_id_for_fetch = probe.probe_id.clone();
    let company_id_for_view = company_id.clone();
    let probe_id_for_view = probe.probe_id.clone();

    Effect::new(move |_| {
        if expanded.get() && !fetched.get() {
            let _pid = probe_id_for_fetch.clone();
            leptos::task::spawn_local(async move {
                #[cfg(not(feature = "ssr"))]
                {
                    let url = format!("/api/probe/{}/devices", _pid);
                    if let Ok(devs) = fetch_json::<Vec<ProbeDevice>>(&url).await {
                        cached_devices.set(devs);
                    }
                }
                fetched.set(true);
            });
        }
    });

    let toggle = move |_: _| {
        expanded.update(|e| *e = !*e);
    };

    // 3-tier severity: expired overrides device counts (stale data).
    // DEGRADED = probe not reporting; CRITICAL = active probe with devices down.
    let is_expired = probe.status != "active";
    let (severity_label, severity_class, dot_class) = if is_expired {
        ("DEGRADED", "severity-degraded", "status-dot degraded")
    } else if probe.devices_down > 0 {
        ("CRITICAL", "severity-critical", "status-dot critical")
    } else {
        ("HEALTHY", "severity-healthy", "status-dot healthy")
    };

    let display_name = if probe.probe_name.is_empty() {
        format!("{}…", &probe.probe_id[..8.min(probe.probe_id.len())])
    } else {
        probe.probe_name.clone()
    };
    let hostname_display = probe.hostname.clone().unwrap_or_else(|| "—".to_string());
    let site_display = probe.site.clone().unwrap_or_else(|| "—".to_string());
    let last_seen = probe
        .last_seen_at
        .as_deref()
        .map(fmt_ts)
        .unwrap_or_else(|| "Unknown".to_string());
    let probe_up = probe.devices_up;
    let probe_down = probe.devices_down;

    view! {
        <tr class="probe-summary-row">
            <td>{display_name}</td>
            <td>{hostname_display}</td>
            <td>{site_display}</td>
            <td><span class=format!("status-badge {}", severity_class)>{severity_label}</span></td>
            <td class={if is_expired { "stale-count" } else { "" }}>{probe_up}</td>
            <td class={if is_expired { "stale-count" } else { "" }}>{probe_down}</td>
            <td><span class=dot_class></span>{last_seen}</td>
            <td>
                {if !is_single {
                    any(view! {
                        <button class="expand-btn" on:click=toggle>
                            {move || if expanded.get() { "▲ Hide" } else { "▼ Show" }}
                        </button>
                    })
                } else {
                    any(view! { <></> })
                }}
            </td>
        </tr>

        {move || -> AnyView {
            if expanded.get() {
                let mut devs = cached_devices.get();
                // Sort by severity: DOWN (0) first, then UP (2). Stable sort preserves original order within ranks.
                devs.sort_by(|a, b| {
                    let rank_a: u8 = if a.status == "Down" { 0 } else { 2 };
                    let rank_b: u8 = if b.status == "Down" { 0 } else { 2 };
                    rank_a.cmp(&rank_b)
                });
                let cid = company_id_for_view.clone();
                let pid = probe_id_for_view.clone();
                any(view! {
                    <tr class="device-subtable-row">
                        <td colspan="8">
                            {if is_expired {
                                any(view! {
                                    <div class="probe-expired-banner">
                                        "\u{26A0} Probe expired \u{2014} device states may be outdated."
                                    </div>
                                })
                            } else {
                                any(view! { <></> })
                            }}
                            <table class="device-table">
                                <thead>
                                    <tr>
                                        <th>"Name"</th>
                                        <th>"Target"</th>
                                        <th>"Status"</th>
                                        <th>"Latency"</th>
                                        <th>"Metric"</th>
                                        <th>"Message"</th>
                                        <th>"Alerts"</th>
                                        <th>"Actions"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    <For
                                        each=move || devs.clone()
                                        key=|d: &ProbeDevice| format!("{}-{}", d.name, d.target)
                                        children=move |dev: ProbeDevice| {
                                            let status_class = if dev.status == "Up" { "up" } else { "down" };
                                            let row_class = if dev.status == "Down" { "device-row-down" } else { "" };
                                            let status_label = dev.status.to_uppercase();
                                            let latency_str = dev.latency_ms
                                                .map(|l| format!("{}ms", l))
                                                .unwrap_or_else(|| "—".to_string());
                                            let metric_str = if let (Some(val), Some(unit)) = (dev.metric_value, &dev.metric_unit) {
                                                format!("{:.1}{}", val, unit)
                                            } else {
                                                "—".to_string()
                                            };
                                            let msg = dev.message.clone().unwrap_or_else(|| "—".to_string());

                                            // Reactive is_silenced — re-evaluates whenever `silences` signal changes.
                                            // Signal::derive returns a Signal<bool> which is Copy,
                                            // so it can be used in multiple reactive closures.
                                            let resource_key = format!("{}:{}:{}:{}",
                                                cid,
                                                pid,
                                                dev.name,
                                                dev.target
                                            );
                                            let rk = resource_key.clone();
                                            let is_silenced = Signal::derive(move || {
                                                silences.get().iter().any(|s| s.scope_type == "resource" && s.scope_id == rk)
                                            });

                                            // Clone data for click handler
                                            let dev_for_click = dev.clone();
                                            let cid_for_click = cid.clone();
                                            let pid_for_click = pid.clone();
                                            let rk_for_unmute = resource_key.clone();

                                            // Toggle handler: unmute if silenced, open modal if not
                                            let silence_click = move |_| {
                                                if silences.get().iter().any(|s| s.scope_type == "resource" && s.scope_id == rk_for_unmute) {
                                                    // Find the silence ID and unmute
                                                    let silence_id = silences.get().iter()
                                                        .find(|s| s.scope_type == "resource" && s.scope_id == rk_for_unmute)
                                                        .map(|s| s.id);
                                                    if let Some(sid) = silence_id {
                                                        // Optimistic: remove from local signal immediately
                                                        silences.update(|list| list.retain(|s| s.id != sid));
                                                        // Fire-and-forget POST to backend
                                                        leptos::task::spawn_local(async move {
                                                            #[cfg(not(feature = "ssr"))]
                                                            {
                                                                let body = serde_json::json!({ "silence_id": sid });
                                                                if let Err(e) = post_json("/api/silence/delete", &body.to_string()).await {
                                                                    eprintln!("Unmute failed: {}", e);
                                                                }
                                                            }
                                                        });
                                                    }
                                                } else {
                                                    // Not silenced — open modal
                                                    silence_company_id.set(cid_for_click.clone());
                                                    silence_probe_id.set(pid_for_click.clone());
                                                    silence_device.set(Some(dev_for_click.clone()));
                                                    silence_modal_open.set(true);
                                                }
                                            };

                                            view! {
                                                <tr class=row_class>
                                                    <td>{dev.name.clone()}</td>
                                                    <td>{dev.target.clone()}</td>
                                                    <td>
                                                        <span class=format!("dev-status {}", status_class)></span>
                                                        <strong>{status_label}</strong>
                                                    </td>
                                                    <td>{latency_str}</td>
                                                    <td>{metric_str}</td>
                                                    <td>{msg}</td>
                                                    <td class="silence-indicator">
                                                        {move || if is_silenced.get() { "🔕" } else { "" }}
                                                    </td>
                                                    <td>
                                                        <button
                                                            class=move || if is_silenced.get() { "btn-small btn-muted" } else { "btn-small" }
                                                            on:click=silence_click
                                                        >
                                                            {move || if is_silenced.get() { "\u{1F515} Muted" } else { "Silence" }}
                                                        </button>
                                                    </td>
                                                </tr>
                                            }
                                        }
                                    />
                                </tbody>
                            </table>
                        </td>
                    </tr>
                })
            } else {
                any(view! { <></> })
            }
        }}
    }
}

// ===========================================================================
// UptimeChart — pure SVG line chart
// ===========================================================================

#[component]
fn UptimeChart(
    buckets: Vec<UptimeBucket>,
    title: Signal<String>,
    selected_range: RwSignal<String>,
) -> impl IntoView {
    const MARGIN_L: f64 = 60.0;
    const MARGIN_R: f64 = 20.0;
    const MARGIN_T: f64 = 20.0;
    const MARGIN_B: f64 = 40.0;
    const PLOT_W: f64 = 960.0 - MARGIN_L - MARGIN_R; // 880
    const PLOT_H: f64 = 300.0 - MARGIN_T - MARGIN_B; // 240

    // Build the entire SVG content as a string, then render via a single <div> wrapper
    let svg_inner: String = if buckets.is_empty() {
        r#"<text class="no-data" x="480" y="150" text-anchor="middle">No uptime data available</text>"#.to_string()
    } else {
        let bucket_count = buckets.len();

        // Compute percentage per bucket
        let points: Vec<(f64, f64)> = buckets
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let total = b.up_count + b.down_count;
                let pct = if total == 0 {
                    100.0
                } else {
                    (b.up_count as f64 / total as f64) * 100.0
                };
                let x = MARGIN_L + (i as f64 / (bucket_count - 1).max(1) as f64) * PLOT_W;
                let y = MARGIN_T + PLOT_H - (pct / 100.0) * PLOT_H;
                (x, y)
            })
            .collect();

        // Y-axis gridlines
        let mut svg = String::new();
        let y_ticks: [(f64, &str); 5] = [
            (0.0, "100%"),
            (25.0, "75%"),
            (50.0, "50%"),
            (75.0, "25%"),
            (100.0, "0%"),
        ];
        for (pct, label) in &y_ticks {
            let y = MARGIN_T + (pct / 100.0) * PLOT_H;
            svg.push_str(&format!(
                r#"<line class="gridline" x1="{}" y1="{:.1}" x2="{}" y2="{:.1}"/>"#,
                MARGIN_L,
                y,
                MARGIN_L + PLOT_W,
                y
            ));
            svg.push_str(&format!(
                r#"<text class="axis-label" x="{}" y="{:.1}" text-anchor="end">{}</text>"#,
                MARGIN_L - 8.0,
                y + 4.0,
                label
            ));
        }

        // X-axis labels — format and density depend on selected range.
        // Target ~8 labels regardless of bucket count.
        let range = selected_range.get_untracked();
        let step = (bucket_count / 8).max(1);
        for (i, b) in buckets.iter().enumerate() {
            if i % step != 0 {
                continue;
            }
            let x = MARGIN_L + (i as f64 / (bucket_count - 1).max(1) as f64) * PLOT_W;
            let s = &b.bucket;
            let time_label = match range.as_str() {
                "24h" => if s.len() >= 16 { &s[11..16] } else { s.as_str() },
                _     => if s.len() >= 10 { &s[5..10]  } else { s.as_str() },
            };
            let y = 300.0 - MARGIN_B + 18.0;
            svg.push_str(&format!(
                r#"<text class="axis-label" x="{:.1}" y="{:.1}" text-anchor="middle">{}</text>"#,
                x, y, time_label
            ));
        }

        // Gradient definition
        svg.push_str(
            r#"<defs>
            <linearGradient id="uptime-gradient" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stop-color="var(--color-up)" stop-opacity="0.3"/>
                <stop offset="100%" stop-color="var(--color-up)" stop-opacity="0.0"/>
            </linearGradient>
        </defs>"#,
        );

        // Area path
        if !points.is_empty() {
            let mut d = format!("M {:.1},{:.1}", points[0].0, points[0].1);
            for (x, y) in &points[1..] {
                d.push_str(&format!(" L {:.1},{:.1}", x, y));
            }
            let last_x = points.last().unwrap().0;
            let first_x = points[0].0;
            d.push_str(&format!(
                " L {:.1},{:.1} L {:.1},{:.1} Z",
                last_x,
                MARGIN_T + PLOT_H,
                first_x,
                MARGIN_T + PLOT_H
            ));
            svg.push_str(&format!(r#"<path class="area-fill" d="{}"/>"#, d));
        }

        // Line
        let line_points: String = points
            .iter()
            .map(|(x, y)| format!("{:.1},{:.1}", x, y))
            .collect::<Vec<_>>()
            .join(" ");
        svg.push_str(&format!(
            r#"<polyline class="line" points="{}"/>"#,
            line_points
        ));

        // Data points
        for (x, y) in &points {
            svg.push_str(&format!(
                r#"<circle class="point" cx="{:.1}" cy="{:.1}" r="3"/>"#,
                x, y
            ));
        }

        svg
    };

    let _full_svg = format!(
        r#"<svg class="chart-svg" viewBox="0 0 960 300">{}</svg>"#,
        svg_inner
    );

    // We render the chart section header + SVG. The SVG is injected as raw HTML
    // via an Effect that sets innerHTML on a container div.
    let chart_ref = NodeRef::<leptos::html::Div>::new();

    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            if let Some(el) = chart_ref.get() {
                el.set_inner_html(&_full_svg);
            }
        }
    });

    let ranges: Vec<(&'static str, &'static str)> =
        vec![("24h", "24h"), ("7d", "7d"), ("30d", "30d"), ("all", "All")];

    view! {
        <div class="chart-container">
            <div class="section-header">
                <h2>{move || title.get()}</h2>
                <div class="range-pills">
                    {ranges.into_iter().map(|(value, label)| {
                        let value_owned = value.to_string();
                        let value_for_click = value.to_string();
                        view! {
                            <button
                                class=move || {
                                    if selected_range.get() == value_owned {
                                        "range-pill range-pill-active"
                                    } else {
                                        "range-pill"
                                    }
                                }
                                on:click=move |_| {
                                    selected_range.set(value_for_click.clone());
                                }
                            >
                                {label}
                            </button>
                        }
                    }).collect::<Vec<_>>()}
                </div>
            </div>
            <div node_ref=chart_ref></div>
        </div>
    }
}

// ===========================================================================
// ArchiveViewerPage (Read-Only Historical View)
// ===========================================================================

#[component]
fn ArchiveViewerPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let archive_id = move || {
        params
            .read()
            .get("archive_id")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0)
    };

    let companies: LocalResource<Result<Vec<DashboardCompany>, String>> =
        LocalResource::new(move || async move {
            let id = archive_id();
            if id == 0 {
                return Err("Invalid archive ID".to_string());
            }
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<Vec<DashboardCompany>>(&format!("/api/archive/{}/companies", id)).await
            }
            #[cfg(feature = "ssr")]
            {
                Ok(vec![])
            }
        });

    view! {
        <div class="archive-banner">
            "📂 HISTORICAL VIEW — READ ONLY ARCHIVE"
        </div>

        <Suspense fallback=|| view! { <div class="loading">"Loading historical data…"</div> }>
            {move || -> AnyView {
                match companies.get() {
                    Some(Ok(list)) if !list.is_empty() => {
                        any(view! {
                            <div class="company-grid">
                                <For
                                    each=move || list.clone()
                                    key=|c: &DashboardCompany| c.company_id.clone()
                                    children=move |company: DashboardCompany| {
                                        let last_report_display = company.last_report
                                            .as_deref()
                                            .unwrap_or("No reports yet")
                                            .to_string();

                                        view! {
                                            <div class="company-card">
                                                <div class="company-header">
                                                    <h3 class="company-name">{company.company_id.clone()}</h3>
                                                    <span class="last-report">{last_report_display}</span>
                                                </div>
                                                <div class="company-stats">
                                                    <div class="stat">
                                                        <span class="stat-label">"Probes"</span>
                                                        <span class="stat-value">{company.total_probes}</span>
                                                    </div>
                                                    <div class="stat">
                                                        <span class="stat-label">"Active"</span>
                                                        <span class="stat-value stat-up">{company.active_probes}</span>
                                                    </div>
                                                    <div class="stat">
                                                        <span class="stat-label">"Expired"</span>
                                                        <span class="stat-value stat-down">{company.expired_probes}</span>
                                                    </div>
                                                    <div class="stat">
                                                        <span class="stat-label">"Devices Up"</span>
                                                        <span class="stat-value stat-up">{company.devices_up}</span>
                                                    </div>
                                                    <div class="stat">
                                                        <span class="stat-label">"Devices Down"</span>
                                                        <span class="stat-value stat-down">{company.devices_down}</span>
                                                    </div>
                                                </div>
                                                <div class="company-actions">
                                                    <span class="archive-note">"Historical data only - no actions available"</span>
                                                </div>
                                            </div>
                                        }
                                    }
                                />
                            </div>
                        })
                    }
                    Some(Ok(_)) => any(view! {
                        <div class="empty-state">"No companies in this archive."</div>
                    }),
                    Some(Err(e)) => any(view! {
                        <div class="error">
                            <p>"Failed to load archive data"</p>
                            <p class="error-detail">{e}</p>
                        </div>
                    }),
                    None => any(view! { <div class="loading">"Loading…"</div> }),
                }
            }}
        </Suspense>
    }
}

// ===========================================================================
// AdminPage
// ===========================================================================

const HUB_VERSION: &str = "0.1.0";
const HUB_BUILD: &str = "1022";
const HUB_BUILD_DATE: &str = "2026-03-08";

#[component]
fn AdminPage() -> impl IntoView {
    let companies: LocalResource<Result<Vec<String>, String>> = LocalResource::new(|| async move {
        #[cfg(not(feature = "ssr"))]
        {
            fetch_json::<Vec<String>>("/api/admin/companies").await
        }
        #[cfg(feature = "ssr")]
        {
            Ok(vec![])
        }
    });

    let probes: LocalResource<Result<Vec<AdminProbe>, String>> =
        LocalResource::new(|| async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<Vec<AdminProbe>>("/api/admin/probes").await
            }
            #[cfg(feature = "ssr")]
            {
                Ok(vec![])
            }
        });

    // Storage info resource
    let storage_info: LocalResource<Result<StorageInfo, String>> =
        LocalResource::new(|| async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<StorageInfo>("/api/admin/storage-info").await
            }
            #[cfg(feature = "ssr")]
            {
                Ok(StorageInfo {
                    current_db_size_mb: 0,
                    warn_threshold_mb: 1000,
                    retention_days: 90,
                    last_archive_date: None,
                    last_archive_size_mb: None,
                    warning: false,
                })
            }
        });

    // Archives resource
    let archives: LocalResource<Result<Vec<ArchiveMetadata>, String>> =
        LocalResource::new(|| async move {
            #[cfg(not(feature = "ssr"))]
            {
                fetch_json::<Vec<ArchiveMetadata>>("/api/admin/archives").await
            }
            #[cfg(feature = "ssr")]
            {
                Ok(vec![])
            }
        });

    // Archive operation state
    let archiving = RwSignal::new(false);
    let archive_error = RwSignal::new(String::new());

    let do_archive = move |_: _| {
        archiving.set(true);
        archive_error.set(String::new());

        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                match post_json("/api/admin/archive", "{}").await {
                    Ok(200) => {
                        storage_info.refetch();
                        archives.refetch();
                    }
                    Ok(status) => {
                        archive_error.set(format!("Archive failed with status {}", status));
                    }
                    Err(e) => {
                        archive_error.set(format!("Archive error: {}", e));
                    }
                }
            }
            archiving.set(false);
        });
    };

    // Modal state
    let modal_title = RwSignal::new(String::new());
    let modal_message = RwSignal::new(String::new());
    let modal_action = RwSignal::new(String::new());
    let modal_url = RwSignal::new(String::new());
    let modal_open = RwSignal::new(false);

    let show_modal = move |title: &str, message: &str, url: &str, body: &str| {
        modal_title.set(title.to_string());
        modal_message.set(message.to_string());
        modal_url.set(url.to_string());
        modal_action.set(body.to_string());
        modal_open.set(true);
    };

    let confirm_action = move |_: _| {
        let _url = modal_url.get();
        let _body = modal_action.get();
        modal_open.set(false);

        leptos::task::spawn_local(async move {
            #[cfg(not(feature = "ssr"))]
            {
                match post_json(&_url, &_body).await {
                    Ok(200) => {
                        leptos::web_sys::window()
                            .unwrap()
                            .location()
                            .reload()
                            .unwrap_or(());
                    }
                    Ok(status) => {
                        eprintln!("Admin action returned {}", status);
                    }
                    Err(e) => {
                        eprintln!("Admin action error: {}", e);
                    }
                }
            }
        });
    };

    let cancel_modal = move |_: _| {
        modal_open.set(false);
    };

    view! {
        <Suspense fallback=|| view! { <div class="loading">"Loading…"</div> }>
            // Storage & Retention Section
            <div class="admin-section">
                <h2>"Storage & Retention"</h2>
                {move || -> AnyView {
                    match storage_info.get() {
                        Some(Ok(info)) => {
                            let warning_class = if info.warning { "storage-warning" } else { "" };
                            let last_archive_date_display = info.last_archive_date
                                .as_deref()
                                .unwrap_or("Never")
                                .to_string();
                            let last_archive_size_display = info.last_archive_size_mb
                                .map(|s| format!("{} MB", s))
                                .unwrap_or_else(|| "—".to_string());

                            any(view! {
                                <div class={format!("storage-info-card {}", warning_class)}>
                                    <div class="storage-stats">
                                        <div class="storage-stat">
                                            <span class="stat-label">"Current DB Size:"</span>
                                            <span class="stat-value">{format!("{} MB", info.current_db_size_mb)}</span>
                                        </div>
                                        <div class="storage-stat">
                                            <span class="stat-label">"Warning Threshold:"</span>
                                            <span class="stat-value">{format!("{} MB", info.warn_threshold_mb)}</span>
                                        </div>
                                        <div class="storage-stat">
                                            <span class="stat-label">"Retention Window:"</span>
                                            <span class="stat-value">{format!("{} days", info.retention_days)}</span>
                                        </div>
                                        <div class="storage-stat">
                                            <span class="stat-label">"Last Archive Date:"</span>
                                            <span class="stat-value">{last_archive_date_display}</span>
                                        </div>
                                        <div class="storage-stat">
                                            <span class="stat-label">"Last Archive Size:"</span>
                                            <span class="stat-value">{last_archive_size_display}</span>
                                        </div>
                                    </div>

                                    {move || if info.warning {
                                        any(view! {
                                            <div class="storage-warning-banner">
                                                "⚠ Database size exceeds threshold. Consider archiving old data."
                                            </div>
                                        })
                                    } else {
                                        any(view! { <></> })
                                    }}

                                    <div class="storage-actions">
                                        <button
                                            class="btn btn-primary"
                                            disabled=archiving
                                            on:click=do_archive
                                        >
                                            {move || if archiving.get() { "Archiving…" } else { "Archive Old Data Now" }}
                                        </button>
                                    </div>

                                    {move || {
                                        let err = archive_error.get();
                                        if !err.is_empty() {
                                            any(view! {
                                                <div class="error-message">{err}</div>
                                            })
                                        } else {
                                            any(view! { <></> })
                                        }
                                    }}
                                </div>
                            })
                        }
                        Some(Err(e)) => any(view! { <div class="error">{format!("Error: {}", e)}</div> }),
                        None => any(view! { <div class="loading">"Loading…"</div> }),
                    }
                }}
            </div>

            // Archives Section
            <div class="admin-section">
                <h2>"Archives"</h2>
                {move || -> AnyView {
                    match archives.get() {
                        Some(Ok(list)) if !list.is_empty() => {
                            any(view! {
                                <table class="admin-table">
                                    <thead>
                                        <tr>
                                            <th>"Date Created"</th>
                                            <th>"Data Cutoff"</th>
                                            <th>"Size (MB)"</th>
                                            <th>"Actions"</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        <For
                                            each=move || list.clone()
                                            key=|a: &ArchiveMetadata| a.id
                                            children=|archive: ArchiveMetadata| {
                                                let archive_url = format!("/archive/{}", archive.id);
                                                view! {
                                                    <tr>
                                                        <td>{archive.created_at.clone()}</td>
                                                        <td>{archive.cutoff_date.clone()}</td>
                                                        <td>{archive.size_mb}</td>
                                                        <td>
                                                            <a href=archive_url class="btn btn-primary btn-sm">"Open"</a>
                                                        </td>
                                                    </tr>
                                                }
                                            }
                                        />
                                    </tbody>
                                </table>
                            })
                        }
                        Some(Ok(_)) => any(view! {
                            <div class="empty-state">"No archives yet. Archive old data to create an archive."</div>
                        }),
                        Some(Err(e)) => any(view! { <div class="error">{format!("Error: {}", e)}</div> }),
                        None => any(view! { <div class="loading">"Loading…"</div> }),
                    }
                }}
            </div>

            <div class="admin-section">
                <h2>"Companies"</h2>
                {move || -> AnyView {
                    match companies.get() {
                        Some(Ok(list)) if !list.is_empty() => {
                            let show = show_modal;
                            any(view! {
                                <div class="admin-list">
                                    <For
                                        each=move || list.clone()
                                        key=|c: &String| c.clone()
                                        children=move |company: String| {
                                            let c = company.clone();
                                            let on_delete = move |_: _| {
                                                show(
                                                    "Delete Company Data",
                                                    &format!("Delete all report data for \"{}\"? The probe heartbeat will remain.", c),
                                                    "/api/admin/delete-company-data",
                                                    &format!("{{\"company_id\":\"{}\"}}", c),
                                                );
                                            };
                                            view! {
                                                <div class="admin-list-item">
                                                    <div class="admin-item-info">
                                                        <span class="admin-item-name">{company.clone()}</span>
                                                    </div>
                                                    <div class="admin-item-actions">
                                                        <button class="btn btn-danger" on:click=on_delete>"Delete Data"</button>
                                                    </div>
                                                </div>
                                            }
                                        }
                                    />
                                </div>
                            })
                        }
                        Some(Ok(_)) => any(view! { <div class="empty-state">"No companies."</div> }),
                        Some(Err(e)) => any(view! { <div class="error">{format!("Error: {}", e)}</div> }),
                        None => any(view! { <div class="loading">"Loading…"</div> }),
                    }
                }}
            </div>

            `<div class="admin-section">
                <h2>"Probes"</h2>
                {move || -> AnyView {
                    match probes.get() {
                        Some(Ok(list)) if !list.is_empty() => {
                            let show = show_modal;
                            any(view! {
                                <div class="admin-list">
                                    <For
                                        each=move || list.clone()
                                        key=|p: &AdminProbe| p.probe_id.clone()
                                        children=move |probe: AdminProbe| {
                                            let pid_del = probe.probe_id.clone();
                                            let pid_rem = probe.probe_id.clone();
                                            let cid = probe.company_id.clone();
                                            let short_id = format!("{}…", &probe.probe_id[..8.min(probe.probe_id.len())]);
                                            let short_id2 = short_id.clone();

                                            let on_delete_data = move |_: _| {
                                                show(
                                                    "Delete Probe Data",
                                                    &format!("Delete all report data for probe \"{}\"? The heartbeat will remain.", short_id),
                                                    "/api/admin/delete-probe-data",
                                                    &format!("{{\"probe_id\":\"{}\"}}", pid_del),
                                                );
                                            };
                                            let on_remove = move |_: _| {
                                                show(
                                                    "Remove Probe",
                                                    &format!("Completely remove probe \"{}\"? This deletes all data and the heartbeat.", short_id2),
                                                    "/api/admin/remove-probe",
                                                    &format!("{{\"probe_id\":\"{}\"}}", pid_rem),
                                                );
                                            };

                                            view! {
                                                <div class="admin-list-item">
                                                    <div class="admin-item-info">
                                                        <span class="admin-item-name">{format!("{}…", &probe.probe_id[..8.min(probe.probe_id.len())])}</span>
                                                        <span class="admin-item-sub">{format!("Company: {}", cid)}</span>
                                                    </div>
                                                    <div class="admin-item-actions">
                                                        <button class="btn btn-warning" on:click=on_delete_data>"Delete Data"</button>
                                                        <button class="btn btn-danger" on:click=on_remove>"Remove"</button>
                                                    </div>
                                                </div>
                                            }
                                        }
                                    />
                                </div>
                            })
                        }
                        Some(Ok(_)) => any(view! { <div class="empty-state">"No probes."</div> }),
                        Some(Err(e)) => any(view! { <div class="error">{format!("Error: {}", e)}</div> }),
                        None => any(view! { <div class="loading">"Loading…"</div> }),
                    }
                }}
            </div>

            <div class="admin-section">
                <h2>"Hub Version"</h2>

                <div class="storage-info-card">
                    <div class="storage-stats">
                        <div class="storage-stat">
                            <span class="stat-label">"Version:"</span>
                            <span class="stat-value">{HUB_VERSION}</span>
                        </div>

                        <div class="storage-stat">
                            <span class="stat-label">"Build:"</span>
                            <span class="stat-value">{HUB_BUILD}</span>
                        </div>

                        <div class="storage-stat">
                            <span class="stat-label">"Build Date:"</span>
                            <span class="stat-value">{HUB_BUILD_DATE}</span>
                        </div>
                    </div>
                </div>
            </div>
        </Suspense>

        // Confirmation modal
        {move || -> AnyView {
            if modal_open.get() {
                any(view! {
                    <div class="modal-overlay">
                        <div class="modal">
                            <h3>{modal_title.get()}</h3>
                            <p>{modal_message.get()}</p>
                            <div class="modal-actions">
                                <button class="btn btn-secondary" on:click=cancel_modal>"Cancel"</button>
                                <button class="btn btn-danger" on:click=confirm_action>"Confirm"</button>
                            </div>
                        </div>
                    </div>
                })
            } else {
                any(view! { <></> })
            }
        }}
    }
}
