//! Feature gate helpers — runtime configuration checks.
//!
//! These functions are used by the HTTP layer and workers to determine
//! whether a particular feature is enabled for the current request or
//! job context.  The gates are session-level PostgreSQL settings set
//! at connection pool initialisation time from the environment:
//!
//! - `tgbot.telegram_commands_enabled`
//! - `tgbot.telegram_auto_replies_enabled`
//!
//! # Default
//!
//! All features default to *disabled* unless explicitly enabled via
//! environment variable (see `tgbot_domain::config`).

use sqlx::PgPool;

/// Check whether Telegram commands are enabled for this connection.
///
/// Returns `true` when the session-level `tgbot.telegram_commands_enabled`
/// setting is `'true'`.
pub async fn telegram_commands_enabled(pool: &PgPool) -> bool {
    check_session_setting(pool, "tgbot.telegram_commands_enabled").await
}

/// Check whether Telegram auto-replies are enabled for this connection.
///
/// Returns `true` when the session-level `tgbot.telegram_auto_replies_enabled`
/// setting is `'true'`.
pub async fn telegram_auto_replies_enabled(pool: &PgPool) -> bool {
    check_session_setting(pool, "tgbot.telegram_auto_replies_enabled").await
}

/// Read a session-level `set_config` text setting.
async fn check_session_setting(pool: &PgPool, setting: &str) -> bool {
    sqlx::query_scalar::<_, String>("SELECT current_setting($1, true)")
        .bind(setting)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "true" | "on" | "1"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unit-level tests for the session-setting parser.
    // The actual PG session variable is tested in integration tests.

    #[test]
    fn test_parse_telegram_commands_enabled() {
        // Re-export the config-level parser used by the HTTP layer.
        assert!(tgbot_domain::config::parse_telegram_commands_enabled(Some(
            "true"
        )));
        assert!(!tgbot_domain::config::parse_telegram_commands_enabled(None));
        assert!(!tgbot_domain::config::parse_telegram_commands_enabled(
            Some("false")
        ));
    }
}
