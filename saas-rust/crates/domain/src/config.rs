use anyhow::{anyhow, Result};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: String,
    pub database_url: String,
    pub database_max_connections: u32,
    pub allow_rest_only: bool,
    pub supabase_url: String,
    pub supabase_service_role_key: String,
    pub supabase_jwt_secret: String,
    pub internal_saas_secret: String,
    pub session_secret: String,
    pub secret_encryption_key: String,
    pub web_admin_token: String,
    pub bot_token: String,
    pub telegram_auto_replies_enabled: bool,
    pub telegram_commands_enabled: bool,
    pub telegram_wallet_monitors_enabled: bool,
    pub cors_allowed_origin: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let database_url = std::env::var("SUPABASE_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .map_err(|_| anyhow!("SUPABASE_DATABASE_URL or DATABASE_URL is required"))?;
        let supabase_jwt_secret = std::env::var("SUPABASE_JWT_SECRET").unwrap_or_default();
        let internal_saas_secret = std::env::var("INTERNAL_SAAS_SECRET").unwrap_or_default();
        let session_secret = std::env::var("SAAS_SESSION_SECRET").unwrap_or_default();
        let secret_encryption_key =
            std::env::var("TGBOT_SECRET_ENCRYPTION_KEY").unwrap_or_default();
        if supabase_jwt_secret.is_empty() && internal_saas_secret.is_empty() {
            return Err(anyhow!(
                "SUPABASE_JWT_SECRET or INTERNAL_SAAS_SECRET is required"
            ));
        }
        if session_secret.as_bytes().len() < 32 {
            return Err(anyhow!("SAAS_SESSION_SECRET must be at least 32 bytes"));
        }
        if secret_encryption_key.trim().is_empty() {
            return Err(anyhow!("TGBOT_SECRET_ENCRYPTION_KEY is required"));
        }

        Ok(Self {
            bind_addr: std::env::var("SAAS_BIND_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:18090".to_string()),
            database_url,
            database_max_connections: std::env::var("SAAS_DB_MAX_CONNECTIONS")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(8),
            allow_rest_only: parse_rest_only_enabled(
                std::env::var("SAAS_ALLOW_REST_ONLY").ok().as_deref(),
            ),
            supabase_url: std::env::var("SUPABASE_URL").unwrap_or_default(),
            supabase_service_role_key: std::env::var("SUPABASE_SERVICE_ROLE_KEY")
                .or_else(|_| std::env::var("SUPABASE_SECRET_KEY"))
                .unwrap_or_default(),
            supabase_jwt_secret,
            internal_saas_secret,
            session_secret,
            secret_encryption_key,
            web_admin_token: std::env::var("WEB_ADMIN_TOKEN").unwrap_or_default(),
            bot_token: std::env::var("BOT_TOKEN").unwrap_or_default(),
            telegram_auto_replies_enabled: parse_telegram_auto_replies_enabled(
                std::env::var("TELEGRAM_AUTO_REPLIES_ENABLED")
                    .ok()
                    .as_deref(),
            ),
            telegram_commands_enabled: parse_telegram_commands_enabled(
                std::env::var("TELEGRAM_COMMANDS_ENABLED").ok().as_deref(),
            ),
            telegram_wallet_monitors_enabled: parse_telegram_commands_enabled(
                std::env::var("TELEGRAM_WALLET_MONITORS_ENABLED")
                    .ok()
                    .as_deref(),
            ),
            cors_allowed_origin: std::env::var("CORS_ALLOWED_ORIGIN")
                .unwrap_or_else(|_| "*".to_string()),
        })
    }
}

pub fn parse_telegram_auto_replies_enabled(value: Option<&str>) -> bool {
    parse_telegram_commands_enabled(value)
}

pub fn parse_rest_only_enabled(value: Option<&str>) -> bool {
    parse_telegram_commands_enabled(value)
}

pub fn parse_telegram_commands_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_enabled_not_set() {
        assert!(!parse_telegram_commands_enabled(None));
    }

    #[test]
    fn test_parse_enabled_empty() {
        assert!(!parse_telegram_commands_enabled(Some("")));
    }

    #[test]
    fn test_parse_enabled_true_variants() {
        for value in [
            "1", "true", "True", "TRUE", "tRuE", "yes", "Yes", "YES", "on", "On", "ON",
        ] {
            assert!(
                parse_telegram_commands_enabled(Some(value)),
                "value={value:?}"
            );
        }
        // trimmed
        assert!(parse_telegram_commands_enabled(Some("  true  ")));
        assert!(parse_telegram_commands_enabled(Some("\t  yes  \n")));
    }

    #[test]
    fn test_parse_enabled_false_or_invalid() {
        for value in [
            "false",
            "False",
            "FALSE",
            "0",
            "no",
            "off",
            "banana",
            "2",
            "ye s",
            "truefalse",
        ] {
            assert!(
                !parse_telegram_commands_enabled(Some(value)),
                "value={value:?}"
            );
        }
    }

    #[test]
    fn test_auto_replies_default_off_and_explicit_on() {
        assert!(!parse_telegram_auto_replies_enabled(None));
        assert!(!parse_telegram_auto_replies_enabled(Some("false")));
        assert!(parse_telegram_auto_replies_enabled(Some(" true ")));
    }

    #[test]
    fn test_rest_only_is_explicitly_opt_in() {
        assert!(!parse_rest_only_enabled(None));
        assert!(!parse_rest_only_enabled(Some("false")));
        assert!(parse_rest_only_enabled(Some("true")));
    }
}
