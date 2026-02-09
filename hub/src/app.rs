use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{Route, Router, Routes, Outlet, ParentRoute},
    hooks::use_params_map,
    StaticSegment, ParamSegment,
};

// ===========================================================================
// Shared data types
// ===========================================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DashboardCompany {
    pub company_id:     String,
    pub total_probes:   i64,
    pub active_probes:  i64,
    pub expired_probes: i64,
    pub devices_up:     i64,
    pub devices_down:   i64,
    pub last_report:    Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompanyProbe {
    pub probe_id:     String,
    pub probe_name:   String,
    pub hostname:     Option<String>,
    pub site:         Option<String>,
    pub status:       String,
    pub last_seen_at: Option<String>,
    pub devices_up:   i64,
    pub devices_down: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProbeDevice {
    pub name:          String,
    pub resource_type: String,
    pub target:        String,
    pub status:        String,
    pub message:       Option<String>,
    pub latency_ms:    Option<i64>,
    pub metric_value:  Option<f64>,
    pub metric_unit:   Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UptimeBucket {
    pub bucket:     String,
    pub up_count:   i64,
    pub down_count: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AdminProbe {
    pub probe_id:      String,
    pub company_id:    String,
    pub status:        String,
    pub last_seen_at:  Option<String>,
    pub first_seen_at: Option<String>,
}

// ===========================================================================
// Helpers
// ===========================================================================

/// Type-erase a view so different branches can be returned from the same closure.
fn any(v: impl IntoView) -> AnyView {
    v.into_any()
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
    resp.json::<T>().await.map_err(|e| format!("JSON error: {}", e))
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
        <Stylesheet id="leptos" href="/pkg/blentinel_hub.css"/>
        <Title text="Blentinel Hub"/>

        <Router>
            <Routes fallback=|| view! { <div class="empty-state">"Page not found."</div> }>
                <Route path=StaticSegment("login") view=LoginPage/>
                <ParentRoute path=StaticSegment("") view=AuthLayout>
                    <Route path=StaticSegment("") view=DashboardPage/>
                    <Route path=StaticSegment("admin") view=AdminPage/>
                    <Route path=(StaticSegment("company"), ParamSegment("company_id")) view=CompanyDetailPage/>
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
                        leptos::web_sys::window().unwrap().location().set_href("/").unwrap_or(());
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
    let companies: LocalResource<Result<Vec<DashboardCompany>, String>> = LocalResource::new(|| async move {
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
                    leptos::web_sys::window().unwrap().location().set_href("/login").unwrap_or(());
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

    // On client mount: read persisted theme preference
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            let win = leptos::web_sys::window().unwrap();
            if let Ok(Some(storage)) = win.local_storage() {
                if let Ok(Some(saved)) = storage.get_item("blentinel_theme") {
                    theme.set(saved.clone());
                    leptos::web_sys::window().unwrap()
                        .document().unwrap()
                        .document_element().unwrap()
                        .set_attribute("data-theme", &saved)
                        .unwrap_or(());
                }
            }
        }
    });

    let toggle_theme = move |_: _| {
        let next = if theme.get() == "dark" { "light" } else { "dark" };
        theme.set(next.to_string());
        #[cfg(not(feature = "ssr"))]
        {
            let win = leptos::web_sys::window().unwrap();
            win.document().unwrap()
                .document_element().unwrap()
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
                leptos::web_sys::window().unwrap().location().set_href("/login").unwrap_or(());
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
    let companies: LocalResource<Result<Vec<DashboardCompany>, String>> = LocalResource::new(|| async move {
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
            let func = cb.as_ref().unchecked_ref::<leptos::web_sys::js_sys::Function>();
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
                    Some(Ok(list)) if !list.is_empty() => any(view! {
                        <div class="company-grid">
                            <For
                                each=move || list.clone()
                                key=|c: &DashboardCompany| c.company_id.clone()
                                children=|company: DashboardCompany| view! { <CompanyCard company=company/> }
                            />
                        </div>
                    }),
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

#[component]
fn CompanyCard(company: DashboardCompany) -> impl IntoView {
    let border_class = if company.expired_probes > 0 {
        "card-border-red"
    } else {
        "card-border-green"
    };

    let href = format!("/company/{}", company.company_id);
    let last_report_display = company.last_report.as_deref()
        .unwrap_or("Never")
        .to_string();

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
    let company_id = Signal::derive(move || {
        params.get().get("company_id").unwrap_or_default()
    });

    let probes: LocalResource<Result<Vec<CompanyProbe>, String>> = LocalResource::new(move || {
        let cid = company_id.get();
        async move {
            if cid.is_empty() { return Ok(vec![]); }
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

    let uptime: LocalResource<Result<Vec<UptimeBucket>, String>> = LocalResource::new(move || {
        let cid = company_id.get();
        async move {
            if cid.is_empty() { return Ok(vec![]); }
            #[cfg(not(feature = "ssr"))]
            {
                let url = format!("/api/company/{}/uptime", cid);
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
            let func = cb.as_ref().unchecked_ref::<leptos::web_sys::js_sys::Function>();
            leptos::web_sys::window()
                .unwrap()
                .set_interval_with_callback_and_timeout_and_arguments_0(func, 15_000)
                .unwrap();
            cb.forget();
        }
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
                    Some(Ok(buckets)) => any(view! { <UptimeChart buckets=buckets/> }),
                    _ => any(view! { <div/> }),
                }
            }}

            <div class="section-header">
                <h2>"Probes"</h2>
            </div>

            {move || -> AnyView {
                match probes.get() {
                    Some(Ok(list)) if !list.is_empty() => any(view! {
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
                                        children=|probe: CompanyProbe| view! { <ProbeRow probe=probe/> }
                                    />
                                </tbody>
                            </table>
                        </div>
                    }),
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
    }
}

// ===========================================================================
// ProbeRow
// ===========================================================================

#[component]
fn ProbeRow(probe: CompanyProbe) -> impl IntoView {
    let expanded = RwSignal::new(false);
    let cached_devices = RwSignal::new(Vec::<ProbeDevice>::new());
    let fetched = RwSignal::new(false);

    let probe_id_for_fetch = probe.probe_id.clone();

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

    let status_str = if probe.status == "active" { "active" } else { "expired" };
    let display_name = if probe.probe_name.is_empty() {
        format!("{}…", &probe.probe_id[..8.min(probe.probe_id.len())])
    } else {
        probe.probe_name.clone()
    };
    let hostname_display = probe.hostname.clone().unwrap_or_else(|| "—".to_string());
    let site_display = probe.site.clone().unwrap_or_else(|| "—".to_string());
    let last_seen = probe.last_seen_at.clone().unwrap_or_else(|| "Unknown".to_string());
    let probe_up = probe.devices_up;
    let probe_down = probe.devices_down;

    view! {
        <tr>
            <td>{display_name}</td>
            <td>{hostname_display}</td>
            <td>{site_display}</td>
            <td><span class=format!("status-badge {}", status_str)>{status_str}</span></td>
            <td>{probe_up}</td>
            <td>{probe_down}</td>
            <td>{last_seen}</td>
            <td>
                <button class="expand-btn" on:click=toggle>
                    {move || if expanded.get() { "▲ Hide" } else { "▼ Show" }}
                </button>
            </td>
        </tr>

        {move || -> AnyView {
            if expanded.get() {
                let devs = cached_devices.get();
                any(view! {
                    <tr class="device-subtable-row">
                        <td colspan="9">
                            <table class="device-table">
                                <thead>
                                    <tr>
                                        <th>"Name"</th>
                                        <th>"Type"</th>
                                        <th>"Target"</th>
                                        <th>"Status"</th>
                                        <th>"Latency"</th>
                                        <th>"Metric"</th>
                                        <th>"Message"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    <For
                                        each=move || devs.clone()
                                        key=|d: &ProbeDevice| format!("{}-{}", d.name, d.target)
                                        children=|dev: ProbeDevice| {
                                            let status_class = if dev.status == "Up" { "up" } else { "down" };
                                            let latency_str = dev.latency_ms
                                                .map(|l| format!("{}ms", l))
                                                .unwrap_or_else(|| "—".to_string());
                                            let metric_str = if let (Some(val), Some(unit)) = (dev.metric_value, &dev.metric_unit) {
                                                format!("{:.1}{}", val, unit)
                                            } else {
                                                "—".to_string()
                                            };
                                            let msg = dev.message.clone().unwrap_or_else(|| "—".to_string());
                                            view! {
                                                <tr>
                                                    <td>{dev.name.clone()}</td>
                                                    <td>{dev.resource_type.clone()}</td>
                                                    <td>{dev.target.clone()}</td>
                                                    <td>
                                                        <span class=format!("dev-status {}", status_class)></span>
                                                        {dev.status.clone()}
                                                    </td>
                                                    <td>{latency_str}</td>
                                                    <td>{metric_str}</td>
                                                    <td>{msg}</td>
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
fn UptimeChart(buckets: Vec<UptimeBucket>) -> impl IntoView {
    const MARGIN_L: f64 = 60.0;
    const MARGIN_R: f64 = 20.0;
    const MARGIN_T: f64 = 20.0;
    const MARGIN_B: f64 = 40.0;
    const PLOT_W: f64   = 960.0 - MARGIN_L - MARGIN_R;   // 880
    const PLOT_H: f64   = 300.0 - MARGIN_T - MARGIN_B;   // 240

    // Build the entire SVG content as a string, then render via a single <div> wrapper
    let svg_inner: String = if buckets.is_empty() {
        r#"<text class="no-data" x="480" y="150" text-anchor="middle">No uptime data available</text>"#.to_string()
    } else {
        let bucket_count = buckets.len();

        // Compute percentage per bucket
        let points: Vec<(f64, f64)> = buckets.iter().enumerate().map(|(i, b)| {
            let total = b.up_count + b.down_count;
            let pct = if total == 0 { 100.0 } else { (b.up_count as f64 / total as f64) * 100.0 };
            let x = MARGIN_L + (i as f64 / (bucket_count - 1).max(1) as f64) * PLOT_W;
            let y = MARGIN_T + PLOT_H - (pct / 100.0) * PLOT_H;
            (x, y)
        }).collect();

        // Y-axis gridlines
        let mut svg = String::new();
        let y_ticks: [(f64, &str); 5] = [
            (0.0, "100%"), (25.0, "75%"), (50.0, "50%"), (75.0, "25%"), (100.0, "0%"),
        ];
        for (pct, label) in &y_ticks {
            let y = MARGIN_T + (pct / 100.0) * PLOT_H;
            svg.push_str(&format!(
                r#"<line class="gridline" x1="{}" y1="{:.1}" x2="{}" y2="{:.1}"/>"#,
                MARGIN_L, y, MARGIN_L + PLOT_W, y
            ));
            svg.push_str(&format!(
                r#"<text class="axis-label" x="{}" y="{:.1}" text-anchor="end">{}</text>"#,
                MARGIN_L - 8.0, y + 4.0, label
            ));
        }

        // X-axis labels (every 4th bucket ≈ every hour)
        for (i, b) in buckets.iter().enumerate() {
            if i % 4 != 0 { continue; }
            let x = MARGIN_L + (i as f64 / (bucket_count - 1).max(1) as f64) * PLOT_W;
            let time_label = if b.bucket.len() >= 16 { &b.bucket[11..16] } else { &b.bucket };
            let y = 300.0 - MARGIN_B + 18.0;
            svg.push_str(&format!(
                r#"<text class="axis-label" x="{:.1}" y="{:.1}" text-anchor="middle">{}</text>"#,
                x, y, time_label
            ));
        }

        // Gradient definition
        svg.push_str(r#"<defs>
            <linearGradient id="uptime-gradient" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stop-color="var(--color-up)" stop-opacity="0.3"/>
                <stop offset="100%" stop-color="var(--color-up)" stop-opacity="0.0"/>
            </linearGradient>
        </defs>"#);

        // Area path
        if !points.is_empty() {
            let mut d = format!("M {:.1},{:.1}", points[0].0, points[0].1);
            for (x, y) in &points[1..] {
                d.push_str(&format!(" L {:.1},{:.1}", x, y));
            }
            let last_x = points.last().unwrap().0;
            let first_x = points[0].0;
            d.push_str(&format!(" L {:.1},{:.1} L {:.1},{:.1} Z",
                last_x, MARGIN_T + PLOT_H, first_x, MARGIN_T + PLOT_H));
            svg.push_str(&format!(r#"<path class="area-fill" d="{}"/>"#, d));
        }

        // Line
        let line_points: String = points.iter()
            .map(|(x, y)| format!("{:.1},{:.1}", x, y))
            .collect::<Vec<_>>()
            .join(" ");
        svg.push_str(&format!(r#"<polyline class="line" points="{}"/>"#, line_points));

        // Data points
        for (x, y) in &points {
            svg.push_str(&format!(r#"<circle class="point" cx="{:.1}" cy="{:.1}" r="3"/>"#, x, y));
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

    view! {
        <div class="chart-container">
            <div class="section-header">
                <h2>"Uptime — Last 24 Hours"</h2>
            </div>
            <div node_ref=chart_ref></div>
        </div>
    }
}

// ===========================================================================
// AdminPage
// ===========================================================================

#[component]
fn AdminPage() -> impl IntoView {
    let companies: LocalResource<Result<Vec<String>, String>> = LocalResource::new(|| async move {
        #[cfg(not(feature = "ssr"))]
        { fetch_json::<Vec<String>>("/api/admin/companies").await }
        #[cfg(feature = "ssr")]
        { Ok(vec![]) }
    });

    let probes: LocalResource<Result<Vec<AdminProbe>, String>> = LocalResource::new(|| async move {
        #[cfg(not(feature = "ssr"))]
        { fetch_json::<Vec<AdminProbe>>("/api/admin/probes").await }
        #[cfg(feature = "ssr")]
        { Ok(vec![]) }
    });

    // Modal state
    let modal_title   = RwSignal::new(String::new());
    let modal_message = RwSignal::new(String::new());
    let modal_action  = RwSignal::new(String::new());
    let modal_url     = RwSignal::new(String::new());
    let modal_open    = RwSignal::new(false);

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
                        leptos::web_sys::window().unwrap().location().reload().unwrap_or(());
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

            <div class="admin-section">
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
