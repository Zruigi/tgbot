pub mod start;
pub mod customer_service;
pub mod merchant_onboarding;
pub mod merchant_runtime;
pub mod group_monitor;
pub mod join_verify;
pub mod channel_verify;
pub mod wallet_monitor;
pub mod crypto_price;
pub mod admin_panel;
pub mod alert_setting;
pub mod wallet_panel;

// ---------------------------------------------------------------------------
// Type Aliases
// ---------------------------------------------------------------------------

/// Result type for handler functions using teloxide errors
pub type HandlerResult<T> = Result<T, teloxide::RequestError>;

/// Result type for handler functions using anyhow errors (for complex operations)
pub type AnyhowHandlerResult<T> = Result<T, anyhow::Error>;