//! One module per route (see the router in `crate::app`).

mod admin;
mod archive_viewer;
mod company_detail;
mod dashboard;
mod login;

pub(crate) use admin::AdminPage;
pub(crate) use archive_viewer::ArchiveViewerPage;
pub(crate) use company_detail::CompanyDetailPage;
pub(crate) use dashboard::DashboardPage;
pub(crate) use login::LoginPage;
