//! Authenticated principal and actor representation.
//!
//! A `Principal` is produced ONLY by the authentication layer (JWT
//! validation in `infrastructure::security`) or by explicitly gated
//! development/test providers. API DTOs cannot construct one.

use serde::{Deserialize, Serialize};

/// Authenticated caller of the API.
#[derive(Debug, Clone)]
pub struct Principal {
    /// Token subject (`sub` claim) — the external identity, NOT an
    /// agent or person ID.
    pub subject: String,
    /// Token issuer.
    pub issuer: String,
    /// Scopes from the `scope` claim.
    pub scopes: Vec<String>,
    /// Realm roles from `realm_access.roles`.
    pub roles: Vec<String>,
    /// Optional agent-scope restriction claim (future org scoping).
    pub agent_scope: Option<String>,
}

impl Principal {
    /// True if `perm` appears in the effective role/permission set.
    /// `roles` already merges audience-scoped client roles upstream.
    pub fn has_permission(&self, perm: &str) -> bool {
        self.roles.iter().any(|r| r == perm)
    }

    /// Platform/agency admin roles get the full permission set.
    pub fn is_admin(&self) -> bool {
        self.roles.iter().any(|r| {
            matches!(
                r.as_str(),
                "IAOP_PLATFORM_ADMIN" | "IAOP_AGENCY_ADMIN" | "IAOP_OPERATIONS_ADMIN"
            )
        })
    }

    /// Stable scope string for idempotency partitioning.
    pub fn idempotency_scope(&self) -> String {
        format!("{}:{}", self.issuer, self.subject)
    }

    /// Resolves caller's Agent ID exclusively from the `agent_scope` /
    /// `agent_id` token claim. The token `sub` is an external identity
    /// (Keycloak user UUID), NOT an agent registry ID — identity linking
    /// is authoritative in Agent Service and must be propagated via the
    /// claim. Never fall back to parsing `sub`: that would be a
    /// confused-deputy risk.
    pub fn resolved_agent_id(&self) -> Option<iaop_agency_service_domain::entity_id::EntityId> {
        self.agent_scope.as_deref().and_then(|s| {
            s.parse::<iaop_agency_service_domain::entity_id::EntityId>()
                .ok()
        })
    }
}

/// Serializable principal claims extracted from a validated JWT.
/// Kept separate from `jsonwebtoken` types so the application layer has
/// no JWT dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenClaims {
    pub sub: String,
    pub iss: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub exp: Option<i64>,
    #[serde(default)]
    pub iat: Option<i64>,
    #[serde(default)]
    pub nbf: Option<i64>,
    #[serde(default)]
    pub aud: Option<serde_json::Value>,
    #[serde(default)]
    pub realm_access: Option<RealmAccess>,
    #[serde(default)]
    pub resource_access: Option<std::collections::HashMap<String, ClientRoles>>,
    #[serde(default, alias = "agent_id")]
    pub agent_scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealmAccess {
    #[serde(default)]
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRoles {
    #[serde(default)]
    pub roles: Vec<String>,
}
