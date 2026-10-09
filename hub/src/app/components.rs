//! UI pieces used by the pages.

mod auth_layout;
mod backups;
mod certificate;
mod check_form;
mod company_card;
mod company_probes;
mod header;
mod hub_keys;
mod hub_version;
mod probe_commands;
mod probe_row;
mod probe_updates;
mod thresholds;
mod uptime_chart;

pub(crate) use auth_layout::AuthLayout;
pub(crate) use backups::BackupsSection;
pub(crate) use certificate::CertificateSection;
pub(crate) use check_form::{CheckDialog, kind_label, switched};
pub(crate) use company_card::CompanyCard;
pub(crate) use company_probes::CompanyProbes;
pub(crate) use header::Header;
pub(crate) use hub_keys::HubKeysSection;
pub(crate) use hub_version::HubVersionSection;
pub(crate) use probe_commands::ProbeCommands;
pub(crate) use probe_row::ProbeRow;
pub(crate) use probe_updates::ProbeUpdatesSection;
pub(crate) use thresholds::{CompanyThresholdsSection, GlobalThresholdsSection};
pub(crate) use uptime_chart::UptimeChart;
