//! Agency aggregate: the top-level organization entity.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::entity_id::EntityId;
use crate::error::DomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgencyStatus {
    Active,
    Suspended,
    Terminated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agency {
    pub id: EntityId,
    pub code: String,
    pub name: String,
    pub status: AgencyStatus,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub enum AgencyEvent {
    AgencyCreated {
        agency_id: EntityId,
        code: String,
    },
    AgencyUpdated {
        agency_id: EntityId,
        version: i64,
    },
    AgencyStatusChanged {
        agency_id: EntityId,
        status: AgencyStatus,
    },
}

impl Agency {
    pub fn create(code: impl Into<String>, name: impl Into<String>) -> Result<Self, DomainError> {
        let code = code.into();
        let name = name.into();
        if code.trim().is_empty() || code.len() > 32 {
            return Err(DomainError::validation("agency code must be 1..=32 chars"));
        }
        if name.trim().is_empty() || name.len() > 200 {
            return Err(DomainError::validation("agency name must be 1..=200 chars"));
        }
        let now = Utc::now();
        Ok(Self {
            id: EntityId::new(),
            code,
            name,
            status: AgencyStatus::Active,
            version: 1,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn rename(&mut self, name: impl Into<String>) -> Result<(), DomainError> {
        if self.status == AgencyStatus::Terminated {
            return Err(DomainError::InvalidState);
        }
        let name = name.into();
        if name.trim().is_empty() || name.len() > 200 {
            return Err(DomainError::validation("agency name must be 1..=200 chars"));
        }
        self.name = name;
        self.touch();
        Ok(())
    }

    pub fn suspend(&mut self) -> Result<(), DomainError> {
        if self.status != AgencyStatus::Active {
            return Err(DomainError::InvalidState);
        }
        self.status = AgencyStatus::Suspended;
        self.touch();
        Ok(())
    }

    pub fn reinstate(&mut self) -> Result<(), DomainError> {
        if self.status != AgencyStatus::Suspended {
            return Err(DomainError::InvalidState);
        }
        self.status = AgencyStatus::Active;
        self.touch();
        Ok(())
    }

    pub fn terminate(&mut self) -> Result<(), DomainError> {
        if self.status == AgencyStatus::Terminated {
            return Err(DomainError::InvalidState);
        }
        self.status = AgencyStatus::Terminated;
        self.touch();
        Ok(())
    }

    fn touch(&mut self) {
        self.version += 1;
        self.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_rename() {
        let mut a = Agency::create("AGT-001", "Test Agency").unwrap();
        assert_eq!(a.status, AgencyStatus::Active);
        a.rename("New Name").unwrap();
        assert_eq!(a.name, "New Name");
        assert_eq!(a.version, 2);
    }

    #[test]
    fn invalid_code_rejected() {
        assert!(Agency::create("", "x").is_err());
        let long = "c".repeat(33);
        assert!(Agency::create(long.as_str(), "x").is_err());
    }

    #[test]
    fn terminate_is_terminal() {
        let mut a = Agency::create("A1", "n").unwrap();
        a.terminate().unwrap();
        assert!(a.rename("x").is_err());
        assert!(a.suspend().is_err());
    }
}
