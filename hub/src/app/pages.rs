//! One module per route (see the router in `crate::app`).

mod account;
mod admin;
mod archive_viewer;
mod company_detail;
mod dashboard;
mod login;
mod setup;
mod users;

pub(crate) use account::AccountPage;
pub(crate) use admin::AdminPage;
pub(crate) use archive_viewer::ArchiveViewerPage;
pub(crate) use company_detail::CompanyDetailPage;
pub(crate) use dashboard::DashboardPage;
pub(crate) use login::LoginPage;
pub(crate) use setup::SetupPage;
pub(crate) use users::UsersPage;
