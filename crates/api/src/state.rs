use std::sync::Arc;

use iaop_agency_service_application::ports::{IdempotencyStore, OrgStore, TokenValidator};
use iaop_agency_service_infrastructure::config::AppConfig;
use iaop_agency_service_infrastructure::idempotency::PgIdempotencyStore;
use iaop_agency_service_infrastructure::jwt::TokenValidatorImpl;
use iaop_agency_service_infrastructure::org_store::PgOrgStoreWithPool;
use sqlx::PgPool;

/// Shared application state injected into handlers via `State`.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    /// `None` when DATABASE_URL is not configured; `/health/ready`
    /// reports accordingly instead of failing.
    pub db: Option<PgPool>,
    /// Wired service ports. `None` when no database is configured —
    /// agency endpoints return 503 in that case.
    pub services: Option<Arc<ServicePorts>>,
}

/// Concrete port implementations wired at startup.
pub struct ServicePorts {
    pub org_store: PgOrgStoreWithPool,
    pub idempotency: PgIdempotencyStore,
    pub token_validator: TokenValidatorImpl,
}

// Trait bounds kept for documentation; ports are used via concrete types.
#[allow(dead_code)]
fn _assert_impls(p: &ServicePorts) {
    fn _is_store<T: OrgStore>(_: &T) {}
    fn _is_idem<T: IdempotencyStore>(_: &T) {}
    fn _is_tok<T: TokenValidator>(_: &T) {}
    _is_store(&p.org_store);
    _is_idem(&p.idempotency);
    _is_tok(&p.token_validator);
}
