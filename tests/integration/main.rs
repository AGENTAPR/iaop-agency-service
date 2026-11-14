//! Smoke integration test: the router serves health and info endpoints
//! without a database.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use iaop_agency_service_api::{AppState, build_router};
use iaop_agency_service_infrastructure::config::AppConfig;
use std::sync::Arc;

fn test_state() -> AppState {
    // Minimal env for AppConfig::from_env in tests.
    unsafe {
        for (k, v) in [
            ("SERVICE_NAME", "iaop-agency-service"),
            ("SERVICE_VERSION", "0.1.0"),
            ("ENVIRONMENT", "test"),
            ("AUTH_MODE", "dev"),
            ("ACTIVATION_MODE", "fixture"),
            ("BIND_ADDR", "127.0.0.1:0"),
        ] {
            std::env::set_var(k, v);
        }
    }
    AppState {
        config: Arc::new(AppConfig::from_env().expect("test config must load")),
        db: None,
        services: None,
    }
}

#[tokio::test]
async fn health_live_returns_ok() {
    let app = build_router(test_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn system_info_returns_metadata() {
    let app = build_router(test_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/system/info")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
