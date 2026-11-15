//! Read-side queries for Agency Service.
//! Enforces: Authentication + Technical Permission + Organizational Scope.

use chrono::NaiveDate;

use iaop_agency_service_domain::entity_id::EntityId;

use crate::authz::{self, OrgScope};
use crate::commands::{AgencyDto, AssignmentDto, UnitDto};
use crate::error::AppError;
use crate::permissions;
use crate::ports::OrgStore;
use crate::principal::Principal;

pub async fn get_agency<S: OrgStore>(
    store: &S,
    id: EntityId,
    principal: &Principal,
) -> Result<AgencyDto, AppError> {
    permissions::require_read(principal)?;
    let today = chrono::Utc::now().date_naive();
    let scope = authz::resolve_scope(store, principal, today).await?;
    if !scope.is_agency_accessible(id) {
        return Err(AppError::Forbidden(format!(
            "agency '{}' outside caller's authorized organizational scope",
            id
        )));
    }
    let a = store
        .get_agency(id)
        .await?
        .ok_or_else(|| AppError::NotFound("entity".into()))?;
    Ok(AgencyDto::from(&a))
}

pub async fn list_agencies<S: OrgStore>(
    store: &S,
    limit: u32,
    cursor: Option<uuid::Uuid>,
    principal: &Principal,
) -> Result<Vec<AgencyDto>, AppError> {
    permissions::require_read(principal)?;
    let today = chrono::Utc::now().date_naive();
    let scope = authz::resolve_scope(store, principal, today).await?;
    let rows = store.list_agencies(limit.min(200), cursor).await?;
    let filtered: Vec<AgencyDto> = rows
        .iter()
        .filter(|a| scope.is_agency_accessible(a.id))
        .map(AgencyDto::from)
        .collect();
    Ok(filtered)
}

pub async fn get_unit<S: OrgStore>(
    store: &S,
    id: EntityId,
    principal: &Principal,
) -> Result<UnitDto, AppError> {
    permissions::require_read(principal)?;
    let today = chrono::Utc::now().date_naive();
    let scope = authz::resolve_scope(store, principal, today).await?;
    if !scope.is_unit_accessible(id) {
        return Err(AppError::Forbidden(format!(
            "unit '{}' outside caller's authorized organizational scope",
            id
        )));
    }
    let u = store
        .get_unit(id)
        .await?
        .ok_or_else(|| AppError::NotFound("entity".into()))?;
    let parent = store.current_parent(id).await?;
    Ok(UnitDto::from((&u, parent)))
}

pub async fn list_units<S: OrgStore>(
    store: &S,
    agency_id: Option<EntityId>,
    limit: u32,
    cursor: Option<uuid::Uuid>,
    principal: &Principal,
) -> Result<Vec<UnitDto>, AppError> {
    permissions::require_read(principal)?;
    let today = chrono::Utc::now().date_naive();
    let scope = authz::resolve_scope(store, principal, today).await?;
    if let Some(aid) = agency_id {
        if !scope.is_agency_accessible(aid) {
            return Err(AppError::Forbidden(format!(
                "agency '{}' outside caller's authorized organizational scope",
                aid
            )));
        }
    }
    let rows = store.list_units(agency_id, limit.min(200), cursor).await?;
    let filtered: Vec<UnitDto> = rows
        .iter()
        .filter(|v| scope.is_unit_accessible(v.unit.id))
        .map(|v| UnitDto::from((&v.unit, v.parent_id)))
        .collect();
    Ok(filtered)
}

/// Ancestor chain (unit -> root) as of `as_of` (defaults to today).
pub async fn ancestors<S: OrgStore>(
    store: &S,
    unit_id: EntityId,
    as_of: Option<NaiveDate>,
    principal: &Principal,
) -> Result<Vec<EntityId>, AppError> {
    permissions::require_read(principal)?;
    let date = as_of.unwrap_or_else(|| chrono::Utc::now().date_naive());
    let scope = authz::resolve_scope(store, principal, date).await?;
    if !scope.is_unit_accessible(unit_id) {
        return Err(AppError::Forbidden(format!(
            "unit '{}' outside caller's authorized organizational scope",
            unit_id
        )));
    }
    store.ancestors_as_of(unit_id, date).await
}

/// Descendant unit ids (current hierarchy).
pub async fn descendants<S: OrgStore>(
    store: &S,
    unit_id: EntityId,
    principal: &Principal,
) -> Result<Vec<EntityId>, AppError> {
    permissions::require_read(principal)?;
    let today = chrono::Utc::now().date_naive();
    let scope = authz::resolve_scope(store, principal, today).await?;
    if !scope.is_unit_accessible(unit_id) {
        return Err(AppError::Forbidden(format!(
            "unit '{}' outside caller's authorized organizational scope",
            unit_id
        )));
    }
    store.descendants(unit_id).await
}

/// Agent assignments as of a date (default today).
pub async fn agent_assignments_as_of<S: OrgStore>(
    store: &S,
    agent_id: EntityId,
    as_of: Option<NaiveDate>,
    principal: &Principal,
) -> Result<Vec<AssignmentDto>, AppError> {
    permissions::require(principal, permissions::Permission::AssignmentRead)?;
    let date = as_of.unwrap_or_else(|| chrono::Utc::now().date_naive());
    let scope = authz::resolve_scope(store, principal, date).await?;

    match &scope {
        OrgScope::Global => {}
        OrgScope::Scoped {
            agent_id: self_id,
            accessible_units,
            ..
        } => {
            if *self_id != agent_id {
                let target_assignments = store.assignments_for_agent_as_of(agent_id, date).await?;
                let in_scope = target_assignments
                    .iter()
                    .any(|a| accessible_units.contains(&a.unit_id));
                if !in_scope {
                    return Err(AppError::Forbidden(format!(
                        "agent '{}' assignments outside caller's authorized leadership scope",
                        agent_id
                    )));
                }
            }
        }
    }

    let rows = store.assignments_for_agent_as_of(agent_id, date).await?;
    Ok(rows.iter().map(AssignmentDto::from).collect())
}

/// Full assignment history for an agent.
pub async fn agent_assignment_history<S: OrgStore>(
    store: &S,
    agent_id: EntityId,
    principal: &Principal,
) -> Result<Vec<AssignmentDto>, AppError> {
    permissions::require(principal, permissions::Permission::AssignmentRead)?;
    let today = chrono::Utc::now().date_naive();
    let scope = authz::resolve_scope(store, principal, today).await?;

    match &scope {
        OrgScope::Global => {}
        OrgScope::Scoped {
            agent_id: self_id,
            accessible_units,
            ..
        } => {
            if *self_id != agent_id {
                let target_assignments = store.assignments_for_agent(agent_id).await?;
                let in_scope = target_assignments
                    .iter()
                    .any(|a| accessible_units.contains(&a.unit_id));
                if !in_scope {
                    return Err(AppError::Forbidden(format!(
                        "agent '{}' assignment history outside caller's authorized leadership scope",
                        agent_id
                    )));
                }
            }
        }
    }

    let rows = store.assignments_for_agent(agent_id).await?;
    Ok(rows.iter().map(AssignmentDto::from).collect())
}

/// Agent assignments for a unit as of a date (default today).
pub async fn unit_assignments_as_of<S: OrgStore>(
    store: &S,
    unit_id: EntityId,
    as_of: Option<NaiveDate>,
    principal: &Principal,
) -> Result<Vec<AssignmentDto>, AppError> {
    permissions::require(principal, permissions::Permission::AssignmentRead)?;
    let date = as_of.unwrap_or_else(|| chrono::Utc::now().date_naive());
    let scope = authz::resolve_scope(store, principal, date).await?;

    if !scope.is_unit_accessible(unit_id) {
        return Err(AppError::Forbidden(format!(
            "unit '{}' outside caller's authorized organizational scope",
            unit_id
        )));
    }

    let rows = store.assignments_for_unit_as_of(unit_id, date).await?;
    Ok(rows.iter().map(AssignmentDto::from).collect())
}

/// Transfer history, optionally filtered by agent.
pub async fn transfer_history<S: OrgStore>(
    store: &S,
    agent_id: Option<EntityId>,
    limit: u32,
    principal: &Principal,
) -> Result<Vec<iaop_agency_service_domain::assignment::Transfer>, AppError> {
    permissions::require(principal, permissions::Permission::TransferRequest)?;
    let today = chrono::Utc::now().date_naive();
    let scope = authz::resolve_scope(store, principal, today).await?;

    match &scope {
        OrgScope::Global => {}
        OrgScope::Scoped {
            agent_id: self_id, ..
        } => {
            if agent_id != Some(*self_id) {
                return Err(AppError::Forbidden(
                    "non-staff callers may only view their own transfer history".into(),
                ));
            }
        }
    }

    let rows = store.list_transfers(agent_id, limit.min(200)).await?;
    Ok(rows)
}

pub async fn leaders_as_of<S: OrgStore>(
    store: &S,
    unit_id: EntityId,
    as_of: Option<NaiveDate>,
    principal: &Principal,
) -> Result<Vec<crate::commands::LeadershipDto>, AppError> {
    permissions::require(principal, permissions::Permission::LeadershipManage)?;
    let date = as_of.unwrap_or_else(|| chrono::Utc::now().date_naive());
    let scope = authz::resolve_scope(store, principal, date).await?;

    if !scope.is_unit_accessible(unit_id) {
        return Err(AppError::Forbidden(format!(
            "unit '{}' outside caller's authorized organizational scope",
            unit_id
        )));
    }

    let rows = store.leaders_as_of(unit_id, date).await?;
    Ok(rows
        .iter()
        .map(crate::commands::LeadershipDto::from)
        .collect())
}
