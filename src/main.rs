//! Composition root for Agent Lifecycle.
//!
//! Wires configuration, persistence, domain services, the HTTP router,
//! and the outbox publisher, then serves with graceful shutdown on
//! SIGINT/SIGTERM.

use std::sync::Arc;

use anyhow::Context;
use iaop_agency_service_api::state::ServicePorts;
use iaop_agency_service_api::{AppState, build_router};
use iaop_agency_service_infrastructure::config::AppConfig;
use iaop_agency_service_infrastructure::idempotency::PgIdempotencyStore;
use iaop_agency_service_infrastructure::jwt::{DevBypassValidator, JwksTokenValidator};
use iaop_agency_service_infrastructure::org_store::PgOrgStoreWithPool;
use iaop_agency_service_infrastructure::outbox;
use iaop_agency_service_infrastructure::persistence;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = AppConfig::from_env().context("failed to load configuration")?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .json()
        .init();

    tracing::info!(config = %config.redacted_summary(), "starting service");

    let db = match &config.database_url {
        Some(url) => {
            let pool = persistence::connect_pool(url, config.database_max_connections)
                .await
                .context("failed to connect to database")?;
            persistence::run_migrations(&pool)
                .await
                .context("failed to run migrations")?;
            tracing::info!("database migrations applied");
            Some(pool)
        }
        None => {
            tracing::warn!("DATABASE_URL not set; running without persistence");
            None
        }
    };

    // ---- wire domain services -------------------------------------------
    let services = db.as_ref().map(|pool| {
        let validator = match config.auth_mode {
            iaop_agency_service_infrastructure::config::AuthMode::DevBypass => {
                iaop_agency_service_infrastructure::jwt::TokenValidatorImpl::Dev(DevBypassValidator)
            }
            _ => iaop_agency_service_infrastructure::jwt::TokenValidatorImpl::Jwks(
                JwksTokenValidator::new(
                    config.oidc_issuer.clone().unwrap_or_default(),
                    config.oidc_jwks_url.clone().unwrap_or_else(|| {
                        format!(
                            "{}/protocol/openid-connect/certs",
                            config.oidc_issuer.clone().unwrap_or_default()
                        )
                    }),
                    config.oidc_audience.clone().unwrap_or_default(),
                ),
            ),
        };

        let agent_validator = if let Some(url) = &config.agent_service_url {
            iaop_agency_service_infrastructure::agent_validation::AgentValidatorImpl::Http(
                iaop_agency_service_infrastructure::agent_validation::HttpAgentValidator::with_credentials(
                    url.clone(),
                    config.oidc_issuer.clone(),
                    config.agent_service_client_id.clone(),
                    config.agent_service_client_secret.clone(),
                ),
            )
        } else if config.environment.eq_ignore_ascii_case("local")
            && config.auth_mode == iaop_agency_service_infrastructure::config::AuthMode::DevBypass
        {
            tracing::warn!("using DevAgentValidator in Local/DevBypass mode");
            iaop_agency_service_infrastructure::agent_validation::AgentValidatorImpl::Dev(
                iaop_agency_service_infrastructure::agent_validation::DevAgentValidator,
            )
        } else {
            tracing::warn!("AGENT_SERVICE_URL unset; using FailClosedAgentValidator");
            iaop_agency_service_infrastructure::agent_validation::AgentValidatorImpl::FailClosed(
                iaop_agency_service_infrastructure::agent_validation::FailClosedAgentValidator,
            )
        };

        Arc::new(ServicePorts {
            org_store: PgOrgStoreWithPool { pool: pool.clone() },
            idempotency: PgIdempotencyStore::new(),
            token_validator: validator,
            agent_validator,
        })
    });

    // ---- outbox publisher (only with DB) --------------------------------
    if let Some(pool) = &db {
        let publisher_pool = pool.clone();
        let transport = Arc::new(match config.event_transport {
            iaop_agency_service_infrastructure::config::EventTransportKind::Kafka => {
                let brokers = config
                    .kafka_brokers
                    .clone()
                    .unwrap_or_else(|| "127.0.0.1:9092".to_string());
                iaop_agency_service_infrastructure::transport::ConfiguredTransport::Kafka(
                    iaop_agency_service_infrastructure::transport::KafkaTransport::new(
                        brokers,
                        config.kafka_topic_prefix.clone(),
                    ),
                )
            }
            iaop_agency_service_infrastructure::config::EventTransportKind::Log => {
                iaop_agency_service_infrastructure::transport::ConfiguredTransport::Log(
                    iaop_agency_service_infrastructure::transport::LogTransport,
                )
            }
        });
        let poll = config.outbox_poll_interval;
        let batch = config.outbox_batch_size;
        let max_attempts = config.outbox_max_attempts;
        tokio::spawn(async move {
            let shutdown = shutdown_signal();
            outbox::run_publisher(
                publisher_pool,
                transport,
                poll,
                batch,
                max_attempts,
                shutdown,
            )
            .await;
        });
        tracing::info!("outbox publisher started");
    }

    let state = AppState {
        config: Arc::new(config.clone()),
        db,
        services,
    };
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(&config.bind_addr)
        .await
        .with_context(|| format!("failed to bind {}", config.bind_addr))?;
    tracing::info!(addr = %config.bind_addr, "listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;

    tracing::info!("shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
