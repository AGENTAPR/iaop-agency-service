//! Resource-level organizational scope authorization for Agency Service.
//! Enforces: Authentication + Technical Permission + Organizational Scope + Temporal Invariants.

use std::collections::HashSet;

use chrono::NaiveDate;
use iaop_agency_service_domain::entity_id::EntityId;
use iaop_agency_service_domain::leadership::LeadershipStatus;

use crate::error::AppError;
use crate::ports::OrgStore;
use crate::principal::Principal;

pub const STAFF_AGENCY_ROLES: &[&str] = &[
    "IAOP_PLATFORM_ADMIN",
    "IAOP_AGENCY_ADMIN",
    "IAOP_OPERATIONS_ADMIN",
    "PLATFORM_ADMIN",
    "AGENCY_ADMIN",
    "AGENCY_OPERATIONS",
    "OPERATIONS_ADMIN",
    "AGENCY_AUDITOR",
];

pub fn is_staff(principal: &Principal) -> bool {
    principal.is_admin()
        || principal
            .roles
            .iter()
            .any(|r| STAFF_AGENCY_ROLES.contains(&r.as_str()))
}

#[derive(Debug, Clone)]
pub enum OrgScope {
    /// Staff / Admin: global unrestricted agency access
    Global,
    /// Non-staff agent: scoped to caller's agent ID, assigned unit, ancestors, and leadership descendants
    Scoped {
        agent_id: EntityId,
        assigned_units: Vec<EntityId>,
        leadership_units: Vec<EntityId>,
        accessible_units: HashSet<EntityId>,
        accessible_agencies: HashSet<EntityId>,
    },
}

impl OrgScope {
    pub fn is_agency_accessible(&self, agency_id: EntityId) -> bool {
        match self {
            Self::Global => true,
            Self::Scoped {
                accessible_agencies,
                ..
            } => accessible_agencies.contains(&agency_id),
        }
    }

    pub fn is_unit_accessible(&self, unit_id: EntityId) -> bool {
        match self {
            Self::Global => true,
            Self::Scoped {
                accessible_units, ..
            } => accessible_units.contains(&unit_id),
        }
    }

    pub fn is_agent_accessible(&self, target_agent_id: EntityId) -> bool {
        match self {
            Self::Global => true,
            Self::Scoped { agent_id, .. } => *agent_id == target_agent_id,
        }
    }
}

pub async fn resolve_scope<S: OrgStore>(
    store: &S,
    principal: &Principal,
    as_of: NaiveDate,
) -> Result<OrgScope, AppError> {
    if is_staff(principal) {
        return Ok(OrgScope::Global);
    }

    let agent_id = principal.resolved_agent_id().ok_or_else(|| {
        AppError::Forbidden("unlinked principal has no organizational scope".into())
    })?;

    // Historical, pending, ended, or cancelled assignments do not grant current access
    let assignments = store.assignments_for_agent_as_of(agent_id, as_of).await?;
    if assignments.is_empty() {
        return Err(AppError::Forbidden("no active agency assignment".into()));
    }

    let mut accessible_units = HashSet::new();
    let mut accessible_agencies = HashSet::new();
    let mut assigned_units = Vec::new();
    let mut leadership_units = Vec::new();

    for a in &assignments {
        assigned_units.push(a.unit_id);
        accessible_units.insert(a.unit_id);

        if let Some(u) = store.get_unit(a.unit_id).await? {
            accessible_agencies.insert(u.agency_id);
        }

        let ancestors = store.ancestors_as_of(a.unit_id, as_of).await?;
        for ancestor in ancestors {
            accessible_units.insert(ancestor);
            if let Some(u) = store.get_unit(ancestor).await? {
                accessible_agencies.insert(u.agency_id);
            }
        }

        let leaders = store.leaders_as_of(a.unit_id, as_of).await?;
        if leaders
            .iter()
            .any(|l| l.agent_id == agent_id && l.status == LeadershipStatus::Active)
        {
            leadership_units.push(a.unit_id);
            let descendants = store.descendants(a.unit_id).await?;
            for descendant in descendants {
                accessible_units.insert(descendant);
            }
        }
    }

    Ok(OrgScope::Scoped {
        agent_id,
        assigned_units,
        leadership_units,
        accessible_units,
        accessible_agencies,
    })
}

// ---------------------------------------------------------------------------
// Mutation scope guards
// ---------------------------------------------------------------------------
// Authorization for writes is evaluated at request time (today), not at the
// command's effective date: a caller must hold organizational scope NOW.

/// Staff-only guard for top-level administrative creation (e.g. new agency).
pub async fn require_global<S: OrgStore>(store: &S, principal: &Principal) -> Result<(), AppError> {
    let today = chrono::Utc::now().date_naive();
    match resolve_scope(store, principal, today).await? {
        OrgScope::Global => Ok(()),
        OrgScope::Scoped { .. } => Err(AppError::Forbidden(
            "organizational mutations require staff scope".into(),
        )),
    }
}

/// Scoped callers may only mutate agencies inside their effective scope.
pub async fn require_agency_scope<S: OrgStore>(
    store: &S,
    principal: &Principal,
    agency_id: EntityId,
) -> Result<(), AppError> {
    let today = chrono::Utc::now().date_naive();
    let scope = resolve_scope(store, principal, today).await?;
    if scope.is_agency_accessible(agency_id) {
        Ok(())
    } else {
        Err(AppError::Forbidden(format!(
            "agency '{agency_id}' outside caller's authorized organizational scope"
        )))
    }
}

/// Scoped callers may only mutate units inside their effective scope.
pub async fn require_unit_scope<S: OrgStore>(
    store: &S,
    principal: &Principal,
    unit_id: EntityId,
) -> Result<(), AppError> {
    let today = chrono::Utc::now().date_naive();
    let scope = resolve_scope(store, principal, today).await?;
    if scope.is_unit_accessible(unit_id) {
        Ok(())
    } else {
        Err(AppError::Forbidden(format!(
            "unit '{unit_id}' outside caller's authorized organizational scope"
        )))
    }
}
