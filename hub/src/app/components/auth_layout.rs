//! Layout for every logged-in page: redirects to /login without a session.

use leptos::prelude::*;
use leptos_router::components::Outlet;

use crate::app::components::Header;
#[cfg(not(feature = "ssr"))]
use crate::app::{
    browser,
    http::{UNAUTHORIZED, fetch_json},
};
use crate::model::DashboardCompany;

#[component]
pub(crate) fn AuthLayout() -> impl IntoView {
    #[cfg_attr(feature = "ssr", allow(unused_variables))] // read only by the browser-side check
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

    // Effects only run in the browser, so the check is compiled there only.
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        if let Some(Err(ref e)) = companies.get()
            && e == UNAUTHORIZED
        {
            browser::navigate("/login");
        }
    });

    view! {
        <Header/>
        <div class="main-content">
            <Outlet/>
        </div>
    }
}
