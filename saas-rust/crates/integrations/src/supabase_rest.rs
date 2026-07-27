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
    pub base_url: String, // e.g. https://xttdylrgdldzmvnbeqoc.supabase.co
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
        h.insert(
            "apikey",
            HeaderValue::from_str(&self.service_role_key).unwrap(),
        );
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
        let body = resp.text().await.map_err(|e| format!("read body: {e}"))?;

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
            let status = resp.status();
            if !status.is_success() {
                let body = resp
                    .text()
                    .await
                    .map_err(|e| format!("read count error body: {e}"))?;
                return Err(format!("REST {status}: {body}"));
            }
            // PostgREST only exposes an exact total through Content-Range. Do
            // not turn a missing/malformed header into a successful zero: that
            // would make unavailable metadata look like a real count.
            let range = resp
                .headers()
                .get("content-range")
                .ok_or_else(|| "REST count response missing Content-Range".to_string())?;
            let range = range
                .to_str()
                .map_err(|e| format!("invalid Content-Range: {e}"))?;
            let count_str = range
                .rsplit('/')
                .next()
                .filter(|value| !value.is_empty() && *value != "*")
                .ok_or_else(|| format!("invalid Content-Range value: {range}"))?;
            return count_str
                .parse::<i64>()
                .map_err(|e| format!("parse count: {e}"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread::{self, JoinHandle};

    fn mock_server(body: &str) -> (String, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response_body = body.to_string();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];

            loop {
                let bytes_read = stream.read(&mut buffer).unwrap();
                if bytes_read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..bytes_read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }

            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            )
            .unwrap();
            String::from_utf8(request).unwrap()
        });

        (format!("http://{address}"), handle)
    }

    #[tokio::test]
    async fn test_select_plans() {
        let (base_url, request_handle) =
            mock_server(r#"[{"plan_code":"beta","plan_name":"Beta"}]"#);
        let client = SupabaseRestClient::new(&base_url, "test-service-key");
        let rows = client
            .select("subscription_plans", "plan_code,plan_name", &[], None)
            .await
            .unwrap();
        let request = request_handle.join().unwrap();

        assert_eq!(rows[0]["plan_code"], "beta");
        assert!(request
            .starts_with("GET /rest/v1/subscription_plans?select=plan_code,plan_name HTTP/1.1"));
        assert!(request.to_lowercase().contains("apikey: test-service-key"));
    }

    #[tokio::test]
    async fn test_select_merchants() {
        let (base_url, request_handle) =
            mock_server(r#"[{"id":7,"tenant_id":"tenant-beta","status":"active"}]"#);
        let client = SupabaseRestClient::new(&base_url, "test-service-key");
        let rows = client
            .select(
                "merchants",
                "id,tenant_id,status",
                &[("tenant_id", "tenant-beta")],
                Some(1),
            )
            .await
            .unwrap();
        let request = request_handle.join().unwrap();

        assert_eq!(rows[0]["id"], 7);
        assert!(request.starts_with(
            "GET /rest/v1/merchants?select=id,tenant_id,status&tenant_id=eq.tenant-beta&limit=1 HTTP/1.1"
        ));
    }

    #[tokio::test]
    async fn test_count_fails_closed_without_content_range() {
        let (base_url, request_handle) = mock_server("[]");
        let client = SupabaseRestClient::new(&base_url, "test-service-key");

        let error = client.count("*", &[]).await.unwrap_err();
        assert!(error.contains("missing Content-Range"));
        let request = request_handle.join().unwrap();
        assert!(request.starts_with("GET /rest/v1/merchants?select=count HTTP/1.1"));
    }
}
