//! Security middleware — log redaction, security headers, and rate limiting
//!
//! Provides:
//! - Redaction of sensitive headers before they reach logs
//! - Security headers (X-Content-Type-Options, X-Frame-Options, etc.)
//! - Per-IP rate limiting via global static

use axum::{
    body::Body,
    extract::connect_info::ConnectInfo,
    http::{HeaderMap, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::LazyLock,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;

/// Sensitive header names to redact in logs
const SENSITIVE_HEADERS: &[&str] = &[
    "x-saas-internal-secret",
    "authorization",
    "cookie",
    "x-merchant-id",
    "x-legacy-merchant-id",
    "x-user-id",
];

/// Global rate limiter state (per-IP, default: 100 req / 60s)
static RATE_LIMITER: LazyLock<RwLock<HashMap<String, Vec<Instant>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Periodic cleanup task for expired rate limiter entries (runs every 5 min)
pub async fn rate_limiter_cleanup_task() {
    let mut interval = tokio::time::interval(Duration::from_secs(300));
    loop {
        interval.tick().await;
        let mut buckets = RATE_LIMITER.write().await;
        let now = Instant::now();
        let window = Duration::from_secs(60);
        buckets.retain(|_, timestamps| {
            timestamps.retain(|&t| now.duration_since(t) < window);
            !timestamps.is_empty()
        });
    }
}

/// Redact sensitive values from request headers for logging
pub fn redact_sensitive_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            let name_lower = name.as_str().to_lowercase();
            if SENSITIVE_HEADERS.contains(&name_lower.as_str()) {
                (name.as_str().to_string(), "[REDACTED]".to_string())
            } else {
                (name.as_str().to_string(), value.to_str().unwrap_or("[INVALID]").to_string())
            }
        })
        .collect()
}

/// Security headers middleware — adds hardening headers to all responses
pub async fn security_headers_middleware(
    request: Request<Body>,
    next: Next,
) -> impl IntoResponse {
    // Log redacted headers at debug level
    let redacted = redact_sensitive_headers(request.headers());
    tracing::debug!(?redacted, "request headers (sensitive values redacted)");

    let mut response = next.run(request).await;

    let headers = response.headers_mut();
    headers.insert("X-Content-Type-Options", "nosniff".parse().unwrap());
    headers.insert("X-Frame-Options", "DENY".parse().unwrap());
    headers.insert("X-XSS-Protection", "1; mode=block".parse().unwrap());
    headers.insert("Referrer-Policy", "strict-origin-when-cross-origin".parse().unwrap());

    response
}

/// Per-IP rate limiting middleware — returns 429 if over limit
pub async fn rate_limit_middleware(
    request: Request<Body>,
    next: Next,
) -> Response {
    // Extract IP from ConnectInfo or fallback
    let ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Skip rate limiting for health checks
    let path = request.uri().path();
    if path == "/healthz" || path == "/api/health" || path == "/ws" {
        return next.run(request).await;
    }

    // Check rate limit via global static
    let allowed = {
        let mut buckets = RATE_LIMITER.write().await;
        let now = Instant::now();
        let window = Duration::from_secs(60);
        let timestamps = buckets.entry(ip).or_default();
        timestamps.retain(|&t| now.duration_since(t) < window);
        if timestamps.len() >= 100 {
            false
        } else {
            timestamps.push(now);
            true
        }
    };

    if !allowed {
        let response = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "status": "error",
                "message": "rate_limit_exceeded",
                "retry_after_secs": 60,
            })),
        );
        return response.into_response();
    }

    next.run(request).await
}
