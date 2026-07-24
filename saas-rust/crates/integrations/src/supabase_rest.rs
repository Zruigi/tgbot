//! Supabase REST API client — pure HTTPS, no PostgreSQL TCP required.
//!
//! Uses Supabase PostgREST (https://<project>.supabase.co/rest/v1/) to
//! perform CRUD operations.  Auth is handled with the service_role key
//! (server-side, never exposed to clients).

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde_json::Value;

const DEFAULT_TIMEOUT_SECS: u64 = 15;

/// Thin wrapper around Supabase's REST/PostgREST API.
#[derive(Clone, Debug)]
pub struct SupabaseRestClient {
    pub base_url: String,   // e.g. https://xttdylrgdldzmvnbeqoc.supabase.co
    pub service_role_key: String,
    client: reqwest::Client,
}

impl SupabaseRestClient {
    pub fn new(supabase_url: &str, service_role_key: &str) -> Self {
        Self {
            base_url: supabase_url.trim_end_matches('/').to_string(),
            service_role_key: service_role_key.to_string(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
                .build()
                .expect("reqwest Client"),
        }
    }

    // --- helpers ----------------------------------------------------------

    fn rest_url(&self, table: &str) -> String {
        format!("{}/rest/v1/{}", self.base_url, table)
    }

    fn auth_headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("apikey", HeaderValue::from_str(&self.service_role_key).unwrap());
        h.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.service_role_key)).unwrap(),
        );
        h.insert("Content-Type", HeaderValue::from_static("application/json"));
        h.insert("Prefer", HeaderValue::from_static("return=representation"));
        h
    }

    // --- SELECT -----------------------------------------------------------

    /// `GET /rest/v1/{table}?select=...&column=eq.value`
    pub async fn select(
        &self,
        table: &str,
        select: &str,
        filters: &[(&str, &str)], // (column, value) — equality only for now
        limit: Option<i64>,
    ) -> Result<Vec<Value>, String> {
        let mut url = self.rest_url(table);
        let mut params: Vec<String> = vec![format!("select={}", select)];

        for (col, val) in filters {
            params.push(format!("{}=eq.{}", col, val));
        }
        if let Some(lim) = limit {
            params.push(format!("limit={}", lim));
        }

        url.push('?');
        url.push_str(&params.join("&"));

        let resp = self
            .client
            .get(&url)
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| format!("REST request failed: {e}"))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("read body: {e}"))?;

        if !status.is_success() {
            return Err(format!("REST {status}: {body}"));
        }

        serde_json::from_str::<Vec<Value>>(&body).map_err(|e| format!("json parse: {e}"))
    }

    /// Select a single row by id column (returns None if not found).
    pub async fn select_one(
        &self,
        table: &str,
        select: &str,
        id_column: &str,
        id_value: &str,
    ) -> Result<Option<Value>, String> {
        let rows = self
            .select(table, select, &[(id_column, id_value)], Some(1))
            .await?;
        Ok(rows.into_iter().next())
    }

    // --- INSERT -----------------------------------------------------------

    /// `POST /rest/v1/{table}` — returns the inserted row(s).
    pub async fn insert(&self, table: &str, body: &Value) -> Result<Value, String> {
        let url = self.rest_url(table);
        let resp = self
            .client
            .post(&url)
            .headers(self.auth_headers())
            .json(body)
            .send()
            .await
            .map_err(|e| format!("REST POST failed: {e}"))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;
        if !status.is_success() {
            return Err(format!("REST {status}: {text}"));
        }
        serde_json::from_str(&text).map_err(|e| format!("json parse: {e}"))
    }

    // --- UPDATE -----------------------------------------------------------

    /// `PATCH /rest/v1/{table}?column=eq.value`
    pub async fn update(
        &self,
        table: &str,
        body: &Value,
        filters: &[(&str, &str)],
    ) -> Result<Vec<Value>, String> {
        let mut url = self.rest_url(table);
        let params: Vec<String> = filters
            .iter()
            .map(|(col, val)| format!("{}=eq.{}", col, val))
            .collect();
        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }

        let resp = self
            .client
            .patch(&url)
            .headers(self.auth_headers())
            .json(body)
            .send()
            .await
            .map_err(|e| format!("REST PATCH failed: {e}"))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;
        if !status.is_success() {
            return Err(format!("REST {status}: {text}"));
        }
        serde_json::from_str(&text).map_err(|e| format!("json parse: {e}"))
    }

    // --- DELETE -----------------------------------------------------------

    /// `DELETE /rest/v1/{table}?column=eq.value`
    pub async fn delete(&self, table: &str, filters: &[(&str, &str)]) -> Result<(), String> {
        let mut url = self.rest_url(table);
        let params: Vec<String> = filters
            .iter()
            .map(|(col, val)| format!("{}=eq.{}", col, val))
            .collect();
        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }

        let resp = self
            .client
            .delete(&url)
            .headers(self.auth_headers())
            .send()
            .await
            .map_err(|e| format!("REST DELETE failed: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;
            return Err(format!("REST {status}: {text}"));
        }
        Ok(())
    }

    // --- RPC (stored functions) -------------------------------------------

    /// `POST /rest/v1/rpc/{function_name}`
    pub async fn rpc(&self, function_name: &str, body: &Value) -> Result<Value, String> {
        let url = format!("{}/rest/v1/rpc/{}", self.base_url, function_name);
        let resp = self
            .client
            .post(&url)
            .headers(self.auth_headers())
            .json(body)
            .send()
            .await
            .map_err(|e| format!("RPC failed: {e}"))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;
        if !status.is_success() {
            return Err(format!("RPC {status}: {text}"));
        }
        serde_json::from_str(&text).map_err(|e| format!("json parse: {e}"))
    }

    // --- Convenience: raw count -------------------------------------------

    pub async fn count(&self, table: &str, filters: &[(&str, &str)]) -> Result<i64, String> {
        let _sel = "count".to_string();
        if table == "*" {
            // head request for total count
            let url = format!("{}/rest/v1/merchants?select=count", self.base_url);
            let resp = self
                .client
                .get(&url)
                .headers(self.auth_headers())
                .header("Prefer", "count=exact")
                .send()
                .await
                .map_err(|e| format!("count failed: {e}"))?;
            // Content-Range header has the count
            if let Some(range) = resp.headers().get("content-range") {
                if let Ok(s) = range.to_str() {
                    if let Some(count_str) = s.split('/').last() {
                        return count_str.parse::<i64>().map_err(|e| format!("parse count: {e}"));
                    }
                }
            }
            return Ok(0);
        }

        let rows = self.select(table, "count", filters, Some(1)).await?;
        if rows.is_empty() {
            return Ok(0);
        }
        rows[0]
            .get("count")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| "count field missing".to_string())
    }
}

// ---------------------------------------------------------------------------
// Quick integration test (not compiled by default, use `cargo test`)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// These tests require SUPABASE_URL + SUPABASE_SERVICE_ROLE_KEY env vars.
    fn test_client() -> SupabaseRestClient {
        let url = std::env::var("SUPABASE_URL").unwrap_or_else(|_| {
            "https://xttdylrgdldzmvnbeqoc.supabase.co".into()
        });
        let key = std::env::var("SUPABASE_SERVICE_ROLE_KEY").unwrap_or_default();
        SupabaseRestClient::new(&url, &key)
    }

    #[tokio::test]
    async fn test_select_plans() {
        let c = test_client();
        let rows = c
            .select("subscription_plans", "plan_code,plan_name", &[], None)
            .await;
        assert!(rows.is_ok(), "{rows:?}");
        let rows = rows.unwrap();
        assert!(!rows.is_empty(), "Expected at least one plan");
        println!("plans: {rows:#?}");
    }

    #[tokio::test]
    async fn test_select_merchants() {
        let c = test_client();
        let rows = c
            .select("merchants", "id,tenant_id,merchant_name,status", &[], None)
            .await;
        assert!(rows.is_ok(), "{rows:?}");
        println!("merchants: {:#?}", rows.unwrap());
    }
}
