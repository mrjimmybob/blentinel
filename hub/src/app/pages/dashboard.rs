//! Dashboard: one card per company, worst first, refreshed every 15 s.

use leptos::prelude::*;

use crate::app::components::CompanyCard;
#[cfg(not(feature = "ssr"))]
use crate::app::{
    browser,
    http::{UNAUTHORIZED, fetch_json},
};
use crate::model::DashboardCompany;

#[component]
pub(crate) fn DashboardPage() -> impl IntoView {
    // RwSignal + spawn_local instead of LocalResource: a plain signal never has a
    // None/pending state, so Memo closures reading it never hit the Leptos 0.8
    // issue where calling resource.get() in a non-Suspense reactive context
    // panics/suspends the owning scope on every refetch cycle.
    let companies_data = RwSignal::new(Vec::<DashboardCompany>::new());
    // Always start loading — SSR and client must agree on initial DOM to avoid
    // hydration mismatch (which causes Leptos to tear down and rebuild the component).
    let is_initial_loading = RwSignal::new(true);
    let fetch_error: RwSignal<Option<String>> = RwSignal::new(None);

    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        {
            // Initial fetch — Effect has no reactive deps so this runs exactly once.
            leptos::task::spawn_local(async move {
                match fetch_json::<Vec<DashboardCompany>>("/api/dashboard/companies").await {
                    Ok(list) => {
                        companies_data.set(list);
                        is_initial_loading.set(false);
                    }
                    Err(e) if e == UNAUTHORIZED => {
                        browser::navigate("/login");
                    }
                    Err(e) => {
                        fetch_error.set(Some(e));
                        is_initial_loading.set(false);
                    }
                }
            });

            // Poll every 15 seconds: fetch and set the signal.
            // The Memo below suppresses downstream updates when data is unchanged.
            browser::poll_every(15_000, move || {
                leptos::task::spawn_local(async move {
                    if let Ok(list) =
                        fetch_json::<Vec<DashboardCompany>>("/api/dashboard/companies").await
                    {
                        companies_data.set(list);
                    }
                });
            });
        }
    });

    // Memo: fires downstream only when the list actually changes (PartialEq on Vec).
    let companies_memo = Memo::new(move |_| companies_data.get());

    // True only until the first fetch completes.
    let is_loading: Memo<bool> = Memo::new(move |_| is_initial_loading.get());

    // Error message — None when successful.
    let error_msg: Memo<Option<String>> = Memo::new(move |_| fetch_error.get());

    // Per-stat Memos: each DOM text node only re-renders when its specific value changes.
    let stat_companies: Memo<i64> = Memo::new(move |_| companies_memo.get().len() as i64);
    let stat_probes: Memo<i64> =
        Memo::new(move |_| companies_memo.get().iter().map(|c| c.total_probes).sum());
    let stat_active: Memo<i64> =
        Memo::new(move |_| companies_memo.get().iter().map(|c| c.active_probes).sum());
    let stat_up: Memo<i64> =
        Memo::new(move |_| companies_memo.get().iter().map(|c| c.devices_up).sum());
    let stat_down: Memo<i64> =
        Memo::new(move |_| companies_memo.get().iter().map(|c| c.devices_down).sum());

    // Sorted company IDs — only fires when sort ORDER changes (company added/removed
    // or severity shifts). Data-only changes (e.g. last_report updating each poll)
    // that don't affect sort position never reach <For>, so existing cards stay mounted.
    let sorted_company_ids: Memo<Vec<String>> =
        Memo::new(move |_| dashboard_order(companies_memo.get()));

    view! {
        <div class="summary-stats">
            <div class="stat-card">
                <div class="stat-label">"Companies"</div>
                <div class="stat-value">{move || stat_companies.get()}</div>
            </div>
            <div class="stat-card">
                <div class="stat-label">"Probes"</div>
                <div class="stat-value">{move || stat_probes.get()}</div>
            </div>
            <div class="stat-card">
                <div class="stat-label">"Active"</div>
                <div class="stat-value" style="color: var(--color-up)">
                    {move || format!("{}/{}", stat_active.get(), stat_probes.get())}
                </div>
            </div>
            <div class="stat-card">
                <div class="stat-label">"Devices Up"</div>
                <div class="stat-value" style="color: var(--color-up)">{move || stat_up.get()}</div>
            </div>
            <div class="stat-card">
                <div class="stat-label">"Devices Down"</div>
                <div class="stat-value" style="color: var(--color-down)">{move || stat_down.get()}</div>
            </div>
        </div>
        <Show when=move || is_loading.get()>
            <div class="loading">"Loading…"</div>
        </Show>
        <Show when=move || error_msg.get().is_some()>
            <div class="error">{move || error_msg.get().unwrap_or_default()}</div>
        </Show>
        <Show when=move || !is_loading.get() && error_msg.get().is_none() && sorted_company_ids.get().is_empty()>
            <div class="empty-state">"No companies found. Deploy a probe to get started."</div>
        </Show>
        <Show when=move || !sorted_company_ids.get().is_empty()>
            <div class="company-grid">
                <For
                    each=move || sorted_company_ids.get()
                    key=|id: &String| id.clone()
                    children=move |id: String| {
                        // Per-company Memo: fires only when THIS company's data changes.
                        // <For> never re-runs this closure for existing IDs — only the
                        // reactive closures inside CompanyCard update when data changes.
                        let href = format!("/company/{id}");
                        let company_memo = Memo::new(move |prev: Option<&DashboardCompany>| {
                            companies_memo.get()
                                .into_iter()
                                .find(|c| c.company_id == id)
                                .or_else(|| prev.cloned())
                                .unwrap_or_default()
                        });
                        view! { <CompanyCard company=company_memo href=href/> }
                    }
                />
            </div>
        </Show>
    }
}

/// Company IDs in dashboard order: worst severity first (`Severity`'s `Ord`),
/// then alphabetical.
fn dashboard_order(mut companies: Vec<DashboardCompany>) -> Vec<String> {
    companies.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.company_id.cmp(&b.company_id))
    });
    companies.into_iter().map(|c| c.company_id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Severity;

    #[test]
    fn dashboard_order_puts_worst_first_then_alphabetical() {
        let company = |id: &str, severity| DashboardCompany {
            company_id: id.into(),
            severity,
            ..Default::default()
        };
        let order = dashboard_order(vec![
            company("b-ok", Severity::Healthy),
            company("z-down", Severity::Critical),
            company("a-ok", Severity::Healthy),
            company("m-stale", Severity::Degraded),
        ]);
        assert_eq!(order, ["z-down", "m-stale", "a-ok", "b-ok"]);
    }
}
