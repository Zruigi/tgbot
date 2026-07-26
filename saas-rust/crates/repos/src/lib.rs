pub mod admin_repo;
pub mod ai_usage_repo;
pub mod audit_repo;
pub mod broadcast_repo;
pub mod feature_repo;
pub mod finance_repo;
pub mod group_repo;
pub mod member_repo;
pub mod merchant_repo;
pub mod message_repo;
pub mod monitor_repo;
pub mod payment_repo;
pub mod push_task_repo;
pub mod quota_repo;
pub mod subscription_repo;
pub mod user_repo;
pub mod verify_repo;

use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use sqlx::{postgres::PgRow, Column, Row, TypeInfo};

// ---------------------------------------------------------------------------
// Helper: convert a PgRow into a JSON Value.
// Copied from tgbot_http routes/platform.rs for reuse across repos.
// ---------------------------------------------------------------------------
pub fn row_to_json(row: PgRow) -> Value {
    let mut map = Map::new();
    for col in row.columns() {
        let name = col.name();
        let type_name = col.type_info().name();
        let value = match type_name {
            "UUID" => row
                .try_get::<uuid::Uuid, _>(name)
                .map(|v| Value::String(v.to_string())),
            "TEXT" | "VARCHAR" | "BPCHAR" => row.try_get::<String, _>(name).map(Value::String),
            "INT2" => row.try_get::<i16, _>(name).map(|v| json!(v)),
            "INT4" => row.try_get::<i32, _>(name).map(|v| json!(v)),
            "INT8" => row.try_get::<i64, _>(name).map(|v| json!(v)),
            "FLOAT4" => row.try_get::<f32, _>(name).map(|v| json!(v)),
            "FLOAT8" => row.try_get::<f64, _>(name).map(|v| json!(v)),
            "NUMERIC" => row
                .try_get::<rust_decimal::Decimal, _>(name)
                .map(|v| json!(v.to_string().parse::<f64>().unwrap_or_default())),
            "BOOL" => row.try_get::<bool, _>(name).map(|v| json!(v)),
            "TIMESTAMPTZ" => row
                .try_get::<DateTime<Utc>, _>(name)
                .map(|v| Value::String(v.to_rfc3339())),
            "JSON" | "JSONB" => Ok(row.try_get::<Value, _>(name).ok().unwrap_or(Value::Null)),
            _ => Ok(row
                .try_get::<String, _>(name)
                .map(Value::String)
                .unwrap_or(Value::Null)),
        }
        .unwrap_or(Value::Null);
        map.insert(name.to_string(), value);
    }
    Value::Object(map)
}
