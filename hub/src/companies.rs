#![cfg(feature = "ssr")]
//! HTTP handlers for companies and sites (D-44). The rules are in
//! `service::companies`; access per route: `crate::routes`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};

use crate::AppState;
use crate::auth::{CurrentUser, RequestInfo};
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::model::Role;
use crate::model::{
    ArchiveCompanyRequest, CompanyDetail, CompanyInfo, CreateCompanyRequest, CreateSiteRequest,
    DiscoveryRequest, UpdateCompanyRequest, UpdateSiteRequest,
};
use crate::service::audit::{self, Actor, Event, action};
use crate::service::companies::{self, CompanyError};
use crate::service::scope;

/// What `me` may see of a company: Clients don't get internal notes and
/// contacts (D-45).
fn visible_to(me: &CurrentUser, detail: CompanyDetail) -> CompanyDetail {
    if me.user.role == Role::Client {
        detail.without_internal_fields()
    } else {
        detail
    }
}

impl From<CompanyError> for ApiError {
    fn from(error: CompanyError) -> Self {
        match error {
            CompanyError::NotFound | CompanyError::SiteNotFound => {
                ApiError::not_found(error.to_string())
            }
            CompanyError::EmptyName => {
                ApiError::new(StatusCode::BAD_REQUEST, "invalid_name", error.to_string())
            }
            CompanyError::BadTimezone(_) => ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_timezone",
                error.to_string(),
            ),
            CompanyError::Storage(e) => ApiError::internal("company operation", e),
        }
    }
}

/// `GET /api/companies`: all companies, archived ones included.
pub async fn list(
    State(state): State<AppState>,
    me: CurrentUser,
) -> ApiResult<Json<Vec<CompanyInfo>>> {
    let mut list = state
        .storage
        .get()
        .companies(&me.user.effective_scope())
        .await
        .map_err(|e| ApiError::internal("list companies", e))?;
    if me.user.role == Role::Client {
        list = list
            .into_iter()
            .map(CompanyInfo::without_internal_fields)
            .collect();
    }
    Ok(Json(list))
}

/// `POST /api/companies`: create a company (Admins).
pub async fn create(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    ApiJson(body): ApiJson<CreateCompanyRequest>,
) -> ApiResult<Json<CompanyDetail>> {
    let store = state.storage.get();
    let now = state.clock.now();
    crate::service::probes::create_company(store.as_ref(), &body.id, &body.name, now).await?;
    let event = Event::new(action::COMPANY_CREATE)
        .target("company", &body.id)
        .company(&body.id)
        .details(serde_json::json!({ "name": body.name }));
    audit::record(
        store.as_ref(),
        &Actor::user(&me.user, info.client_ip),
        event,
        now,
    )
    .await;
    Ok(Json(companies::company(store.as_ref(), &body.id).await?))
}

/// `POST /api/companies/{id}/sites`: create a site of one of the user's companies.
pub async fn create_site(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<CreateSiteRequest>,
) -> ApiResult<Json<CompanyDetail>> {
    let store = state.storage.get();
    let now = state.clock.now();
    let scope = me.user.effective_scope();
    crate::service::probes::create_site(store.as_ref(), &scope, &id, &body.id, &body.name, now)
        .await?;
    let event = Event::new(action::SITE_CREATE)
        .target("site", &body.id)
        .company(&id)
        .details(serde_json::json!({ "name": body.name }));
    audit::record(
        store.as_ref(),
        &Actor::user(&me.user, info.client_ip),
        event,
        now,
    )
    .await;
    let detail = companies::company(store.as_ref(), &id).await?;
    Ok(Json(visible_to(&me, detail)))
}

/// `GET /api/companies/{id}`: a company and its sites.
pub async fn detail(
    State(state): State<AppState>,
    me: CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<CompanyDetail>> {
    scope::require(&me.user.effective_scope(), &id)?;
    let detail = companies::company(state.storage.get().as_ref(), &id).await?;
    Ok(Json(visible_to(&me, detail)))
}

/// `POST /api/companies/{id}`: edit name, contacts and notes.
pub async fn update(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateCompanyRequest>,
) -> ApiResult<Json<CompanyDetail>> {
    scope::require(&me.user.effective_scope(), &id)?;
    let store = state.storage.get();
    let details = serde_json::to_value(&body).unwrap_or_default();
    let detail = companies::update_company(store.as_ref(), &id, body, state.clock.now()).await?;
    let event = Event::new(action::COMPANY_UPDATE)
        .target("company", &id)
        .company(&id)
        .details(details);
    let actor = Actor::user(&me.user, info.client_ip);
    audit::record(store.as_ref(), &actor, event, state.clock.now()).await;
    Ok(Json(detail))
}

/// `POST /api/companies/{id}/archive`: `{"archived": bool}`.
pub async fn archive(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ArchiveCompanyRequest>,
) -> ApiResult<Json<CompanyDetail>> {
    scope::require(&me.user.effective_scope(), &id)?;
    let store = state.storage.get();
    let detail =
        companies::set_archived(store.as_ref(), &id, body.archived, state.clock.now()).await?;
    let what = if body.archived {
        action::COMPANY_ARCHIVE
    } else {
        action::COMPANY_UNARCHIVE
    };
    let event = Event::new(what).target("company", &id).company(&id);
    let actor = Actor::user(&me.user, info.client_ip);
    audit::record(store.as_ref(), &actor, event, state.clock.now()).await;
    Ok(Json(detail))
}

/// `POST /api/companies/{id}/discovery`: `{"allowed": bool}`. Whether the
/// company's probes may look at their networks when asked (D-101).
pub async fn set_discovery(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<DiscoveryRequest>,
) -> ApiResult<Json<CompanyDetail>> {
    scope::require(&me.user.effective_scope(), &id)?;
    let store = state.storage.get();
    let detail = crate::service::discovery::set_allowed(store.as_ref(), &id, body.allowed).await?;
    let event = Event::new(action::COMPANY_DISCOVERY)
        .target("company", &id)
        .company(&id)
        .details(serde_json::json!({ "allowed": body.allowed }));
    let actor = Actor::user(&me.user, info.client_ip);
    audit::record(store.as_ref(), &actor, event, state.clock.now()).await;
    Ok(Json(detail))
}

/// `POST /api/companies/{id}/sites/{site}`: edit a site's name, address and notes.
pub async fn update_site(
    State(state): State<AppState>,
    info: RequestInfo,
    me: CurrentUser,
    Path((id, site)): Path<(String, String)>,
    ApiJson(body): ApiJson<UpdateSiteRequest>,
) -> ApiResult<Json<CompanyDetail>> {
    scope::require(&me.user.effective_scope(), &id)?;
    let store = state.storage.get();
    let details = serde_json::to_value(&body).unwrap_or_default();
    let detail =
        companies::update_site(store.as_ref(), &id, &site, body, state.clock.now()).await?;
    let event = Event::new(action::SITE_UPDATE)
        .target("site", &site)
        .company(&id)
        .details(details);
    let actor = Actor::user(&me.user, info.client_ip);
    audit::record(store.as_ref(), &actor, event, state.clock.now()).await;
    Ok(Json(detail))
}
