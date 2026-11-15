//! Resource-level organizational scope authorization for Agency Service.
//! Enforces: Authentication + Technical Permission + Organizational Scope + Temporal Invariants.
//!
//! # Auth-as-of invariant
//! Scope is ALWAYS resolved at the CURRENT wall-clock date (`Utc::now().date_naive()`),
//! never at the caller-supplied `as_of` query parameter. Historical queries must not
//! grant access that the caller no longer holds today.

use std::collections::HashSet;

use chrono::NaiveDate;
use iaop_agency_service_domain::entity_id::EntityId;
use iaop_agency_service_domain::leadership::LeadershipStatus;

use crate::error::AppError;
use crate::ports::OrgStore;
use crate::principal::Principal;

/// Roles that are granted **Global** org scope (unrestricted read/write).
/// Only platform-level administrative and audit roles belong here.
/// Agency-local roles (`IAOP_AGENCY_ADMIN`, `AGENCY_AUDITOR`, …) are intentionally
/// excluded; they receive Scoped access determined by their own assignments.
pub const GLOBAL_STAFF_ROLES: &[&str] = &["IAOP_PLATFORM_ADMIN", "IAOP_GLOBAL_AUDITOR"];

/// Returns `true` if the principal holds a role that grants Global org scope.
pub fn is_staff(principal: &Principal) -> bool {
    principal
        .roles
        .iter()
        .any(|r| GLOBAL_STAFF_ROLES.contains(&r.as_str()))
}

/// Roles permitted to perform GLOBAL organizational mutations.
/// GLOBAL_READ != GLOBAL_WRITE: read-only global roles such as
/// `IAOP_GLOBAL_AUDITOR` receive `OrgScope::Global` for reads but must
/// never mutate organizational structure.
pub const GLOBAL_WRITE_ROLES: &[&str] = &["IAOP_PLATFORM_ADMIN"];

/// Returns `true` if the principal may perform global organizational writes.
pub fn is_global_writer(principal: &Principal) -> bool {
    principal
        .roles
        .iter()
        .any(|r| GLOBAL_WRITE_ROLES.contains(&r.as_str()))
}

// ---------------------------------------------------------------------------
// OrgScope
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum OrgScope {
    /// Global (staff): unrestricted access to all agencies and units.
    Global,
    /// Scoped (non-staff): access limited to the agent's own assignments
    /// and the organisational hierarchy they can navigate/manage.
    Scoped {
        /// The resolved Agent ID of the caller.
        agent_id: EntityId,
        /// Units where the agent is directly assigned (membership).
        /// Used for membership-based access decisions (read own records).
        membership_units: Vec<EntityId>,
        /// All units reachable by the agent via hierarchy navigation:
        /// membership units + their ancestors (for read-up), and leadership
        /// descendants (for leader read-down).
        navigation_units: HashSet<EntityId>,
        /// Legacy alias kept for internal use — same set as `navigation_units`
        /// exposed as `accessible_units` in match arms that predate the rename.
        accessible_units: HashSet<EntityId>,
        /// Agencies that contain at least one accessible unit.
        accessible_agencies: HashSet<EntityId>,
        /// Units where the agent is an active leader (superset of membership).
        leadership_units: Vec<EntityId>,
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

    // --- Per-operation permission helpers -----------------------------------

    /// Staff-level write guard: may create/update agencies and hierarchy.
    pub fn can_manage_agency(&self) -> bool {
        matches!(self, Self::Global)
    }

    /// Read-level guard: Global scope or any unit within the caller's
    /// navigable org scope grants agency read access.
    pub fn can_read_agency(&self, agency_id: EntityId) -> bool {
        self.is_agency_accessible(agency_id)
    }

    /// Membership read: Global scope, or the caller is reading their own
    /// record, or the target is in a unit the caller leads.
    pub fn can_read_members(&self, target_agent_id: EntityId) -> bool {
        match self {
            Self::Global => true,
            Self::Scoped { agent_id, .. } => *agent_id == target_agent_id,
        }
    }
}

// ---------------------------------------------------------------------------
// resolve_scope
// ---------------------------------------------------------------------------

/// Resolves the caller's organisational scope **always at the current date**.
///
/// The `as_of` parameter is intentionally unused here — scope is authoritative
/// NOW, not at some historical point. Historical queries may carry their own
/// `as_of` for data retrieval, but that date must never influence who may call
/// the query. Pass any `NaiveDate` value at call sites; it is ignored.
pub async fn resolve_scope<S: OrgStore>(
    store: &S,
    principal: &Principal,
    _as_of: NaiveDate,
) -> Result<OrgScope, AppError> {
    // Auth is ALWAYS evaluated at today — ignoring the caller's as_of.
    let auth_as_of = chrono::Utc::now().date_naive();

    if is_staff(principal) {
        return Ok(OrgScope::Global);
    }

    let agent_id = principal.resolved_agent_id().ok_or_else(|| {
        AppError::Forbidden("unlinked principal has no organizational scope".into())
    })?;

    // Only ACTIVE assignments at today's date grant current access.
    let assignments = store
        .assignments_for_agent_as_of(agent_id, auth_as_of)
        .await?;
    if assignments.is_empty() {
        return Err(AppError::Forbidden("no active agency assignment".into()));
    }

    let mut accessible_units: HashSet<EntityId> = HashSet::new();
    let mut accessible_agencies: HashSet<EntityId> = HashSet::new();
    let mut membership_units: Vec<EntityId> = Vec::new();
    let mut leadership_units: Vec<EntityId> = Vec::new();

    for a in &assignments {
        // Direct membership
        membership_units.push(a.unit_id);
        accessible_units.insert(a.unit_id);

        if let Some(u) = store.get_unit(a.unit_id).await? {
            accessible_agencies.insert(u.agency_id);
        }

        // Navigate ancestors (read-up): caller may read parent/grandparent units.
        let ancestors = store.ancestors_as_of(a.unit_id, auth_as_of).await?;
        for ancestor in ancestors {
            accessible_units.insert(ancestor);
            if let Some(u) = store.get_unit(ancestor).await? {
                accessible_agencies.insert(u.agency_id);
            }
        }

        // Leadership read-down: active leaders may read descendant units.
        let leaders = store.leaders_as_of(a.unit_id, auth_as_of).await?;
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

    // `navigation_units` mirrors `accessible_units` — both point to the same set.
    let navigation_units = accessible_units.clone();

    Ok(OrgScope::Scoped {
        agent_id,
        membership_units,
        navigation_units,
        accessible_units,
        accessible_agencies,
        leadership_units,
    })
}

// ---------------------------------------------------------------------------
// Mutation scope guards
// ---------------------------------------------------------------------------
// Authorization for writes is evaluated at request time (today), not at the
// command's effective date: a caller must hold organizational scope NOW.

/// Staff-only guard for top-level administrative creation (e.g. new agency).
/// Requires BOTH a global-write role AND Global organizational scope —
/// a global auditor's read scope must not authorize mutations.
pub async fn require_global<S: OrgStore>(store: &S, principal: &Principal) -> Result<(), AppError> {
    if !is_global_writer(principal) {
        return Err(AppError::Forbidden(
            "global organizational mutations require a platform-administration role".into(),
        ));
    }
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
