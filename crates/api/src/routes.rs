//! HTTP route handlers for Agency Service.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::NaiveDate;
use serde::Deserialize;
use uuid::Uuid;

use iaop_agency_service_application::commands;
use iaop_agency_service_application::commands::*;
use iaop_agency_service_application::error::AppError;
use iaop_agency_service_application::queries;
use iaop_agency_service_application::service_info::GetServiceInfoQuery;
use iaop_agency_service_domain::assignment::{AssignmentType, Transfer};
use iaop_agency_service_domain::entity_id::EntityId;
use iaop_agency_service_domain::org_unit::UnitKind;
use iaop_agency_service_infrastructure::persistence;
use utoipa::OpenApi;

use crate::auth::AuthPrincipal;
use crate::dto::{HealthResponse, ProblemDetails, SystemInfoResponse};
use crate::middleware;
use crate::openapi::ApiDoc;
use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route("/api/v1/system/info", get(system_info))
        .route("/api/docs/openapi.json", get(openapi_json))
        .route("/api/v1/agencies", post(create_agency).get(list_agencies))
        .route("/api/v1/agencies/{id}", get(get_agency))
        .route("/api/v1/agencies/{id}/lifecycle", post(transition_agency))
        .route(
            "/api/v1/organization-units",
            post(create_unit).get(list_units),
        )
        .route("/api/v1/organization-units/{id}", get(get_unit))
        .route(
            "/api/v1/organization-units/{id}/lifecycle",
            post(transition_unit),
        )
        .route(
            "/api/v1/organization-units/{id}/parent",
            axum::routing::patch(change_parent),
        )
        .route(
            "/api/v1/organization-units/{id}/ancestors",
            get(get_ancestors),
        )
        .route(
            "/api/v1/organization-units/{id}/descendants",
            get(get_descendants),
        )
        .route(
            "/api/v1/organization-units/{id}/assignments",
            get(unit_assignments),
        )
        .route("/api/v1/agent-assignments", post(assign_agent))
        .route(
            "/api/v1/agents/{agent_id}/assignments",
            get(agent_assignments),
        )
        .route(
            "/api/v1/agents/{agent_id}/current-assignment",
            get(agent_current_assignment),
        )
        .route(
            "/api/v1/agency-transfers",
            post(transfer_agent).get(list_transfers),
        )
        .route("/api/v1/leadership-appointments", post(assign_leader_route))
        .route(
            "/api/v1/leadership-appointments/{id}/end",
            axum::routing::post(end_leader_route),
        )
        .route("/api/v1/organization-units/{id}/leaders", get(get_leaders))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::request_context,
        ))
        .with_state(state)
}

async fn live() -> impl IntoResponse {
    (StatusCode::OK, Json(HealthResponse { status: "ok" }))
}

async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    match &state.db {
        Some(pool) => {
            if persistence::ping(pool).await {
                (StatusCode::OK, Json(HealthResponse { status: "ready" }))
            } else {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(HealthResponse { status: "degraded" }),
                )
            }
        }
        None => (StatusCode::OK, Json(HealthResponse { status: "ready" })),
    }
}

async fn system_info(State(state): State<AppState>) -> impl IntoResponse {
    let info = GetServiceInfoQuery {
        service_name: state.config.service_name.clone(),
        service_version: state.config.service_version.clone(),
        environment: state.config.environment.clone(),
    }
    .execute();
    Json(SystemInfoResponse {
        service: info.service,
        version: info.version,
        environment: info.environment,
        status: info.status,
    })
}

async fn openapi_json() -> impl IntoResponse {
    Json(ApiDoc::openapi())
}

fn store(
    state: &AppState,
) -> Result<&iaop_agency_service_infrastructure::org_store::PgOrgStoreWithPool, Response> {
    state
        .services
        .as_ref()
        .map(|s| &s.org_store)
        .ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error":"persistence not configured"})),
            )
                .into_response()
        })
}

fn parse_id(s: &str) -> Result<EntityId, Response> {
    s.parse::<Uuid>()
        .map(EntityId::from_uuid)
        .map_err(|_| bad_id())
}

fn bad_id() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error":"invalid id"})),
    )
        .into_response()
}

use axum::http::HeaderMap;
use iaop_agency_service_application::ports::{IdempotencyReserve, IdempotencyStore, Tx};
use std::time::Duration;

const IDEM_TTL: Duration = Duration::from_secs(24 * 3600);

/// Reserves an idempotency key inside `tx`. Returns:
/// - Ok(Some(record_id)) → caller owns key; must `idem_complete` before commit
/// - Ok(None) → replay: stored response already returned via `replay`
/// - Err(Response) → conflict or infra failure
async fn idem_begin(
    state: &AppState,
    tx: &mut Tx<'_>,
    principal: &iaop_agency_service_application::Principal,
    operation: &str,
    headers: &HeaderMap,
    fingerprint: &str,
) -> Result<Option<uuid::Uuid>, Response> {
    let Some(key) = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .filter(|k| !k.is_empty() && k.len() <= 255)
    else {
        return Ok(None);
    };
    let services = state.services.as_ref().ok_or_else(|| {
        respond(AppError::ServiceUnavailable(
            "persistence not configured".into(),
        ))
    })?;
    match services
        .idempotency
        .reserve(
            tx,
            &principal.subject,
            operation,
            key,
            fingerprint,
            IDEM_TTL,
        )
        .await
        .map_err(respond)?
    {
        IdempotencyReserve::Acquired { record_id } => Ok(Some(record_id)),
        IdempotencyReserve::Replay { status, body } => Err((
            axum::http::StatusCode::from_u16(status as u16).unwrap_or(StatusCode::OK),
            Json(body),
        )
            .into_response()),
        IdempotencyReserve::Conflict => Err(respond(AppError::IdempotencyConflict)),
    }
}

async fn idem_complete(
    state: &AppState,
    tx: &mut Tx<'_>,
    record_id: uuid::Uuid,
    status: i32,
    body: serde_json::Value,
) -> Result<(), AppError> {
    let services = state
        .services
        .as_ref()
        .ok_or_else(|| AppError::ServiceUnavailable("persistence not configured".into()))?;
    services
        .idempotency
        .complete(tx, record_id, status, body)
        .await
}

fn respond(e: AppError) -> Response {
    let (status, code) = match &e {
        AppError::Validation(_) => (StatusCode::BAD_REQUEST, "VALIDATION"),
        AppError::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND"),
        AppError::Conflict(_) | AppError::IdempotencyConflict => (StatusCode::CONFLICT, "CONFLICT"),
        AppError::Forbidden(_) => (StatusCode::FORBIDDEN, "FORBIDDEN"),
        AppError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "UNAUTHENTICATED"),
        AppError::ServiceUnavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "UNAVAILABLE"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL"),
    };
    (
        status,
        Json(serde_json::json!({
            "type": format!("https://api.iaop.internal/problems/{}", code.to_lowercase()),
            "title": code,
            "status": status.as_u16(),
            "code": code,
            "detail": e.to_string(),
        })),
    )
        .into_response()
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ListQuery {
    limit: Option<u32>,
    cursor: Option<Uuid>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct AsOfQuery {
    as_of: Option<NaiveDate>,
}

// ---- agencies ----

#[derive(Deserialize, serde::Serialize, utoipa::ToSchema)]
pub struct CreateAgencyBody {
    code: String,
    name: String,
}

#[utoipa::path(post, path = "/api/v1/agencies", tag = "agency", operation_id = "createAgency", summary = "Create agency", description = "Creates a top-level agency. Requires platform-administration role and global organizational scope.", request_body = CreateAgencyBody, responses((status = 201, description = "Agency created", body = AgencyDto),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn create_agency(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    headers: HeaderMap,
    Json(b): Json<CreateAgencyBody>,
) -> Response {
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let pool = match state.db.as_ref() {
        Some(p) => p.clone(),
        None => {
            return respond(AppError::ServiceUnavailable(
                "persistence not configured".into(),
            ));
        }
    };
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => return respond(AppError::infra(e)),
    };
    let fingerprint = serde_json::to_string(&b).unwrap_or_default();
    let idem = match idem_begin(&state, &mut tx, &p, "agency.create", &headers, &fingerprint).await
    {
        Ok(v) => v,
        Err(r) => return r,
    };
    match commands::create_agency(
        store,
        &mut tx,
        CreateAgency {
            code: b.code,
            name: b.name,
            principal: p.clone(),
        },
    )
    .await
    {
        Ok(dto) => {
            let body = serde_json::to_value(&dto).unwrap_or(serde_json::Value::Null);
            if let Some(rid) = idem {
                if let Err(e) = idem_complete(&state, &mut tx, rid, 201, body).await {
                    return respond(e);
                }
            }
            if let Err(e) = tx.commit().await {
                return respond(AppError::infra(e));
            }
            (StatusCode::CREATED, Json(dto)).into_response()
        }
        Err(e) => respond(e),
    }
}

#[utoipa::path(get, path = "/api/v1/agencies", tag = "agency", operation_id = "listAgencies", summary = "List agencies", description = "Returns agencies visible within the caller organizational scope.", params(("limit" = Option<u32>, Query, description = "Page size (max 200)"),("cursor" = Option<Uuid>, Query, description = "Pagination cursor")), responses((status = 200, description = "Agencies", body = [AgencyDto]),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn list_agencies(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    Query(q): Query<ListQuery>,
) -> Response {
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match queries::list_agencies(store, q.limit.unwrap_or(50), q.cursor, &p).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => respond(e),
    }
}

#[utoipa::path(get, path = "/api/v1/agencies/{id}", tag = "agency", operation_id = "getAgency", summary = "Get agency", description = "Returns a single agency if inside the caller organizational scope.", params(("id" = String, Path, description = "Agency ID")), responses((status = 200, description = "Agency", body = AgencyDto),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn get_agency(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    Path(id): Path<String>,
) -> Response {
    let id = match parse_id(&id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match queries::get_agency(store, id, &p).await {
        Ok(dto) => Json(dto).into_response(),
        Err(e) => respond(e),
    }
}

// ---- org units ----

#[derive(Deserialize, serde::Serialize, utoipa::ToSchema)]
pub struct CreateUnitBody {
    agency_id: String,
    code: String,
    name: String,
    kind: String,
    parent_id: Option<String>,
    effective_from: NaiveDate,
}

#[utoipa::path(post, path = "/api/v1/organization-units", tag = "agency", operation_id = "createUnit", summary = "Create organizational unit", description = "Creates a unit under an agency (optionally with initial parent). Requires hierarchy-manage permission and scope over the target agency.", request_body = CreateUnitBody, responses((status = 201, description = "Unit created", body = UnitDto),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn create_unit(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    headers: HeaderMap,
    Json(b): Json<CreateUnitBody>,
) -> Response {
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let fingerprint = serde_json::to_string(&b).unwrap_or_default();
    let agency_id = match parse_id(&b.agency_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let parent_id = match b.parent_id.as_deref().map(parse_id).transpose() {
        Ok(v) => v,
        Err(r) => return r,
    };
    let kind = match b.kind.to_uppercase().as_str() {
        "REGION" => UnitKind::Region,
        "DEPARTMENT" => UnitKind::Department,
        "TEAM" => UnitKind::Team,
        _ => UnitKind::Branch,
    };
    let cmd = CreateUnit {
        agency_id,
        code: b.code,
        name: b.name,
        kind,
        parent_id,
        effective_from: b.effective_from,
        principal: p.clone(),
    };
    let pool = match state.db.as_ref() {
        Some(p) => p.clone(),
        None => {
            return respond(AppError::ServiceUnavailable(
                "persistence not configured".into(),
            ));
        }
    };
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => return respond(AppError::infra(e)),
    };
    let idem = match idem_begin(
        &state,
        &mut tx,
        &p,
        "org_unit.create",
        &headers,
        &fingerprint,
    )
    .await
    {
        Ok(v) => v,
        Err(r) => return r,
    };
    match commands::create_unit(store, &mut tx, cmd).await {
        Ok(dto) => {
            let body = serde_json::to_value(&dto).unwrap_or(serde_json::Value::Null);
            if let Some(rid) = idem {
                if let Err(e) = idem_complete(&state, &mut tx, rid, 201, body).await {
                    return respond(e);
                }
            }
            if let Err(e) = tx.commit().await {
                return respond(AppError::infra(e));
            }
            (StatusCode::CREATED, Json(dto)).into_response()
        }
        Err(e) => respond(e),
    }
}

#[utoipa::path(get, path = "/api/v1/organization-units", tag = "agency", operation_id = "listUnits", summary = "List organizational units", description = "Returns units visible within the caller organizational scope.", params(("agency_id" = Option<Uuid>, Query, description = "Filter by agency"),("limit" = Option<u32>, Query),("cursor" = Option<Uuid>, Query)), responses((status = 200, description = "Units", body = [UnitDto]),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn list_units(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    Query(q): Query<ListQuery>,
) -> Response {
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match queries::list_units(store, None, q.limit.unwrap_or(50), q.cursor, &p).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => respond(e),
    }
}

#[utoipa::path(get, path = "/api/v1/organization-units/{id}", tag = "agency", operation_id = "getUnit", summary = "Get organizational unit", description = "Returns a single unit if inside the caller organizational scope.", params(("id" = String, Path, description = "Unit ID")), responses((status = 200, description = "Unit", body = UnitDto),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn get_unit(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    Path(id): Path<String>,
) -> Response {
    let id = match parse_id(&id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match queries::get_unit(store, id, &p).await {
        Ok(dto) => Json(dto).into_response(),
        Err(e) => respond(e),
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ChangeParentBody {
    new_parent_id: Option<String>,
    effective_from: NaiveDate,
    expected_version: i64,
}

#[utoipa::path(patch, path = "/api/v1/organization-units/{id}/parent", tag = "agency", operation_id = "changeUnitParent", summary = "Reparent organizational unit", description = "Changes a unit parent effective-dated; requires scope over the unit and optimistic concurrency via expected_version.", params(("id" = String, Path, description = "Unit ID")), request_body = ChangeParentBody, responses((status = 200, description = "Unit updated", body = UnitDto),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn change_parent(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    Path(id): Path<String>,
    Json(b): Json<ChangeParentBody>,
) -> Response {
    let id = match parse_id(&id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let new_parent = match b.new_parent_id.as_deref().map(parse_id).transpose() {
        Ok(v) => v,
        Err(r) => return r,
    };
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let cmd = ChangeParent {
        unit_id: id,
        new_parent_id: new_parent,
        effective_from: b.effective_from,
        expected_version: b.expected_version,
        principal: p.clone(),
    };
    let pool = match state.db.as_ref() {
        Some(p) => p.clone(),
        None => {
            return respond(AppError::ServiceUnavailable(
                "persistence not configured".into(),
            ));
        }
    };
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => return respond(AppError::infra(e)),
    };
    match commands::change_parent(store, &mut tx, cmd).await {
        Ok(dto) => {
            if let Err(e) = tx.commit().await {
                return respond(AppError::infra(e));
            }
            Json(dto).into_response()
        }
        Err(e) => respond(e),
    }
}

#[utoipa::path(get, path = "/api/v1/organization-units/{id}/ancestors", tag = "agency", operation_id = "getUnitAncestors", summary = "Unit ancestor chain", description = "Returns ancestor unit IDs as of a date (default today). Authorization is evaluated at the current date, never at as_of.", params(("id" = String, Path, description = "Unit ID"),("as_of" = Option<NaiveDate>, Query, description = "Historical date for data selection only")), responses((status = 200, description = "Ancestor unit IDs", body = [String]),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn get_ancestors(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    Path(id): Path<String>,
    Query(q): Query<AsOfQuery>,
) -> Response {
    let id = match parse_id(&id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match queries::ancestors(store, id, q.as_of, &p).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => respond(e),
    }
}

#[utoipa::path(get, path = "/api/v1/organization-units/{id}/descendants", tag = "agency", operation_id = "getUnitDescendants", summary = "Unit descendants", description = "Returns descendant unit IDs in the current hierarchy.", params(("id" = String, Path, description = "Unit ID")), responses((status = 200, description = "Descendant unit IDs", body = [String]),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn get_descendants(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    Path(id): Path<String>,
) -> Response {
    let id = match parse_id(&id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match queries::descendants(store, id, &p).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => respond(e),
    }
}

// ---- assignments ----

#[derive(Deserialize, serde::Serialize, utoipa::ToSchema)]
pub struct AssignBody {
    agent_id: String,
    unit_id: String,
    assignment_type: Option<String>,
    effective_from: NaiveDate,
}

#[utoipa::path(post, path = "/api/v1/agent-assignments", tag = "agency", operation_id = "assignAgent", summary = "Assign agent to unit", description = "Creates an agent assignment. Requires assignment-manage permission and scope over the destination unit; agent lifecycle is validated against Agent Service.", request_body = AssignBody, responses((status = 201, description = "Assignment created", body = AssignmentDto),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn assign_agent(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    headers: HeaderMap,
    Json(b): Json<AssignBody>,
) -> Response {
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let fingerprint = serde_json::to_string(&b).unwrap_or_default();
    let agent_id = match parse_id(&b.agent_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let unit_id = match parse_id(&b.unit_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let at = match b.assignment_type.as_deref().unwrap_or("PRIMARY") {
        "SECONDARY" => AssignmentType::Secondary,
        _ => AssignmentType::Primary,
    };
    let cmd = AssignAgent {
        agent_id,
        unit_id,
        assignment_type: at,
        effective_from: b.effective_from,
        principal: p.clone(),
    };
    let pool = match state.db.as_ref() {
        Some(p) => p.clone(),
        None => {
            return respond(AppError::ServiceUnavailable(
                "persistence not configured".into(),
            ));
        }
    };
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => return respond(AppError::infra(e)),
    };
    let idem = match idem_begin(
        &state,
        &mut tx,
        &p,
        "assignment.create",
        &headers,
        &fingerprint,
    )
    .await
    {
        Ok(v) => v,
        Err(r) => return r,
    };
    let res = if let Some(services) = state.services.as_ref() {
        commands::assign_agent_validated(store, &services.agent_validator, &mut tx, cmd).await
    } else {
        commands::assign_agent(store, &mut tx, cmd).await
    };
    match res {
        Ok(dto) => {
            let body = serde_json::to_value(&dto).unwrap_or(serde_json::Value::Null);
            if let Some(rid) = idem {
                if let Err(e) = idem_complete(&state, &mut tx, rid, 201, body).await {
                    return respond(e);
                }
            }
            if let Err(e) = tx.commit().await {
                return respond(AppError::infra(e));
            }
            (StatusCode::CREATED, Json(dto)).into_response()
        }
        Err(e) => respond(e),
    }
}

#[utoipa::path(get, path = "/api/v1/agents/{agent_id}/assignments", tag = "agency", operation_id = "agentAssignments", summary = "Agent assignments", description = "Assignments for an agent as of a date. Scoped callers receive only records inside their accessible units.", params(("agent_id" = String, Path, description = "Agent ID"),("as_of" = Option<NaiveDate>, Query)), responses((status = 200, description = "Assignments", body = [AssignmentDto]),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn agent_assignments(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    Path(agent_id): Path<String>,
    Query(q): Query<AsOfQuery>,
) -> Response {
    let agent_id = match parse_id(&agent_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match queries::agent_assignments_as_of(store, agent_id, q.as_of, &p).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => respond(e),
    }
}

#[utoipa::path(get, path = "/api/v1/agents/{agent_id}/current-assignment", tag = "agency", operation_id = "agentCurrentAssignment", summary = "Agent current assignment", description = "The agent current primary assignment.", params(("agent_id" = String, Path, description = "Agent ID")), responses((status = 200, description = "Current assignment", body = AssignmentDto),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn agent_current_assignment(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    Path(agent_id): Path<String>,
) -> Response {
    let agent_id = match parse_id(&agent_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match queries::agent_current_assignment(store, agent_id, &p).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => respond(e),
    }
}

#[utoipa::path(get, path = "/api/v1/organization-units/{id}/assignments", tag = "agency", operation_id = "unitAssignments", summary = "Unit assignments", description = "Agent assignments in a unit as of a date (default today).", params(("id" = String, Path, description = "Unit ID"),("as_of" = Option<NaiveDate>, Query)), responses((status = 200, description = "Assignments", body = [AssignmentDto]),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn unit_assignments(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    Path(id): Path<String>,
    Query(q): Query<AsOfQuery>,
) -> Response {
    let unit_id = match parse_id(&id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match queries::unit_assignments_as_of(store, unit_id, q.as_of, &p).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => respond(e),
    }
}

// ---- transfers ----

#[derive(Deserialize, serde::Serialize, utoipa::ToSchema)]
pub struct TransferBody {
    agent_id: String,
    to_unit_id: String,
    effective_from: NaiveDate,
    reason: Option<String>,
}

#[utoipa::path(post, path = "/api/v1/agency-transfers", tag = "agency", operation_id = "transferAgent", summary = "Transfer agent", description = "Immediately-effective transfer: ends the current primary assignment and creates the new one plus a transfer record. Caller must hold scope over BOTH source and destination units. Idempotency-Key supported.", request_body = TransferBody, responses((status = 201, description = "Transfer executed", body = AssignmentDto),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn transfer_agent(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    headers: HeaderMap,
    Json(b): Json<TransferBody>,
) -> Response {
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let fingerprint = serde_json::to_string(&b).unwrap_or_default();
    let agent_id = match parse_id(&b.agent_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let to_unit_id = match parse_id(&b.to_unit_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let cmd = TransferAgent {
        agent_id,
        to_unit_id,
        effective_from: b.effective_from,
        reason: b.reason,
        principal: p.clone(),
    };
    let pool = match state.db.as_ref() {
        Some(p) => p.clone(),
        None => {
            return respond(AppError::ServiceUnavailable(
                "persistence not configured".into(),
            ));
        }
    };
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => return respond(AppError::infra(e)),
    };
    let idem = match idem_begin(
        &state,
        &mut tx,
        &p,
        "agent.transfer",
        &headers,
        &fingerprint,
    )
    .await
    {
        Ok(v) => v,
        Err(r) => return r,
    };
    let res = if let Some(services) = state.services.as_ref() {
        commands::transfer_agent_validated(store, &services.agent_validator, &mut tx, cmd).await
    } else {
        commands::transfer_agent(store, &mut tx, cmd).await
    };
    match res {
        Ok((dto, _t)) => {
            let body = serde_json::to_value(&dto).unwrap_or(serde_json::Value::Null);
            if let Some(rid) = idem {
                if let Err(e) = idem_complete(&state, &mut tx, rid, 201, body).await {
                    return respond(e);
                }
            }
            if let Err(e) = tx.commit().await {
                return respond(AppError::infra(e));
            }
            (StatusCode::CREATED, Json(dto)).into_response()
        }
        Err(e) => respond(e),
    }
}

#[utoipa::path(get, path = "/api/v1/agency-transfers", tag = "agency", operation_id = "listTransfers", summary = "Transfer history", description = "Transfer records. Scoped callers may only view their own transfer history.", params(("agent_id" = Option<Uuid>, Query, description = "Filter by agent"),("limit" = Option<u32>, Query),("cursor" = Option<Uuid>, Query)), responses((status = 200, description = "Transfers", body = [Transfer]),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn list_transfers(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    Query(q): Query<ListQuery>,
) -> Response {
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match queries::transfer_history(store, None, q.limit.unwrap_or(50), &p).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => respond(e),
    }
}

// ---- leadership ----

#[derive(Deserialize, serde::Serialize, utoipa::ToSchema)]
pub struct AssignLeaderBody {
    unit_id: String,
    agent_id: String,
    role: String,
    effective_from: NaiveDate,
}

#[utoipa::path(post, path = "/api/v1/leadership-appointments", tag = "agency", operation_id = "assignLeader", summary = "Assign unit leader", description = "Creates an effective-dated leadership appointment. Requires leadership-manage permission and scope over the unit.", request_body = AssignLeaderBody, responses((status = 201, description = "Appointment created", body = LeadershipDto),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn assign_leader_route(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    headers: HeaderMap,
    Json(b): Json<AssignLeaderBody>,
) -> Response {
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let unit_id = match parse_id(&b.unit_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let agent_id = match parse_id(&b.agent_id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let role =
        match iaop_agency_service_domain::leadership::LeadershipRole::parse(&b.role.to_uppercase())
        {
            Some(r) => r,
            None => return respond(AppError::Validation("invalid role".into())),
        };
    let fingerprint = serde_json::to_string(&b).unwrap_or_default();
    let pool = match state.db.as_ref() {
        Some(p) => p.clone(),
        None => {
            return respond(AppError::ServiceUnavailable(
                "persistence not configured".into(),
            ));
        }
    };
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => return respond(AppError::infra(e)),
    };
    let idem = match idem_begin(
        &state,
        &mut tx,
        &p,
        "leadership.assign",
        &headers,
        &fingerprint,
    )
    .await
    {
        Ok(v) => v,
        Err(r) => return r,
    };
    let cmd = commands::AssignLeader {
        unit_id,
        agent_id,
        role,
        effective_from: b.effective_from,
        principal: p.clone(),
    };
    match commands::assign_leader(store, &mut tx, cmd).await {
        Ok(dto) => {
            let body = serde_json::to_value(&dto).unwrap_or(serde_json::Value::Null);
            if let Some(rid) = idem {
                if let Err(e) = idem_complete(&state, &mut tx, rid, 201, body).await {
                    return respond(e);
                }
            }
            if let Err(e) = tx.commit().await {
                return respond(AppError::infra(e));
            }
            (StatusCode::CREATED, Json(dto)).into_response()
        }
        Err(e) => respond(e),
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct EndLeaderBody {
    effective_to: NaiveDate,
    expected_version: i64,
}

#[utoipa::path(post, path = "/api/v1/leadership-appointments/{id}/end", tag = "agency", operation_id = "endLeader", summary = "End leadership appointment", description = "Ends an appointment effective-dated with optimistic concurrency.", params(("id" = String, Path, description = "Appointment ID")), request_body = EndLeaderBody, responses((status = 200, description = "Appointment ended", body = LeadershipDto),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn end_leader_route(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    Path(id): Path<String>,
    Json(b): Json<EndLeaderBody>,
) -> Response {
    let id = match parse_id(&id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let pool = match state.db.as_ref() {
        Some(p) => p.clone(),
        None => {
            return respond(AppError::ServiceUnavailable(
                "persistence not configured".into(),
            ));
        }
    };
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => return respond(AppError::infra(e)),
    };
    let cmd = commands::EndLeader {
        appointment_id: id,
        effective_to: b.effective_to,
        expected_version: b.expected_version,
        principal: p,
    };
    match commands::end_leader(store, &mut tx, cmd).await {
        Ok(dto) => {
            if let Err(e) = tx.commit().await {
                return respond(AppError::infra(e));
            }
            Json(dto).into_response()
        }
        Err(e) => respond(e),
    }
}

#[utoipa::path(get, path = "/api/v1/organization-units/{id}/leaders", tag = "agency", operation_id = "unitLeaders", summary = "Unit leaders", description = "Leadership appointments for a unit as of a date (default today).", params(("id" = String, Path, description = "Unit ID"),("as_of" = Option<NaiveDate>, Query)), responses((status = 200, description = "Leadership appointments", body = [LeadershipDto]),(status = 400, description = "Malformed request", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn get_leaders(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    Path(id): Path<String>,
    Query(q): Query<AsOfQuery>,
) -> Response {
    let id = match parse_id(&id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match queries::leaders_as_of(store, id, q.as_of, &p).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => respond(e),
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct AgencyLifecycleBody {
    /// SUSPEND | REINSTATE | TERMINATE
    action: String,
    expected_version: i64,
}

#[utoipa::path(post, path = "/api/v1/agencies/{id}/lifecycle", tag = "agency", operation_id = "transitionAgency", summary = "Transition agency lifecycle", description = "Applies an approved agency lifecycle transition (SUSPEND, REINSTATE, TERMINATE). Requires platform-administration global write scope and optimistic concurrency via expected_version. TERMINATED is terminal; no cascade to units, assignments, or leadership.", params(("id" = String, Path, description = "Agency ID")), request_body = AgencyLifecycleBody, responses((status = 200, description = "Agency updated", body = AgencyDto),(status = 400, description = "Malformed request or unknown action", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Invalid transition or version conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn transition_agency(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    Path(id): Path<String>,
    Json(b): Json<AgencyLifecycleBody>,
) -> Response {
    let id = match parse_id(&id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let action = match b.action.to_uppercase().as_str() {
        "SUSPEND" => commands::AgencyLifecycleAction::Suspend,
        "REINSTATE" => commands::AgencyLifecycleAction::Reinstate,
        "TERMINATE" => commands::AgencyLifecycleAction::Terminate,
        _ => return respond(AppError::Validation("unknown lifecycle action".into())),
    };
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let cmd = commands::TransitionAgency {
        agency_id: id,
        action,
        expected_version: b.expected_version,
        principal: p.clone(),
    };
    let pool = match state.db.as_ref() {
        Some(p) => p.clone(),
        None => {
            return respond(AppError::ServiceUnavailable(
                "persistence not configured".into(),
            ));
        }
    };
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => return respond(AppError::infra(e)),
    };
    match commands::transition_agency(store, &mut tx, cmd).await {
        Ok(dto) => {
            if let Err(e) = tx.commit().await {
                return respond(AppError::infra(e));
            }
            Json(dto).into_response()
        }
        Err(e) => respond(e),
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UnitLifecycleBody {
    /// ACTIVATE | DEACTIVATE
    action: String,
    expected_version: i64,
}

#[utoipa::path(post, path = "/api/v1/organization-units/{id}/lifecycle", tag = "agency", operation_id = "transitionUnit", summary = "Transition organizational unit lifecycle", description = "Activates or deactivates a unit. Requires hierarchy-manage permission and scope over the unit plus optimistic concurrency via expected_version. Deactivation preserves all relationships, assignments, leadership, and history.", params(("id" = String, Path, description = "Unit ID")), request_body = UnitLifecycleBody, responses((status = 200, description = "Unit updated", body = UnitDto),(status = 400, description = "Malformed request or unknown action", body = ProblemDetails),(status = 401, description = "Unauthenticated", body = ProblemDetails),(status = 403, description = "Outside organizational scope or missing permission", body = ProblemDetails),(status = 404, description = "Not found", body = ProblemDetails),(status = 409, description = "Invalid transition or version conflict", body = ProblemDetails),(status = 503, description = "Dependency unavailable", body = ProblemDetails)), security(("bearerAuth" = [])))]
pub(crate) async fn transition_unit(
    State(state): State<AppState>,
    AuthPrincipal(p): AuthPrincipal,
    Path(id): Path<String>,
    Json(b): Json<UnitLifecycleBody>,
) -> Response {
    let id = match parse_id(&id) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let action = match b.action.to_uppercase().as_str() {
        "ACTIVATE" => commands::UnitLifecycleAction::Activate,
        "DEACTIVATE" => commands::UnitLifecycleAction::Deactivate,
        _ => return respond(AppError::Validation("unknown lifecycle action".into())),
    };
    let store = match store(&state) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let cmd = commands::TransitionUnit {
        unit_id: id,
        action,
        expected_version: b.expected_version,
        principal: p.clone(),
    };
    let pool = match state.db.as_ref() {
        Some(p) => p.clone(),
        None => {
            return respond(AppError::ServiceUnavailable(
                "persistence not configured".into(),
            ));
        }
    };
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => return respond(AppError::infra(e)),
    };
    match commands::transition_unit(store, &mut tx, cmd).await {
        Ok(dto) => {
            if let Err(e) = tx.commit().await {
                return respond(AppError::infra(e));
            }
            Json(dto).into_response()
        }
        Err(e) => respond(e),
    }
}
