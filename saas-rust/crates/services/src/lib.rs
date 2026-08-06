pub mod ad_detection;
pub mod ad_governance;
pub mod ad_pipeline;
pub mod ad_text;
pub mod feature_gate;
pub mod group_policy;
pub mod telegram_auto_replies;
pub mod telegram_commands;
pub mod telegram_preferences;
pub mod telegram_wallet;

// Empty/legacy modules are not part of the formal app release graph. Keep
// them behind an opt-in feature so an accidental import cannot expose a
// success-shaped placeholder implementation.
#[cfg(feature = "legacy-stubs")]
pub mod audit_service;
#[cfg(feature = "legacy-stubs")]
pub mod auth_service;
#[cfg(feature = "legacy-stubs")]
pub mod broadcast_service;
#[cfg(feature = "legacy-stubs")]
pub mod chat_service;
#[cfg(feature = "legacy-stubs")]
pub mod finance_service;
#[cfg(feature = "legacy-stubs")]
pub mod group_service;
#[cfg(feature = "legacy-stubs")]
pub mod miniapp_service;
#[cfg(feature = "legacy-stubs")]
pub mod monitor_service;
#[cfg(feature = "legacy-stubs")]
pub mod pusher;
#[cfg(feature = "legacy-stubs")]
pub mod quota_service;
#[cfg(feature = "legacy-stubs")]
pub mod subscription_lifecycle;

// Re-export commonly used functions
#[cfg(feature = "legacy-stubs")]
pub use broadcast_service::{execute_push_task, send_broadcast_to_chats, send_broadcast_to_users};
