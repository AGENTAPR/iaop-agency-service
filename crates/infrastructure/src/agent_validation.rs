//! Authoritative Agent Registry validation adapter for Agency Service.

use std::time::Duration;

use serde::Deserialize;

use iaop_agency_service_application::error::AppError;
use iaop_agency_service_application::ports::AgentValidationPort;
use iaop_agency_service_domain::entity_id::EntityId;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentResponse {
    lifecycle_status: String,
}

/// HTTP client calling `iaop-agent-service` to verify agent identity and status.
pub struct HttpAgentValidator {
    client: reqwest::Client,
    base_url: String,
}

impl HttpAgentValidator {
    pub fn new(base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap_or_default();
        Self { client, base_url }
    }
}

impl AgentValidationPort for HttpAgentValidator {
    async fn validate_agent_active(&self, agent_id: EntityId) -> Result<(), AppError> {
        let url = format!("{}/api/v1/agents/{}", self.base_url, agent_id.as_uuid());
        let resp = match self.client.get(&url).send().await {
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
                "agent lifecycle status is {} â€” assignment prohibited",
                agent.lifecycle_status
            )));
        }

        Ok(())
    }
}

/// Local development / test fallback validator.
pub struct DevAgentValidator;

impl AgentValidationPort for DevAgentValidator {
    async fn validate_agent_active(&self, _agent_id: EntityId) -> Result<(), AppError> {
        Ok(())
    }
}

/// Dispatcher selecting either HTTP or Dev validator.
pub enum AgentValidatorImpl {
    Http(HttpAgentValidator),
    Dev(DevAgentValidator),
}

impl AgentValidationPort for AgentValidatorImpl {
    async fn validate_agent_active(&self, agent_id: EntityId) -> Result<(), AppError> {
        match self {
            Self::Http(v) => v.validate_agent_active(agent_id).await,
            Self::Dev(v) => v.validate_agent_active(agent_id).await,
        }
    }
}
