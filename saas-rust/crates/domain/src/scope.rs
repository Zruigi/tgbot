use uuid::Uuid;

use crate::models::Role;

/// Authorization scope resolved from request headers/token.
#[derive(Debug, Clone)]
pub struct Scope {
    pub authenticated: bool,
    pub user_id: Option<Uuid>,
    pub actor_key: Option<String>,
    pub role: Role,
    pub actor_role: Role,
    pub tenant_id: Option<String>,
    pub merchant_id: Option<Uuid>,
}

impl Scope {
    pub fn require_platform(&self) -> Result<(), crate::error::ApiError> {
        if self.role == Role::PlatformAdmin {
            Ok(())
        } else {
            Err(crate::error::ApiError::Forbidden)
        }
    }

    pub fn require_merchant(&self) -> Result<(String, Uuid), crate::error::ApiError> {
        match (&self.tenant_id, self.merchant_id) {
            (Some(tenant_id), Some(merchant_id))
                if matches!(
                    self.role,
                    Role::MerchantOwner | Role::MerchantAdmin | Role::MerchantViewer
                ) =>
            {
                Ok((tenant_id.clone(), merchant_id))
            }
            _ => Err(crate::error::ApiError::Forbidden),
        }
    }

    /// Resolve the tenant scope for read-only finance views.
    ///
    /// Platform admins may view aggregate finance data across the platform
    /// (tenant_id is empty, so queries must not filter by tenant).
    /// Merchant roles are restricted to their own tenant.
    /// Returns the tenant_id to filter by (empty string = platform-wide).
    pub fn require_tenant_view(&self) -> Result<String, crate::error::ApiError> {
        if self.role == Role::PlatformAdmin {
            return Ok(String::new());
        }
        if matches!(
            self.role,
            Role::MerchantOwner | Role::MerchantAdmin | Role::MerchantViewer
        ) {
            if let Some(tenant_id) = self.tenant_id.as_deref() {
                if !tenant_id.is_empty() {
                    return Ok(tenant_id.to_string());
                }
            }
        }
        Err(crate::error::ApiError::Forbidden)
    }
}
