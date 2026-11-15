//! Authentication extractor: validates Bearer JWT via configured
//! TokenValidatorImpl (JWKS or gated dev bypass).

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};

use iaop_agency_service_application::Principal;
use iaop_agency_service_application::ports::{AgentValidationPort, TokenValidator};

use crate::state::AppState;

pub struct AuthPrincipal(pub Principal);

impl FromRequestParts<AppState> for AuthPrincipal {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let services = state.services.as_ref().ok_or_else(service_unavailable)?;
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(unauthorized)?;
        let mut principal = services
            .token_validator
            .validate(token)
            .await
            .map_err(|_| unauthorized())?;

        // Server-side identity resolution: when the token carries no
        // agent_scope/agent_id claim, resolve the caller's OWN
        // (issuer, subject) against Agent Service identity links.
        // Global staff roles are exempt (they are not agents).
        // Failure to resolve leaves agent_scope unset -> downstream
        // authorization fails closed (403). Registry outages surface as 503.
        if principal.resolved_agent_id().is_none()
            && !iaop_agency_service_application::authz::is_staff(&principal)
        {
            match services
                .agent_validator
                .resolve_agent_identity(&principal.issuer, &principal.subject)
                .await
            {
                Ok(Some(agent_id)) => {
                    principal.agent_scope = Some(agent_id.to_string());
                    tracing::debug!(agent_id = %agent_id, "resolved agent identity via Agent Service");
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "agent identity resolution failed");
                    return Err(service_unavailable());
                }
            }
        }

        Ok(AuthPrincipal(principal))
    }
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(axum::http::header::WWW_AUTHENTICATE, "Bearer")],
        r#"{"type":"https://api.iaop.internal/problems/unauthenticated","title":"Unauthenticated","status":401,"code":"UNAUTHENTICATED"}"#,
    )
        .into_response()
}

fn service_unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        r#"{"type":"https://api.iaop.internal/problems/service_unavailable","title":"Service unavailable","status":503,"code":"SERVICE_UNAVAILABLE"}"#,
    )
        .into_response()
}
