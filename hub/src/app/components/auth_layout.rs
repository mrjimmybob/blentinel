//! Layout for every logged-in page: loads the user once (shared as
//! [`Me`]), sends visitors without a session to /login, and users who must
//! change their password to /account.

use leptos::prelude::*;
use leptos_router::components::Outlet;

use crate::app::components::Header;
use crate::app::ui::Me;
#[cfg(not(feature = "ssr"))]
use crate::app::{
    browser,
    http::{UNAUTHORIZED, fetch_json},
};
use crate::model::UserInfo;

#[component]
pub(crate) fn AuthLayout() -> impl IntoView {
    let me = Me(RwSignal::new(None));
    provide_context(me);

    #[cfg_attr(feature = "ssr", allow(unused_variables))] // read only in the browser
    let location = leptos_router::hooks::use_location();

    #[cfg_attr(feature = "ssr", allow(unused_variables))] // read only in the browser
    let user: LocalResource<Result<UserInfo, String>> = LocalResource::new(|| async move {
        #[cfg(not(feature = "ssr"))]
        {
            fetch_json::<UserInfo>("/api/auth/me").await
        }
        #[cfg(feature = "ssr")]
        {
            Err(String::new())
        }
    });

    // Effects only run in the browser, so the checks are compiled there only.
    Effect::new(move |_| {
        #[cfg(not(feature = "ssr"))]
        match user.get() {
            Some(Err(ref e)) if e == UNAUTHORIZED => browser::navigate("/login"),
            Some(Ok(u)) => {
                let must_change = u.must_change_password;
                me.0.set(Some(u));
                if must_change && location.pathname.get_untracked() != "/account" {
                    browser::navigate("/account");
                }
            }
            _ => {}
        }
    });

    view! {
        <Header/>
        <div class="main-content">
            <Outlet/>
        </div>
    }
}
