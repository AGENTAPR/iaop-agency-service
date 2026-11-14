#![allow(async_fn_in_trait)]
//! Application ports (hexagonal boundaries). No infrastructure types.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use iaop_agency_service_domain::agency::Agency;
use iaop_agency_service_domain::assignment::{Assignment, Transfer};
use iaop_agency_service_domain::entity_id::EntityId;
use iaop_agency_service_domain::leadership::LeadershipAppointment;
use iaop_agency_service_domain::org_unit::{OrgRelationship, OrgUnit};

use crate::error::AppError;

/// Transaction handle type used by write ports (SQLx Postgres).
pub type Tx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

/// Row written to the transactional outbox inside the business tx.
#[derive(Debug, Clone)]
pub struct OutboxEvent {
    pub event_id: Uuid,
    pub aggregate_id: Uuid,
    pub aggregate_type: String,
    pub aggregate_version: Option<i64>,
    pub event_type: String,
    pub event_version: i32,
    pub payload: serde_json::Value,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub source: String,
    pub correlation_id: Option<Uuid>,
}

/// Immutable audit record written in the same tx as the mutation.
#[derive(Debug, Clone)]
pub struct AuditRecord {
    pub id: Uuid,
    pub actor_subject: String,
    pub action: String,
    pub aggregate_type: String,
    pub aggregate_id: Uuid,
    pub previous_state: Option<serde_json::Value>,
    pub new_state: Option<serde_json::Value>,
    pub effective_date: Option<NaiveDate>,
    pub reason: Option<String>,
    pub correlation_id: Option<Uuid>,
}

/// Read model for an org unit joined with its current parent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitView {
    #[serde(flatten)]
    pub unit: OrgUnit,
    pub parent_id: Option<EntityId>,
}

/// Durable store for agency aggregates. Implemented by infrastructure.
pub trait OrgStore {
    // ---- agencies ----
    async fn insert_agency(&self, tx: &mut Tx<'_>, a: &Agency) -> Result<(), AppError>;
    async fn get_agency(&self, id: EntityId) -> Result<Option<Agency>, AppError>;
    async fn get_agency_by_code(&self, code: &str) -> Result<Option<Agency>, AppError>;
    async fn list_agencies(
        &self,
        limit: u32,
        cursor: Option<Uuid>,
    ) -> Result<Vec<Agency>, AppError>;
    async fn update_agency(
        &self,
        tx: &mut Tx<'_>,
        a: &Agency,
        expected_version: i64,
    ) -> Result<(), AppError>;

    // ---- org units ----
    async fn insert_unit(&self, tx: &mut Tx<'_>, u: &OrgUnit) -> Result<(), AppError>;
    async fn get_unit(&self, id: EntityId) -> Result<Option<OrgUnit>, AppError>;
    async fn list_units(
        &self,
        agency_id: Option<EntityId>,
        limit: u32,
        cursor: Option<Uuid>,
    ) -> Result<Vec<UnitView>, AppError>;
    async fn update_unit(
        &self,
        tx: &mut Tx<'_>,
        u: &OrgUnit,
        expected_version: i64,
    ) -> Result<(), AppError>;

    // ---- relationships (temporal) ----
    async fn insert_relationship(
        &self,
        tx: &mut Tx<'_>,
        r: &OrgRelationship,
    ) -> Result<(), AppError>;
    /// Closes an open relationship at `effective_to` (exclusive).
    async fn close_relationship(
        &self,
        tx: &mut Tx<'_>,
        child_id: EntityId,
        effective_to: NaiveDate,
    ) -> Result<(), AppError>;
    /// Current (open-ended) parent of a unit.
    async fn current_parent(&self, unit_id: EntityId) -> Result<Option<EntityId>, AppError>;
    /// Parent chain of `unit_id` walking current edges upward.
    async fn ancestors(&self, unit_id: EntityId) -> Result<Vec<EntityId>, AppError>;
    /// All descendant unit ids via recursive CTE on current edges.
    async fn descendants(&self, unit_id: EntityId) -> Result<Vec<EntityId>, AppError>;
    /// Children of a unit under the relationship covering `as_of`.
    async fn children_as_of(
        &self,
        unit_id: EntityId,
        as_of: NaiveDate,
    ) -> Result<Vec<EntityId>, AppError>;
    /// Ancestor chain as of a date.
    async fn ancestors_as_of(
        &self,
        unit_id: EntityId,
        as_of: NaiveDate,
    ) -> Result<Vec<EntityId>, AppError>;

    // ---- assignments ----
    async fn insert_assignment(&self, tx: &mut Tx<'_>, a: &Assignment) -> Result<(), AppError>;
    async fn get_assignment(&self, id: EntityId) -> Result<Option<Assignment>, AppError>;
    async fn update_assignment(
        &self,
        tx: &mut Tx<'_>,
        a: &Assignment,
        expected_version: i64,
    ) -> Result<(), AppError>;
    /// Assignments for an agent covering `as_of`.
    async fn assignments_for_agent_as_of(
        &self,
        agent_id: EntityId,
        as_of: NaiveDate,
    ) -> Result<Vec<Assignment>, AppError>;
    /// All assignments for an agent (history).
    async fn assignments_for_agent(&self, agent_id: EntityId) -> Result<Vec<Assignment>, AppError>;
    /// Primary assignments overlapping [from, to) — overlap detection.
    async fn overlapping_primary(
        &self,
        agent_id: EntityId,
        from: NaiveDate,
        to: Option<NaiveDate>,
    ) -> Result<Vec<Assignment>, AppError>;

    // ---- transfers ----
    async fn insert_transfer(&self, tx: &mut Tx<'_>, t: &Transfer) -> Result<(), AppError>;
    async fn get_transfer(&self, id: EntityId) -> Result<Option<Transfer>, AppError>;
    async fn list_transfers(
        &self,
        agent_id: Option<EntityId>,
        limit: u32,
    ) -> Result<Vec<Transfer>, AppError>;

    // ---- leadership appointments ----
    async fn insert_leadership(
        &self,
        tx: &mut Tx<'_>,
        a: &LeadershipAppointment,
    ) -> Result<(), AppError>;
    async fn get_leadership(&self, id: EntityId)
    -> Result<Option<LeadershipAppointment>, AppError>;
    async fn update_leadership(
        &self,
        tx: &mut Tx<'_>,
        a: &LeadershipAppointment,
        expected_version: i64,
    ) -> Result<(), AppError>;
    /// Open leadership appointments covering `as_of` for a unit.
    async fn leaders_as_of(
        &self,
        unit_id: EntityId,
        as_of: NaiveDate,
    ) -> Result<Vec<LeadershipAppointment>, AppError>;

    /// Serializes hierarchy mutations: takes a table-level lock on
    /// organization_relationships so cycle checks + edge writes are atomic
    /// against concurrent reparent/create operations. Org-structure changes
    /// are low-frequency admin operations; this is the correct trade-off.
    async fn lock_relationships(&self, tx: &mut Tx<'_>) -> Result<(), AppError>;

    // ---- transactional outbox + audit (write inside business tx) ----
    async fn insert_outbox(&self, tx: &mut Tx<'_>, e: &OutboxEvent) -> Result<(), AppError>;
    async fn insert_audit(&self, tx: &mut Tx<'_>, a: &AuditRecord) -> Result<(), AppError>;
}

/// Token validator port (implemented by JWT/JWKS infra).
pub trait TokenValidator {
    fn validate<'a>(
        &'a self,
        token: &'a str,
    ) -> impl std::future::Future<Output = Result<crate::principal::Principal, AppError>> + Send;
}

/// Outcome of reserving an idempotency key.
#[derive(Debug)]
pub enum IdempotencyReserve {
    /// This request owns the key — proceed and `complete` it.
    Acquired { record_id: Uuid },
    /// Identical request already completed — replay stored response.
    Replay {
        status: i32,
        body: serde_json::Value,
    },
    /// Same key, different payload, or in-flight duplicate.
    Conflict,
}

/// Durable idempotency store. Reservation happens INSIDE the business
/// transaction so a rollback never leaves a phantom record.
pub trait IdempotencyStore: Send + Sync {
    async fn reserve(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        principal_scope: &str,
        operation: &str,
        key: &str,
        fingerprint: &str,
        ttl: std::time::Duration,
    ) -> Result<IdempotencyReserve, AppError>;

    async fn complete(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        record_id: Uuid,
        status: i32,
        body: serde_json::Value,
    ) -> Result<(), AppError>;
}

/// Versioned integration event envelope (see iaop-event-contracts).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IntegrationEvent {
    pub event_id: Uuid,
    pub event_type: String,
    pub event_version: u32,
    pub aggregate_id: Uuid,
    pub aggregate_version: Option<u64>,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub source: String,
    pub correlation_id: Option<Uuid>,
    pub causation_id: Option<Uuid>,
    pub payload: serde_json::Value,
}

/// Transport port for the outbox publisher.
pub trait EventTransport: Send + Sync {
    async fn deliver(&self, event: &IntegrationEvent) -> Result<(), String>;
}

/// Reserved placeholder matching earlier scaffold references.
#[derive(Debug)]
pub enum IdemOutcome {
    Acquired {
        record_id: Uuid,
    },
    Replay {
        status: i32,
        body: serde_json::Value,
    },
    Conflict,
}
