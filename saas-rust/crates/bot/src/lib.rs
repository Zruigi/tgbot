#![cfg_attr(
    not(feature = "legacy-bot"),
    doc = "Historical standalone bot package; enable the `legacy-bot` feature for an explicit audit."
)]

#[cfg(feature = "legacy-bot")]
pub mod dispatcher;
#[cfg(feature = "legacy-bot")]
pub mod handlers;
#[cfg(feature = "legacy-bot")]
pub mod runtime_registry;
#[cfg(feature = "legacy-bot")]
pub mod telegram_client;
