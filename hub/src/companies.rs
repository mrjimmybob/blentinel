#![cfg(feature = "ssr")]
//! HTTP handlers for companies and sites (D-44). The rules are in
//! `service::companies`; access per route: `crate::routes`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};

use crate::AppState;
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::model::{
    ArchiveCompanyRequest, CompanyDetail, CompanyInfo, UpdateCompanyRequest, UpdateSiteRequest,
};
use crate::service::companies::{self, CompanyError};

impl From<CompanyError> for ApiError {
    fn from(error: CompanyError) -> Self {
        match error {
            CompanyError::NotFound | CompanyError::SiteNotFound => {
                ApiError::not_found(error.to_string())
            }
            CompanyError::EmptyName => {
                ApiError::new(StatusCode::BAD_REQUEST, "invalid_name", error.to_string())
            }
            CompanyError::Storage(e) => ApiError::internal("company operation", e),
        }
    }
}

/// `GET /api/companies`: all companies, archived ones included.
pub async fn list(State(state): State<AppState>) -> ApiResult<Json<Vec<CompanyInfo>>> {
    let list = state
        .storage
        .get()
        .companies()
        .await
        .map_err(|e| ApiError::internal("list companies", e))?;
    Ok(Json(list))
}

/// `GET /api/companies/{id}`: a company and its sites.
pub async fn detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<CompanyDetail>> {
    Ok(Json(
        companies::company(state.storage.get().as_ref(), &id).await?,
    ))
}

/// `POST /api/companies/{id}`: edit name, contacts and notes.
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateCompanyRequest>,
) -> ApiResult<Json<CompanyDetail>> {
    let store = state.storage.get();
    let detail = companies::update_company(store.as_ref(), &id, body, state.clock.now()).await?;
    Ok(Json(detail))
}

/// `POST /api/companies/{id}/archive`: `{"archived": bool}`.
pub async fn archive(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ArchiveCompanyRequest>,
) -> ApiResult<Json<CompanyDetail>> {
    let store = state.storage.get();
    let detail =
        companies::set_archived(store.as_ref(), &id, body.archived, state.clock.now()).await?;
    Ok(Json(detail))
}

/// `POST /api/companies/{id}/sites/{site}`: edit a site's name, address and notes.
pub async fn update_site(
    State(state): State<AppState>,
    Path((id, site)): Path<(String, String)>,
    ApiJson(body): ApiJson<UpdateSiteRequest>,
) -> ApiResult<Json<CompanyDetail>> {
    let store = state.storage.get();
    let detail =
        companies::update_site(store.as_ref(), &id, &site, body, state.clock.now()).await?;
    Ok(Json(detail))
}
