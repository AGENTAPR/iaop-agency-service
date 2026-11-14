//! Leadership appointments: effective-dated leadership roles on org units.
//! Reporting hierarchy != compensation hierarchy; leadership is a separate
//! concern stored as its own aggregate records.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::entity_id::EntityId;
use crate::error::DomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LeadershipRole {
    UnitHead,
    RegionalManager,
    AgencyDirector,
}

impl LeadershipRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UnitHead => "UNIT_HEAD",
            Self::RegionalManager => "REGIONAL_MANAGER",
            Self::AgencyDirector => "AGENCY_DIRECTOR",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "UNIT_HEAD" => Some(Self::UnitHead),
            "REGIONAL_MANAGER" => Some(Self::RegionalManager),
            "AGENCY_DIRECTOR" => Some(Self::AgencyDirector),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LeadershipStatus {
    Pending,
    Active,
    Ended,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeadershipAppointment {
    pub id: EntityId,
    pub unit_id: EntityId,
    /// External reference to Agent Registry — no cross-service FK.
    pub agent_id: EntityId,
    pub role: LeadershipRole,
    pub status: LeadershipStatus,
    pub effective_from: NaiveDate,
    /// Exclusive upper bound; `None` = open-ended.
    pub effective_to: Option<NaiveDate>,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl LeadershipAppointment {
    pub fn create(
        unit_id: EntityId,
        agent_id: EntityId,
        role: LeadershipRole,
        effective_from: NaiveDate,
    ) -> Result<Self, DomainError> {
        if effective_from > Utc::now().date_naive() + chrono::Duration::days(366) {
            return Err(DomainError::validation("effective_from too far in future"));
        }
        Ok(Self {
            id: EntityId::new(),
            unit_id,
            agent_id,
            role,
            status: LeadershipStatus::Pending,
            effective_from,
            effective_to: None,
            version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    /// Activate when the effective date has arrived.
    pub fn activate(&mut self, today: NaiveDate) -> Result<(), DomainError> {
        match self.status {
            LeadershipStatus::Pending if self.effective_from <= today => {
                self.status = LeadershipStatus::Active;
                self.bump_version();
                Ok(())
            }
            LeadershipStatus::Pending => {
                Err(DomainError::invalid_state("appointment not yet effective"))
            }
            _ => Err(DomainError::invalid_state("appointment not pending")),
        }
    }

    /// End at `effective_to` (exclusive). History is preserved.
    pub fn end(&mut self, effective_to: NaiveDate) -> Result<(), DomainError> {
        if self.status != LeadershipStatus::Active {
            return Err(DomainError::invalid_state(
                "only active appointments can end",
            ));
        }
        if effective_to <= self.effective_from {
            return Err(DomainError::validation(
                "effective_to must be after effective_from",
            ));
        }
        self.status = LeadershipStatus::Ended;
        self.effective_to = Some(effective_to);
        self.bump_version();
        Ok(())
    }

    pub fn covers(&self, date: NaiveDate) -> bool {
        self.effective_from <= date && self.effective_to.is_none_or(|t| date < t)
    }

    pub fn bump_version(&mut self) {
        self.version += 1;
        self.updated_at = Utc::now();
    }
}
