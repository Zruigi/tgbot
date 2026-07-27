pub mod admin_panel;
pub mod alert_setting;
pub mod channel_verify;
pub mod crypto_price;
pub mod customer_service;
pub mod group_monitor;
pub mod join_verify;
pub mod merchant_onboarding;
pub mod merchant_runtime;
pub mod start;
pub mod wallet_monitor;
pub mod wallet_panel;

// ---------------------------------------------------------------------------
// Type Aliases
// ---------------------------------------------------------------------------

/// Result type for handler functions using teloxide errors
pub type HandlerResult<T> = Result<T, teloxide::RequestError>;

/// Result type for handler functions using anyhow errors (for complex operations)
pub type AnyhowHandlerResult<T> = Result<T, anyhow::Error>;
