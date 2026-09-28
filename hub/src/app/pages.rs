//! One module per route (see the router in `crate::app`).

mod account;
mod admin;
mod archive_viewer;
mod companies;
mod company_detail;
mod company_edit;
mod dashboard;
mod login;
mod setup;
mod users;

pub(crate) use account::AccountPage;
pub(crate) use admin::AdminPage;
pub(crate) use archive_viewer::ArchiveViewerPage;
pub(crate) use companies::CompaniesPage;
pub(crate) use company_detail::CompanyDetailPage;
pub(crate) use company_edit::CompanyEditPage;
pub(crate) use dashboard::DashboardPage;
pub(crate) use login::LoginPage;
pub(crate) use setup::SetupPage;
pub(crate) use users::UsersPage;
