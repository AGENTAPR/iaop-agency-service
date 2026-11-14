//! Organizational unit aggregate: effective-dated hierarchy node.
//! Parent references are temporal — history is preserved by closing
//! the old relationship and opening a new one.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::entity_id::EntityId;
use crate::error::DomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UnitKind {
    Region,
    Branch,
    Department,
    Team,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UnitStatus {
    Active,
    Inactive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgUnit {
    pub id: EntityId,
    pub agency_id: EntityId,
    pub code: String,
    pub name: String,
    pub kind: UnitKind,
    pub status: UnitStatus,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A temporal parent-child edge between two org units.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgRelationship {
    pub id: EntityId,
    pub parent_id: EntityId,
    pub child_id: EntityId,
    pub effective_from: NaiveDate,
    /// Exclusive upper bound; `None` = open-ended.
    pub effective_to: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
}

impl OrgRelationship {
    pub fn covers(&self, date: NaiveDate) -> bool {
        self.effective_from <= date && self.effective_to.is_none_or(|t| date < t)
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum OrgUnitEvent {
    UnitCreated {
        unit_id: EntityId,
        agency_id: EntityId,
        code: String,
    },
    UnitUpdated {
        unit_id: EntityId,
        version: i64,
    },
    ParentChanged {
        unit_id: EntityId,
        old_parent: Option<EntityId>,
        new_parent: Option<EntityId>,
        effective_from: NaiveDate,
    },
    UnitStatusChanged {
        unit_id: EntityId,
        status: UnitStatus,
    },
}

impl OrgUnit {
    pub fn create(
        agency_id: EntityId,
        code: impl Into<String>,
        name: impl Into<String>,
        kind: UnitKind,
    ) -> Result<Self, DomainError> {
        let code = code.into();
        let name = name.into();
        if code.trim().is_empty() || code.len() > 32 {
            return Err(DomainError::validation("unit code must be 1..=32 chars"));
        }
        if name.trim().is_empty() || name.len() > 200 {
            return Err(DomainError::validation("unit name must be 1..=200 chars"));
        }
        let now = Utc::now();
        Ok(Self {
            id: EntityId::new(),
            agency_id,
            code,
            name,
            kind,
            status: UnitStatus::Active,
            version: 1,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn rename(&mut self, name: impl Into<String>) -> Result<(), DomainError> {
        if self.status == UnitStatus::Inactive {
            return Err(DomainError::InvalidState);
        }
        let name = name.into();
        if name.trim().is_empty() || name.len() > 200 {
            return Err(DomainError::validation("unit name must be 1..=200 chars"));
        }
        self.name = name;
        self.touch();
        Ok(())
    }

    pub fn deactivate(&mut self) -> Result<(), DomainError> {
        if self.status == UnitStatus::Inactive {
            return Err(DomainError::InvalidState);
        }
        self.status = UnitStatus::Inactive;
        self.touch();
        Ok(())
    }

    pub fn activate(&mut self) -> Result<(), DomainError> {
        if self.status == UnitStatus::Active {
            return Err(DomainError::InvalidState);
        }
        self.status = UnitStatus::Active;
        self.touch();
        Ok(())
    }

    fn touch(&mut self) {
        self.version += 1;
        self.updated_at = Utc::now();
    }

    /// Public version bump for store-level updates (e.g. reparent).
    pub fn bump_version(&mut self) {
        self.touch();
    }
}

/// True if making `child_id` a child of `new_parent_id` would create a
/// cycle. `ancestors_of_new_parent` must contain the new parent's ancestor
/// chain (excluding itself is fine — equality is checked separately).
pub fn would_create_cycle(
    child_id: EntityId,
    new_parent_id: EntityId,
    ancestors_of_new_parent: &[EntityId],
) -> bool {
    child_id == new_parent_id || ancestors_of_new_parent.contains(&child_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relationship_covers_exclusive_end() {
        let r = OrgRelationship {
            id: EntityId::new(),
            parent_id: EntityId::new(),
            child_id: EntityId::new(),
            effective_from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            effective_to: Some(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()),
            created_at: Utc::now(),
        };
        assert!(r.covers(NaiveDate::from_ymd_opt(2026, 3, 31).unwrap()));
        assert!(!r.covers(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()));
    }

    #[test]
    fn cycle_detection() {
        let a = EntityId::new();
        let b = EntityId::new();
        assert!(would_create_cycle(a, a, &[]));
        assert!(would_create_cycle(a, b, &[b, a]));
        assert!(!would_create_cycle(a, b, &[b, EntityId::new()]));
    }

    #[test]
    fn unit_lifecycle() {
        let u = OrgUnit::create(EntityId::new(), "BR-01", "Branch", UnitKind::Branch).unwrap();
        assert_eq!(u.status, UnitStatus::Active);
        let mut u = u;
        u.deactivate().unwrap();
        assert!(u.rename("x").is_err());
        u.activate().unwrap();
        assert!(u.rename("y").is_ok());
    }
}
