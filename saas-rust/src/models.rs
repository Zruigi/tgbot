use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    PlatformAdmin,
    MerchantOwner,
    MerchantAdmin,
    MerchantViewer,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::PlatformAdmin => "platform_admin",
            Role::MerchantOwner => "merchant_owner",
            Role::MerchantAdmin => "merchant_admin",
            Role::MerchantViewer => "merchant_viewer",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Scope {
    pub authenticated: bool,
    pub user_id: Option<Uuid>,
    pub role: Role,
    pub actor_role: Role,
    pub tenant_id: Option<String>,
    pub merchant_id: Option<Uuid>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Merchant {
    pub id: Uuid,
    pub legacy_merchant_id: Option<i64>,
    pub tenant_id: String,
    pub owner_user_id: Option<Uuid>,
    pub owner_tg_user_id: Option<i64>,
    pub owner_username: Option<String>,
    pub merchant_name: String,
    pub bot_id: Option<i64>,
    pub bot_username: Option<String>,
    pub status: String,
    pub plan_type: String,
    pub subscription_status: String,
    pub subscription_expires_at: Option<DateTime<Utc>>,
    pub onboarding_completed: bool,
    pub runtime_version: i32,
    pub config_version: i32,
    pub feature_version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateMerchantRequest {
    pub owner_user_id: Option<Uuid>,
    pub owner_tg_user_id: Option<i64>,
    pub owner_username: Option<String>,
    pub merchant_name: String,
    pub tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertPlanRequest {
    pub plan_code: String,
    pub plan_name: String,
    pub price_usdt: f64,
    pub duration_days: i32,
    pub features_json: Value,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateStatusRequest {
    pub status: String,
    pub onboarding_completed: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct SaveSecretRefRequest {
    pub bot_token: String,
}

#[derive(Debug, Deserialize)]
pub struct PlanCodeRequest {
    pub plan_code: String,
}

#[derive(Debug, Deserialize)]
pub struct OrderIdRequest {
    pub order_id: Uuid,
}
