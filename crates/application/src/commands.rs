//! Use-case commands for Agency Service.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use iaop_agency_service_domain::agency::{Agency, AgencyStatus};
use iaop_agency_service_domain::assignment::{Assignment, AssignmentType, Transfer};
use iaop_agency_service_domain::entity_id::EntityId;
use iaop_agency_service_domain::org_unit::{OrgUnit, UnitKind, UnitStatus, would_create_cycle};

use crate::error::AppError;
use crate::permissions;
use crate::ports::{
    AgentValidationPort, AuditRecord, NoopAgentValidator, OrgStore, OutboxEvent, Tx,
};
use crate::principal::Principal;

#[derive(Debug, Clone)]
pub struct CreateAgency {
    pub code: String,
    pub name: String,
    pub principal: Principal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgencyDto {
    pub id: EntityId,
    pub code: String,
    pub name: String,
    pub status: String,
    pub version: i64,
}

impl From<&Agency> for AgencyDto {
    fn from(a: &Agency) -> Self {
        Self {
            id: a.id,
            code: a.code.clone(),
            name: a.name.clone(),
            status: format!("{:?}", a.status).to_uppercase(),
            version: a.version,
        }
    }
}

pub async fn create_agency<S: OrgStore>(
    store: &S,
    tx: &mut Tx<'_>,
    cmd: CreateAgency,
) -> Result<AgencyDto, AppError> {
    permissions::require(&cmd.principal, permissions::Permission::AgencyCreate)?;
    if store.get_agency_by_code(&cmd.code).await?.is_some() {
        return Err(AppError::Conflict("agency code already exists".into()));
    }
    let agency = Agency::create(cmd.code, cmd.name).map_err(AppError::from)?;
    store.insert_agency(tx, &agency).await?;
    emit(store, tx, Emit {
        event_type: "agency.created.v1",
        aggregate_type: "Agency",
        aggregate_id: agency.id,
        aggregate_version: Some(agency.version),
        payload: serde_json::json!({"agency_id": agency.id, "code": agency.code, "name": agency.name}),
        principal: cmd.principal.clone(),
        action: "agency.create",
        effective_date: None,
    }).await?;
    Ok(AgencyDto::from(&agency))
}

#[derive(Debug, Clone)]
pub struct CreateUnit {
    pub agency_id: EntityId,
    pub code: String,
    pub name: String,
    pub kind: UnitKind,
    pub parent_id: Option<EntityId>,
    pub effective_from: NaiveDate,
    pub principal: Principal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitDto {
    pub id: EntityId,
    pub agency_id: EntityId,
    pub code: String,
    pub name: String,
    pub kind: String,
    pub status: String,
    pub parent_id: Option<EntityId>,
    pub version: i64,
}

impl From<(&OrgUnit, Option<EntityId>)> for UnitDto {
    fn from((u, p): (&OrgUnit, Option<EntityId>)) -> Self {
        Self {
            id: u.id,
            agency_id: u.agency_id,
            code: u.code.clone(),
            name: u.name.clone(),
            kind: format!("{:?}", u.kind).to_uppercase(),
            status: format!("{:?}", u.status).to_uppercase(),
            parent_id: p,
            version: u.version,
        }
    }
}

pub async fn create_unit<S: OrgStore>(
    store: &S,
    tx: &mut Tx<'_>,
    cmd: CreateUnit,
) -> Result<UnitDto, AppError> {
    permissions::require(&cmd.principal, permissions::Permission::HierarchyManage)?;
    let agency = store
        .get_agency(cmd.agency_id)
        .await?
        .ok_or_else(|| AppError::NotFound("entity".into()))?;
    if agency.status != AgencyStatus::Active {
        return Err(AppError::Conflict("agency is not active".into()));
    }
    if let Some(pid) = cmd.parent_id {
        let parent = store
            .get_unit(pid)
            .await?
            .ok_or_else(|| AppError::NotFound("entity".into()))?;
        if parent.agency_id != cmd.agency_id {
            return Err(AppError::Conflict(
                "parent belongs to different agency".into(),
            ));
        }
    }
    let unit =
        OrgUnit::create(cmd.agency_id, cmd.code, cmd.name, cmd.kind).map_err(AppError::from)?;
    store.lock_relationships(tx).await?;
    store.insert_unit(tx, &unit).await?;
    if let Some(pid) = cmd.parent_id {
        store
            .insert_relationship(tx, &open_rel(pid, unit.id, cmd.effective_from))
            .await?;
    }
    emit(store, tx, Emit { event_type: "agency.organization-unit-created.v1", aggregate_type: "OrgUnit", aggregate_id: unit.id, aggregate_version: Some(unit.version), payload: serde_json::json!({"unit_id": unit.id, "agency_id": unit.agency_id, "code": unit.code, "kind": format!("{:?}", unit.kind)}), principal: cmd.principal.clone(), action: "org_unit.create", effective_date: Some(cmd.effective_from) }).await?;
    Ok(UnitDto::from((&unit, cmd.parent_id)))
}

#[derive(Debug, Clone)]
pub struct ChangeParent {
    pub unit_id: EntityId,
    pub new_parent_id: Option<EntityId>,
    pub effective_from: NaiveDate,
    pub expected_version: i64,
    pub principal: Principal,
}

/// Changes a unit's parent effective `effective_from`. Closes the current
/// relationship at that date and opens the new edge. Cycle-checked.
pub async fn change_parent<S: OrgStore>(
    store: &S,
    tx: &mut Tx<'_>,
    cmd: ChangeParent,
) -> Result<UnitDto, AppError> {
    permissions::require(&cmd.principal, permissions::Permission::HierarchyManage)?;
    store.lock_relationships(tx).await?;
    let mut unit = store
        .get_unit(cmd.unit_id)
        .await?
        .ok_or_else(|| AppError::NotFound("entity".into()))?;
    if unit.version != cmd.expected_version {
        return Err(AppError::PreconditionFailed("version mismatch".into()));
    }
    if unit.status == UnitStatus::Inactive {
        return Err(AppError::Conflict("cannot reparent inactive unit".into()));
    }
    if let Some(pid) = cmd.new_parent_id {
        if pid == unit.id {
            return Err(AppError::Conflict("unit cannot be its own parent".into()));
        }
        let parent = store
            .get_unit(pid)
            .await?
            .ok_or_else(|| AppError::NotFound("entity".into()))?;
        if parent.agency_id != unit.agency_id {
            return Err(AppError::Conflict(
                "parent belongs to different agency".into(),
            ));
        }
        let ancestors = store.ancestors(pid).await?;
        if would_create_cycle(unit.id, pid, &ancestors) {
            return Err(AppError::Conflict(
                "reparenting would create a cycle".into(),
            ));
        }
    }
    let old_parent = store.current_parent(unit.id).await?;
    store
        .close_relationship(tx, unit.id, cmd.effective_from)
        .await?;
    if let Some(pid) = cmd.new_parent_id {
        store
            .insert_relationship(tx, &open_rel(pid, unit.id, cmd.effective_from))
            .await?;
    }
    unit.bump_version();
    store.update_unit(tx, &unit, cmd.expected_version).await?;
    emit(store, tx, Emit { event_type: "agency.hierarchy-changed.v1", aggregate_type: "OrgUnit", aggregate_id: unit.id, aggregate_version: Some(unit.version), payload: serde_json::json!({"unit_id": unit.id, "old_parent_id": old_parent, "new_parent_id": cmd.new_parent_id, "effective_from": cmd.effective_from}), principal: cmd.principal.clone(), action: "org_unit.reparent", effective_date: Some(cmd.effective_from) }).await?;
    Ok(UnitDto::from((&unit, cmd.new_parent_id.or(old_parent))))
}

#[derive(Debug, Clone)]
pub struct AssignAgent {
    pub agent_id: EntityId,
    pub unit_id: EntityId,
    pub assignment_type: AssignmentType,
    pub effective_from: NaiveDate,
    pub principal: Principal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignmentDto {
    pub id: EntityId,
    pub agent_id: EntityId,
    pub unit_id: EntityId,
    pub assignment_type: String,
    pub status: String,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    pub version: i64,
}

impl From<&Assignment> for AssignmentDto {
    fn from(a: &Assignment) -> Self {
        Self {
            id: a.id,
            agent_id: a.agent_id,
            unit_id: a.unit_id,
            assignment_type: format!("{:?}", a.assignment_type).to_uppercase(),
            status: format!("{:?}", a.status).to_uppercase(),
            effective_from: a.effective_from,
            effective_to: a.effective_to,
            version: a.version,
        }
    }
}

pub async fn assign_agent<S: OrgStore>(
    store: &S,
    tx: &mut Tx<'_>,
    cmd: AssignAgent,
) -> Result<AssignmentDto, AppError> {
    assign_agent_validated(store, &NoopAgentValidator, tx, cmd).await
}

pub async fn assign_agent_validated<S: OrgStore, V: AgentValidationPort>(
    store: &S,
    validator: &V,
    tx: &mut Tx<'_>,
    cmd: AssignAgent,
) -> Result<AssignmentDto, AppError> {
    permissions::require(&cmd.principal, permissions::Permission::AssignmentManage)?;
    validator.validate_agent_active(cmd.agent_id).await?;
    let unit = store
        .get_unit(cmd.unit_id)
        .await?
        .ok_or_else(|| AppError::NotFound("entity".into()))?;
    if unit.status != UnitStatus::Active {
        return Err(AppError::Conflict("cannot assign to inactive unit".into()));
    }
    let overlaps = store
        .overlapping_primary(cmd.agent_id, cmd.effective_from, None)
        .await?;
    if cmd.assignment_type == AssignmentType::Primary
        && overlaps
            .iter()
            .any(|a| a.assignment_type == AssignmentType::Primary)
    {
        return Err(AppError::Conflict(
            "agent already has a primary assignment in this window".into(),
        ));
    }
    let mut a = Assignment::create(
        cmd.agent_id,
        cmd.unit_id,
        cmd.assignment_type,
        cmd.effective_from,
    )
    .map_err(AppError::from)?;
    let today = chrono::Utc::now().date_naive();
    if cmd.effective_from <= today {
        a.activate(today).map_err(AppError::from)?;
    }
    store.insert_assignment(tx, &a).await?;
    emit(store, tx, Emit { event_type: "agency.agent-assigned.v1", aggregate_type: "Assignment", aggregate_id: a.id, aggregate_version: Some(a.version), payload: serde_json::json!({"assignment_id": a.id, "agent_id": a.agent_id, "unit_id": a.unit_id, "effective_from": a.effective_from}), principal: cmd.principal.clone(), action: "assignment.create", effective_date: Some(a.effective_from) }).await?;
    Ok(AssignmentDto::from(&a))
}

#[derive(Debug, Clone)]
pub struct TransferAgent {
    pub agent_id: EntityId,
    pub to_unit_id: EntityId,
    pub effective_from: NaiveDate,
    pub reason: Option<String>,
    pub principal: Principal,
}

/// Ends the agent's current primary assignment at `effective_from` and
/// creates the new assignment + transfer record.
pub async fn transfer_agent<S: OrgStore>(
    store: &S,
    tx: &mut Tx<'_>,
    cmd: TransferAgent,
) -> Result<(AssignmentDto, Transfer), AppError> {
    transfer_agent_validated(store, &NoopAgentValidator, tx, cmd).await
}

pub async fn transfer_agent_validated<S: OrgStore, V: AgentValidationPort>(
    store: &S,
    validator: &V,
    tx: &mut Tx<'_>,
    cmd: TransferAgent,
) -> Result<(AssignmentDto, Transfer), AppError> {
    permissions::require(&cmd.principal, permissions::Permission::TransferRequest)?;
    validator.validate_agent_active(cmd.agent_id).await?;
    let to_unit = store
        .get_unit(cmd.to_unit_id)
        .await?
        .ok_or_else(|| AppError::NotFound("entity".into()))?;
    if to_unit.status != UnitStatus::Active {
        return Err(AppError::Conflict(
            "cannot transfer to inactive unit".into(),
        ));
    }
    let current = store
        .assignments_for_agent_as_of(cmd.agent_id, cmd.effective_from)
        .await?
        .into_iter()
        .find(|a| a.assignment_type == AssignmentType::Primary);
    let from_unit = current.as_ref().map(|a| a.unit_id);

    let mut new_assignment = Assignment::create(
        cmd.agent_id,
        cmd.to_unit_id,
        AssignmentType::Primary,
        cmd.effective_from,
    )
    .map_err(AppError::from)?;
    let today = chrono::Utc::now().date_naive();
    if cmd.effective_from <= today {
        new_assignment.activate(today).map_err(AppError::from)?;
    }
    let transfer = Transfer::record(
        cmd.agent_id,
        from_unit,
        cmd.to_unit_id,
        cmd.effective_from,
        cmd.reason,
    );

    if let Some(mut cur) = current {
        let expected = cur.version;
        cur.end(cmd.effective_from).map_err(AppError::from)?;
        store.update_assignment(tx, &cur, expected).await?;
    }
    store.insert_assignment(tx, &new_assignment).await?;
    store.insert_transfer(tx, &transfer).await?;
    emit(store, tx, Emit { event_type: "agency.agent-transferred.v1", aggregate_type: "Transfer", aggregate_id: transfer.id, aggregate_version: Some(transfer.version), payload: serde_json::json!({"transfer_id": transfer.id, "agent_id": transfer.agent_id, "from_unit_id": transfer.from_unit_id, "to_unit_id": transfer.to_unit_id, "effective_from": transfer.effective_from}), principal: cmd.principal.clone(), action: "agent.transfer", effective_date: Some(cmd.effective_from) }).await?;
    Ok((AssignmentDto::from(&new_assignment), transfer))
}

fn open_rel(
    parent: EntityId,
    child: EntityId,
    from: chrono::NaiveDate,
) -> iaop_agency_service_domain::org_unit::OrgRelationship {
    iaop_agency_service_domain::org_unit::OrgRelationship {
        id: EntityId::new(),
        parent_id: parent,
        child_id: child,
        effective_from: from,
        effective_to: None,
        created_at: chrono::Utc::now(),
    }
}

/// Writes the domain event to the outbox and an audit record inside `tx`.
struct Emit {
    event_type: &'static str,
    aggregate_type: &'static str,
    aggregate_id: EntityId,
    aggregate_version: Option<i64>,
    payload: serde_json::Value,
    principal: Principal,
    action: &'static str,
    effective_date: Option<NaiveDate>,
}

async fn emit<S: OrgStore>(store: &S, tx: &mut Tx<'_>, e: Emit) -> Result<(), AppError> {
    let principal = &e.principal;
    store
        .insert_outbox(
            tx,
            &OutboxEvent {
                event_id: EntityId::new().as_uuid(),
                aggregate_id: e.aggregate_id.as_uuid(),
                aggregate_type: e.aggregate_type.into(),
                aggregate_version: e.aggregate_version,
                event_type: e.event_type.into(),
                event_version: 1,
                payload: e.payload.clone(),
                occurred_at: chrono::Utc::now(),
                source: "iaop-agency-service".into(),
                correlation_id: None,
            },
        )
        .await?;
    store
        .insert_audit(
            tx,
            &AuditRecord {
                id: EntityId::new().as_uuid(),
                actor_subject: principal.subject.clone(),
                action: e.action.into(),
                aggregate_type: e.aggregate_type.into(),
                aggregate_id: e.aggregate_id.as_uuid(),
                previous_state: None,
                new_state: Some(e.payload.clone()),
                effective_date: e.effective_date,
                reason: None,
                correlation_id: None,
            },
        )
        .await
}

// ---- leadership appointments ----

use iaop_agency_service_domain::leadership::{LeadershipAppointment, LeadershipRole};

#[derive(Debug, Clone)]
pub struct AssignLeader {
    pub unit_id: EntityId,
    pub agent_id: EntityId,
    pub role: LeadershipRole,
    pub effective_from: NaiveDate,
    pub principal: Principal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeadershipDto {
    pub id: EntityId,
    pub unit_id: EntityId,
    pub agent_id: EntityId,
    pub role: String,
    pub status: String,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    pub version: i64,
}

impl From<&LeadershipAppointment> for LeadershipDto {
    fn from(a: &LeadershipAppointment) -> Self {
        Self {
            id: a.id,
            unit_id: a.unit_id,
            agent_id: a.agent_id,
            role: a.role.as_str().to_string(),
            status: format!("{:?}", a.status).to_uppercase(),
            effective_from: a.effective_from,
            effective_to: a.effective_to,
            version: a.version,
        }
    }
}

pub async fn assign_leader<S: OrgStore>(
    store: &S,
    tx: &mut Tx<'_>,
    cmd: AssignLeader,
) -> Result<LeadershipDto, AppError> {
    permissions::require(&cmd.principal, permissions::Permission::LeadershipManage)?;
    let unit = store
        .get_unit(cmd.unit_id)
        .await?
        .ok_or_else(|| AppError::NotFound("entity".into()))?;
    if unit.status != UnitStatus::Active {
        return Err(AppError::Conflict(
            "cannot assign leader to inactive unit".into(),
        ));
    }
    // One open appointment per (unit, role).
    let existing = store.leaders_as_of(cmd.unit_id, cmd.effective_from).await?;
    if existing.iter().any(|a| a.role == cmd.role) {
        return Err(AppError::Conflict(
            "unit already has an open appointment for this role".into(),
        ));
    }
    let mut a =
        LeadershipAppointment::create(cmd.unit_id, cmd.agent_id, cmd.role, cmd.effective_from)
            .map_err(AppError::from)?;
    let today = chrono::Utc::now().date_naive();
    if cmd.effective_from <= today {
        a.activate(today).map_err(AppError::from)?;
    }
    store.insert_leadership(tx, &a).await?;
    emit(store, tx, Emit {
        event_type: "agency.leadership-assigned.v1",
        aggregate_type: "LeadershipAppointment",
        aggregate_id: a.id,
        aggregate_version: Some(a.version),
        payload: serde_json::json!({"appointment_id": a.id, "unit_id": a.unit_id, "agent_id": a.agent_id, "role": a.role.as_str(), "effective_from": a.effective_from}),
        principal: cmd.principal.clone(),
        action: "leadership.assign",
        effective_date: Some(a.effective_from),
    }).await?;
    Ok(LeadershipDto::from(&a))
}

#[derive(Debug, Clone)]
pub struct EndLeader {
    pub appointment_id: EntityId,
    pub effective_to: NaiveDate,
    pub expected_version: i64,
    pub principal: Principal,
}

pub async fn end_leader<S: OrgStore>(
    store: &S,
    tx: &mut Tx<'_>,
    cmd: EndLeader,
) -> Result<LeadershipDto, AppError> {
    permissions::require(&cmd.principal, permissions::Permission::LeadershipManage)?;
    let mut a = store
        .get_leadership(cmd.appointment_id)
        .await?
        .ok_or_else(|| AppError::NotFound("entity".into()))?;
    if a.version != cmd.expected_version {
        return Err(AppError::PreconditionFailed("version mismatch".into()));
    }
    a.end(cmd.effective_to).map_err(AppError::from)?;
    store
        .update_leadership(tx, &a, cmd.expected_version)
        .await?;
    emit(store, tx, Emit {
        event_type: "agency.leadership-ended.v1",
        aggregate_type: "LeadershipAppointment",
        aggregate_id: a.id,
        aggregate_version: Some(a.version),
        payload: serde_json::json!({"appointment_id": a.id, "unit_id": a.unit_id, "agent_id": a.agent_id, "effective_to": cmd.effective_to}),
        principal: cmd.principal.clone(),
        action: "leadership.end",
        effective_date: Some(cmd.effective_to),
    }).await?;
    Ok(LeadershipDto::from(&a))
}
