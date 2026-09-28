//! The web UI (Leptos): server-side rendered, hydrated in the browser.
//!
//! - `pages/`: one module per route.
//! - `components/`: pieces used by several pages or large enough to stand alone.
//! - `ui`: presentation helpers; `http` and `browser`: client-only API calls
//!   and browser APIs.
//!
//! Data types are shared with the server in `crate::model`.

use leptos::prelude::*;
use leptos_meta::{MetaTags, Stylesheet, Title, provide_meta_context};
use leptos_router::{
    ParamSegment, StaticSegment,
    components::{ParentRoute, Route, Router, Routes},
};

mod components;
mod pages;
mod ui;

#[cfg(not(feature = "ssr"))]
mod browser;
#[cfg(not(feature = "ssr"))]
mod http;

use components::AuthLayout;
use pages::{
    AccountPage, AdminPage, ArchiveViewerPage, AuditPage, CompaniesPage, CompanyDetailPage,
    CompanyEditPage, DashboardPage, LoginPage, SetupPage, UsersPage,
};

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
                <Route path=StaticSegment("setup") view=SetupPage/>
                <ParentRoute path=StaticSegment("") view=AuthLayout>
                    <Route path=StaticSegment("") view=DashboardPage/>
                    <Route path=StaticSegment("admin") view=AdminPage/>
                    <Route path=StaticSegment("users") view=UsersPage/>
                    <Route path=StaticSegment("audit") view=AuditPage/>
                    <Route path=StaticSegment("companies") view=CompaniesPage/>
                    <Route path=(StaticSegment("companies"), ParamSegment("company_id")) view=CompanyEditPage/>
                    <Route path=StaticSegment("account") view=AccountPage/>
                    <Route path=(StaticSegment("company"), ParamSegment("company_id")) view=CompanyDetailPage/>
                    <Route path=(StaticSegment("archive"), ParamSegment("archive_id")) view=ArchiveViewerPage/>
                    <Route
                        path=(StaticSegment("archive"), ParamSegment("archive_id"), StaticSegment("company"), ParamSegment("company_id"))
                        view=CompanyDetailPage
                    />
                </ParentRoute>
            </Routes>
        </Router>
    }
}
