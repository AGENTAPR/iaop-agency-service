//! Authentication extractor: validates Bearer JWT via configured
//! TokenValidatorImpl (JWKS or gated dev bypass).

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};

use iaop_agency_service_application::Principal;
use iaop_agency_service_application::ports::TokenValidator;

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
        let principal = services
            .token_validator
            .validate(token)
            .await
            .map_err(|_| unauthorized())?;
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
