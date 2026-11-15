//! Authoritative Agent Registry validation adapter for Agency Service.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use std::collections::HashMap;

use serde::Deserialize;
use uuid::Uuid;

use iaop_agency_service_application::error::AppError;
use iaop_agency_service_application::ports::AgentValidationPort;
use iaop_agency_service_domain::entity_id::EntityId;

#[derive(Deserialize)]
struct AgentResponse {
    // Agent Service serializes DTOs in snake_case.
    lifecycle_status: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<u64>,
}

struct CachedToken {
    token: String,
    expires_at: Instant,
}

#[derive(Deserialize)]
struct IdentityLinkResponse {
    agent_id: Uuid,
    status: String,
    unlinked_at: Option<serde_json::Value>,
}

/// HTTP client calling `iaop-agent-service` to verify agent identity and status.
///
/// Agent Service requires a Bearer token; when service-account credentials
/// are configured the validator obtains a `client_credentials` token from
/// the OIDC issuer and caches it until shortly before expiry. Without
/// credentials the request is sent unauthenticated and Agent Service
/// denies it — validation fails closed, never silently succeeds.
pub struct HttpAgentValidator {
    client: reqwest::Client,
    base_url: String,
    token_url: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    cached: Mutex<Option<CachedToken>>,
    /// Bounded TTL cache for issuer+subject -> agent identity resolution.
    /// Identity unlink propagates within IDENTITY_CACHE_TTL seconds.
    identity_cache: Mutex<HashMap<(String, String), (Option<EntityId>, Instant)>>,
}

impl HttpAgentValidator {
    pub fn new(base_url: String) -> Self {
        Self::with_credentials(base_url, None, None, None)
    }

    pub fn with_credentials(
        base_url: String,
        issuer: Option<String>,
        client_id: Option<String>,
        client_secret: Option<String>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap_or_default();
        let token_url =
            issuer.map(|i| format!("{}/protocol/openid-connect/token", i.trim_end_matches('/')));
        Self {
            client,
            base_url,
            token_url,
            client_id,
            client_secret,
            cached: Mutex::new(None),
            identity_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Returns a cached-or-fresh service token. `Ok(None)` means no
    /// credentials are configured (request goes out unauthenticated).
    async fn service_token(&self) -> Result<Option<String>, AppError> {
        let (Some(token_url), Some(client_id), Some(client_secret)) = (
            self.token_url.as_deref(),
            self.client_id.as_deref(),
            self.client_secret.as_deref(),
        ) else {
            return Ok(None);
        };

        if let Ok(guard) = self.cached.lock() {
            if let Some(t) = guard.as_ref() {
                if Instant::now() < t.expires_at {
                    return Ok(Some(t.token.clone()));
                }
            }
        }

        let resp = self
            .client
            .post(token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", client_id),
                ("client_secret", client_secret),
            ])
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "service token request failed");
                AppError::ServiceUnavailable("identity provider unavailable".into())
            })?;
        if !resp.status().is_success() {
            tracing::warn!(status = %resp.status(), "service token request rejected");
            return Err(AppError::ServiceUnavailable(
                "service credentials rejected by identity provider".into(),
            ));
        }
        let body: TokenResponse = resp
            .json()
            .await
            .map_err(|e| AppError::ServiceUnavailable(format!("invalid token response: {e}")))?;
        let ttl = body.expires_in.unwrap_or(60).saturating_sub(30).max(1);
        if let Ok(mut guard) = self.cached.lock() {
            *guard = Some(CachedToken {
                token: body.access_token.clone(),
                expires_at: Instant::now() + Duration::from_secs(ttl),
            });
        }
        Ok(Some(body.access_token))
    }
}

impl AgentValidationPort for HttpAgentValidator {
    async fn validate_agent_active(&self, agent_id: EntityId) -> Result<(), AppError> {
        let token = self.service_token().await?;

        let url = format!("{}/api/v1/agents/{}", self.base_url, agent_id.as_uuid());
        let mut req = self.client.get(&url);
        if let Some(t) = &token {
            req = req.bearer_auth(t);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "agent validation request failed; failing closed");
                return Err(AppError::ServiceUnavailable(
                    "agent registry service unavailable".into(),
                ));
            }
        };

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(AppError::NotFound("agent not found in registry".into()));
        }

        if !resp.status().is_success() {
            return Err(AppError::ServiceUnavailable(format!(
                "agent registry returned {}",
                resp.status()
            )));
        }

        let agent: AgentResponse = resp.json().await.map_err(|e| {
            AppError::ServiceUnavailable(format!("invalid agent registry response: {e}"))
        })?;

        if agent.lifecycle_status != "ACTIVE" {
            return Err(AppError::Conflict(format!(
                "agent lifecycle status is {} — assignment prohibited",
                agent.lifecycle_status
            )));
        }

        Ok(())
    }
    /// Server-side identity resolution (Option B): maps the caller's own
    /// (issuer, subject) to an Agent Registry ID via Agent Service
    /// `GET /api/v1/identity-links/resolve`. Results are cached for 60s;
    /// revocation propagates within that bound. Transport failures return
    /// `ServiceUnavailable` — never a fabricated identity.
    async fn resolve_agent_identity(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<EntityId>, AppError> {
        const TTL: Duration = Duration::from_secs(60);
        let key = (issuer.to_string(), subject.to_string());
        if let Ok(guard) = self.identity_cache.lock() {
            if let Some((v, exp)) = guard.get(&key) {
                if Instant::now() < *exp {
                    return Ok(*v);
                }
            }
        }

        let token = self.service_token().await?;
        let url = format!("{}/api/v1/identity-links/resolve", self.base_url);
        let mut req = self
            .client
            .get(&url)
            .query(&[("issuer", issuer), ("subject", subject)]);
        if let Some(t) = &token {
            req = req.bearer_auth(t);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "identity resolution request failed; failing closed");
                return Err(AppError::ServiceUnavailable(
                    "agent registry service unavailable".into(),
                ));
            }
        };

        let resolved = if resp.status() == reqwest::StatusCode::NOT_FOUND {
            None
        } else if !resp.status().is_success() {
            return Err(AppError::ServiceUnavailable(format!(
                "agent registry returned {} during identity resolution",
                resp.status()
            )));
        } else {
            let link: IdentityLinkResponse = resp.json().await.map_err(|e| {
                AppError::ServiceUnavailable(format!("invalid identity-link response: {e}"))
            })?;
            if link.status.eq_ignore_ascii_case("ACTIVE") && link.unlinked_at.is_none() {
                Some(EntityId::from_uuid(link.agent_id))
            } else {
                None
            }
        };

        if let Ok(mut guard) = self.identity_cache.lock() {
            guard.insert(key, (resolved, Instant::now() + TTL));
        }
        Ok(resolved)
    }
}

/// Local development / test fallback validator.
pub struct DevAgentValidator;

impl AgentValidationPort for DevAgentValidator {
    async fn validate_agent_active(&self, _agent_id: EntityId) -> Result<(), AppError> {
        Ok(())
    }
}

/// Fail-closed validator for production safety when upstream registry is unconfigured.
pub struct FailClosedAgentValidator;

impl AgentValidationPort for FailClosedAgentValidator {
    async fn validate_agent_active(&self, _agent_id: EntityId) -> Result<(), AppError> {
        Err(AppError::ServiceUnavailable(
            "Agent Service URL not configured; agent validation failed closed".into(),
        ))
    }
}

/// Dispatcher selecting either HTTP, Dev, or FailClosed validator.
pub enum AgentValidatorImpl {
    Http(HttpAgentValidator),
    Dev(DevAgentValidator),
    FailClosed(FailClosedAgentValidator),
}

impl AgentValidationPort for AgentValidatorImpl {
    async fn validate_agent_active(&self, agent_id: EntityId) -> Result<(), AppError> {
        match self {
            Self::Http(v) => v.validate_agent_active(agent_id).await,
            Self::Dev(v) => v.validate_agent_active(agent_id).await,
            Self::FailClosed(v) => v.validate_agent_active(agent_id).await,
        }
    }

    async fn resolve_agent_identity(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<EntityId>, AppError> {
        match self {
            Self::Http(v) => v.resolve_agent_identity(issuer, subject).await,
            // Dev/fail-closed validators never fabricate an identity.
            _ => Ok(None),
        }
    }
}
