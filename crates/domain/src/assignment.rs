//! Agent organizational assignment aggregate + transfer record.
//! `agent_id` is an external reference to Agent Registry — no FK.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::entity_id::EntityId;
use crate::error::DomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AssignmentType {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AssignmentStatus {
    Pending,
    Active,
    Ended,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assignment {
    pub id: EntityId,
    /// External Agent Registry reference — validated at the boundary.
    pub agent_id: EntityId,
    pub unit_id: EntityId,
    pub assignment_type: AssignmentType,
    pub status: AssignmentStatus,
    pub effective_from: NaiveDate,
    /// Exclusive upper bound; `None` = open-ended.
    pub effective_to: Option<NaiveDate>,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Assignment {
    pub fn create(
        agent_id: EntityId,
        unit_id: EntityId,
        assignment_type: AssignmentType,
        effective_from: NaiveDate,
    ) -> Result<Self, DomainError> {
        let now = Utc::now();
        Ok(Self {
            id: EntityId::new(),
            agent_id,
            unit_id,
            assignment_type,
            status: AssignmentStatus::Pending,
            effective_from,
            effective_to: None,
            version: 1,
            created_at: now,
            updated_at: now,
        })
    }

    /// Activates a pending assignment whose effective date has arrived.
    pub fn activate(&mut self, as_of: NaiveDate) -> Result<(), DomainError> {
        if self.status != AssignmentStatus::Pending {
            return Err(DomainError::InvalidState);
        }
        if as_of < self.effective_from {
            return Err(DomainError::validation("assignment not yet effective"));
        }
        self.status = AssignmentStatus::Active;
        self.touch();
        Ok(())
    }

    /// Ends the assignment at `effective_to` (exclusive bound).
    pub fn end(&mut self, effective_to: NaiveDate) -> Result<(), DomainError> {
        if matches!(
            self.status,
            AssignmentStatus::Ended | AssignmentStatus::Cancelled
        ) {
            return Err(DomainError::InvalidState);
        }
        if effective_to <= self.effective_from {
            return Err(DomainError::validation(
                "effective_to must be after effective_from",
            ));
        }
        self.effective_to = Some(effective_to);
        self.status = AssignmentStatus::Ended;
        self.touch();
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<(), DomainError> {
        if self.status == AssignmentStatus::Active {
            return Err(DomainError::InvalidState);
        }
        self.status = AssignmentStatus::Cancelled;
        self.touch();
        Ok(())
    }

    /// True if this assignment covers `date`.
    pub fn covers(&self, date: NaiveDate) -> bool {
        self.effective_from <= date
            && self.effective_to.is_none_or(|t| date < t)
            && !matches!(self.status, AssignmentStatus::Cancelled)
    }

    fn touch(&mut self) {
        self.version += 1;
        self.updated_at = Utc::now();
    }
}

/// Immutable record of an agent transfer between organizational units.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Transfer {
    pub id: EntityId,
    pub agent_id: EntityId,
    pub from_unit_id: Option<EntityId>,
    pub to_unit_id: EntityId,
    pub effective_from: NaiveDate,
    pub reason: Option<String>,
    pub status: AssignmentStatus,
    pub version: i64,
    pub created_at: DateTime<Utc>,
}

impl Transfer {
    pub fn record(
        agent_id: EntityId,
        from_unit_id: Option<EntityId>,
        to_unit_id: EntityId,
        effective_from: NaiveDate,
        reason: Option<String>,
    ) -> Self {
        Self {
            id: EntityId::new(),
            agent_id,
            from_unit_id,
            to_unit_id,
            effective_from,
            reason,
            status: AssignmentStatus::Pending,
            version: 1,
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignment_lifecycle() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let mut a =
            Assignment::create(EntityId::new(), EntityId::new(), AssignmentType::Primary, d)
                .unwrap();
        assert_eq!(a.status, AssignmentStatus::Pending);
        a.activate(d).unwrap();
        assert_eq!(a.status, AssignmentStatus::Active);
        a.end(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()).unwrap();
        assert_eq!(a.status, AssignmentStatus::Ended);
        assert!(!a.covers(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()));
    }

    #[test]
    fn cannot_end_before_start() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let mut a =
            Assignment::create(EntityId::new(), EntityId::new(), AssignmentType::Primary, d)
                .unwrap();
        a.activate(d).unwrap();
        assert!(a.end(d).is_err());
    }
}
