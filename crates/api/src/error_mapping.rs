//! RFC 9457 Problem Details error mapping.
//!
//! Never expose stack traces, SQL errors, or internal details to clients.

use axum::http::StatusCode;
use iaop_agency_service_application::error::AppError;
use uuid::Uuid;

use iaop_agency_service_domain::error::DomainError;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub title: &'static str,
    pub detail: String,
    pub correlation_id: Option<Uuid>,
}

impl ApiError {
    pub fn new(
        status: StatusCode,
        code: &'static str,
        title: &'static str,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            status,
            code,
            title,
            detail: detail.into(),
            correlation_id: None,
        }
    }

    pub fn validation(detail: impl Into<String>) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "VALIDATION_ERROR",
            "Validation failed",
            detail,
        )
    }

    pub fn not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "Resource not found",
            "The requested resource does not exist",
        )
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        // Detail is logged server-side; clients get a generic message.
        tracing::error!(error = %detail.into(), "internal error");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            "Internal error",
            "An unexpected error occurred",
        )
    }

    pub fn with_correlation(mut self, id: Option<Uuid>) -> Self {
        self.correlation_id = id;
        self
    }
}

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        match e {
            AppError::Domain(d) => domain_error(d),
            AppError::AgentNotFound => ApiError::not_found(),
            AppError::Conflict(msg) => {
                ApiError::new(StatusCode::CONFLICT, "CONFLICT", "Conflict", msg)
            }
            AppError::PreconditionFailed(msg) => ApiError::new(
                StatusCode::PRECONDITION_FAILED,
                "PRECONDITION_FAILED",
                "Precondition failed",
                msg,
            ),
            AppError::IdempotencyConflict => ApiError::new(
                StatusCode::CONFLICT,
                "IDEMPOTENCY_CONFLICT",
                "Idempotency conflict",
                "Idempotency-Key was already used with a different request",
            ),
            AppError::Forbidden(msg) => {
                ApiError::new(StatusCode::FORBIDDEN, "FORBIDDEN", "Forbidden", msg)
            }
            AppError::NotFound(m)
            | AppError::Validation(m)
            | AppError::Unauthorized(m)
            | AppError::ServiceUnavailable(m) => ApiError::validation(m.clone()),
            AppError::Infrastructure(err) => {
                tracing::error!(error = %err, "infrastructure error");
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL_ERROR",
                    "Internal error",
                    "An unexpected error occurred",
                )
            }
        }
    }
}

fn domain_error(e: DomainError) -> ApiError {
    match e {
        DomainError::NotFound => ApiError::not_found(),
        DomainError::Validation(_) => ApiError::validation(e.to_string()),
        DomainError::Conflict(_) | DomainError::InvalidState => {
            ApiError::new(StatusCode::CONFLICT, "CONFLICT", "Conflict", e.to_string())
        }
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            axum::Json(serde_json::json!({
                "type": format!("https://api.iaop.internal/problems/{}", self.code.to_lowercase()),
                "title": self.title,
                "status": self.status.as_u16(),
                "code": self.code,
                "detail": self.detail,
            })),
        )
            .into_response()
    }
}
