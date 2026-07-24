//! Dual-transport database helpers.
//!
//! Each function tries PostgreSQL first; falls back to Supabase REST when PG is unavailable.
//! Routes call these instead of raw sqlx, so REST-only mode works transparently.

use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use sqlx::{postgres::PgRow, Column, Row, TypeInfo};

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Helper: convert a PgRow into a JSON Value.
// Copied from the verified pattern in routes/platform.rs and repos/src/lib.rs.
// ---------------------------------------------------------------------------
fn row_to_json(row: PgRow) -> Value {
    let mut map = Map::new();
    for col in row.columns() {
        let name = col.name();
        let type_name = col.type_info().name();
        let value = match type_name {
            "UUID" => row.try_get::<uuid::Uuid, _>(name).map(|v| Value::String(v.to_string())),
            "TEXT" | "VARCHAR" | "BPCHAR" => row.try_get::<String, _>(name).map(Value::String),
            "INT2" => row.try_get::<i16, _>(name).map(|v| json!(v)),
            "INT4" => row.try_get::<i32, _>(name).map(|v| json!(v)),
            "INT8" => row.try_get::<i64, _>(name).map(|v| json!(v)),
            "FLOAT4" => row.try_get::<f32, _>(name).map(|v| json!(v)),
            "FLOAT8" | "NUMERIC" => row.try_get::<f64, _>(name).map(|v| json!(v)),
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

fn rows_to_json(rows: Vec<PgRow>) -> Vec<Value> {
    rows.into_iter().map(row_to_json).collect()
}

// ---- Select -----------------------------------------------------------------

/// Select all rows from a table, with optional WHERE filter and limit.
pub async fn select(
    state: &AppState,
    table: &str,
    select_cols: &str,
    filters: &[(&str, &str)],
    limit: Option<i64>,
) -> Result<(Vec<Value>, i64), String> {
    if let Some(ref pool) = state.pool {
        let query = format!("SELECT {select_cols} FROM {table}");
        let rows: Vec<PgRow> =
            sqlx::query(&query).fetch_all(pool).await.map_err(|e| format!("PG: {e}"))?;
        let total = rows.len() as i64;
        let data = rows_to_json(rows);
        Ok((data, total))
    } else {
        // REST fallback
        let rows = state.supabase.select(table, select_cols, filters, limit).await?;
        let total = rows.len() as i64;
        Ok((rows, total))
    }
}

/// Select a single row by id.
pub async fn select_one(
    state: &AppState,
    table: &str,
    select_cols: &str,
    id_col: &str,
    id_val: &str,
) -> Result<Option<Value>, String> {
    if let Some(ref pool) = state.pool {
        let query = format!("SELECT {select_cols} FROM {table} WHERE {id_col} = $1 LIMIT 1");
        let row: Option<PgRow> = sqlx::query(&query).bind(id_val).fetch_optional(pool).await.map_err(|e| format!("PG: {e}"))?;
        Ok(row.map(row_to_json))
    } else {
        state.supabase.select_one(table, select_cols, id_col, id_val).await
    }
}

// ---- Insert -----------------------------------------------------------------

pub async fn insert(
    state: &AppState,
    table: &str,
    body: &Value,
) -> Result<Value, String> {
    if let Some(ref pool) = state.pool {
        let cols: Vec<String> = body.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
        let vals: Vec<String> = cols.iter().map(|c| format!("'{}'", body.get(c).map(|v| v.to_string()).unwrap_or_default())).collect();
        let query = format!("INSERT INTO {table} ({}) VALUES ({}) RETURNING *", cols.join(","), vals.join(","));
        let row: PgRow = sqlx::query(&query).fetch_one(pool).await.map_err(|e| format!("PG: {e}"))?;
        Ok(row_to_json(row))
    } else {
        state.supabase.insert(table, body).await
    }
}

// ---- Update -----------------------------------------------------------------

pub async fn update(
    state: &AppState,
    table: &str,
    body: &Value,
    filters: &[(&str, &str)],
) -> Result<Vec<Value>, String> {
    if let Some(ref pool) = state.pool {
        let sets: Vec<String> = body.as_object().map(|m| m.iter().map(|(k,v)| format!("{k} = '{v}'")).collect()).unwrap_or_default();
        let mut where_clause = String::new();
        for (i, (col, val)) in filters.iter().enumerate() {
            if i == 0 { where_clause.push_str(&format!("{col} = '{val}'")); }
            else { where_clause.push_str(&format!(" AND {col} = '{val}'")); }
        }
        let query = format!("UPDATE {table} SET {} WHERE {where_clause} RETURNING *", sets.join(", "));
        let rows: Vec<PgRow> = sqlx::query(&query).fetch_all(pool).await.map_err(|e| format!("PG: {e}"))?;
        Ok(rows_to_json(rows))
    } else {
        state.supabase.update(table, body, filters).await
    }
}

// ---- Delete -----------------------------------------------------------------

pub async fn delete(
    state: &AppState,
    table: &str,
    filters: &[(&str, &str)],
) -> Result<(), String> {
    if let Some(ref pool) = state.pool {
        let mut where_clause = String::new();
        for (i, (col, val)) in filters.iter().enumerate() {
            if i == 0 { where_clause.push_str(&format!("{col} = '{val}'")); }
            else { where_clause.push_str(&format!(" AND {col} = '{val}'")); }
        }
        let query = format!("DELETE FROM {table} WHERE {where_clause}");
        sqlx::query(&query).execute(pool).await.map_err(|e| format!("PG: {e}"))?;
        Ok(())
    } else {
        state.supabase.delete(table, filters).await
    }
}
