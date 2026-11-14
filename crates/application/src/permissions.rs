//! Authorization: maps Keycloak roles/scopes to technical permissions.
//! Resource-level (org-scope) checks remain the service's job.

use crate::error::AppError;
use crate::principal::Principal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    AgencyRead,
    AgencyCreate,
    AgencyUpdate,
    HierarchyManage,
    AssignmentRead,
    AssignmentManage,
    TransferRequest,
    TransferApprove,
    LeadershipManage,
}

impl Permission {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AgencyRead => "agency:read",
            Self::AgencyCreate => "agency:create",
            Self::AgencyUpdate => "agency:update",
            Self::HierarchyManage => "agency:hierarchy:manage",
            Self::AssignmentRead => "agency:assignment:read",
            Self::AssignmentManage => "agency:assignment:manage",
            Self::TransferRequest => "agency:transfer:request",
            Self::TransferApprove => "agency:transfer:approve",
            Self::LeadershipManage => "agency:leadership:manage",
        }
    }
}

/// Full Phase-1 permission set granted to platform/agency admins.
pub const ADMIN_PERMISSIONS: &[&str] = &[
    "agency:read",
    "agency:create",
    "agency:update",
    "agency:hierarchy:manage",
    "agency:assignment:read",
    "agency:assignment:manage",
    "agency:transfer:request",
    "agency:transfer:approve",
    "agency:leadership:manage",
];

/// Requires `perm` in the principal's effective permissions (realm roles
/// merged with audience-scoped client roles upstream).
pub fn require(principal: &Principal, perm: Permission) -> Result<(), AppError> {
    if principal.has_permission(perm.as_str()) || principal.is_admin() {
        Ok(())
    } else {
        Err(AppError::Forbidden(format!(
            "missing permission {}",
            perm.as_str()
        )))
    }
}

/// Read access: `agency:read` or any manage permission implies read.
pub fn require_read(principal: &Principal) -> Result<(), AppError> {
    if principal.has_permission("agency:read")
        || principal.has_permission("agency:hierarchy:manage")
        || principal.is_admin()
    {
        Ok(())
    } else {
        Err(AppError::Forbidden("missing permission agency:read".into()))
    }
}
