use anyhow::{anyhow, Result};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: String,
    pub database_url: String,
    pub database_max_connections: u32,
    pub supabase_url: String,
    pub supabase_service_role_key: String,
    pub supabase_jwt_secret: String,
    pub internal_saas_secret: String,
    pub web_admin_token: String,
    pub bot_token: String,
    pub cors_allowed_origin: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let database_url = std::env::var("SUPABASE_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .map_err(|_| anyhow!("SUPABASE_DATABASE_URL or DATABASE_URL is required"))?;
        let supabase_jwt_secret = std::env::var("SUPABASE_JWT_SECRET").unwrap_or_default();
        let internal_saas_secret = std::env::var("INTERNAL_SAAS_SECRET").unwrap_or_default();
        if supabase_jwt_secret.is_empty() && internal_saas_secret.is_empty() {
            return Err(anyhow!(
                "SUPABASE_JWT_SECRET or INTERNAL_SAAS_SECRET is required"
            ));
        }

        Ok(Self {
            bind_addr: std::env::var("SAAS_BIND_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:8090".to_string()),
            database_url,
            database_max_connections: std::env::var("SAAS_DB_MAX_CONNECTIONS")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(8),
            supabase_url: std::env::var("SUPABASE_URL").unwrap_or_default(),
            supabase_service_role_key: std::env::var("SUPABASE_SERVICE_ROLE_KEY")
                .or_else(|_| std::env::var("SUPABASE_SECRET_KEY"))
                .unwrap_or_default(),
            supabase_jwt_secret,
            internal_saas_secret,
            web_admin_token: std::env::var("WEB_ADMIN_TOKEN").unwrap_or_default(),
            bot_token: std::env::var("BOT_TOKEN").unwrap_or_default(),
            cors_allowed_origin: std::env::var("CORS_ALLOWED_ORIGIN")
                .unwrap_or_else(|_| "*".to_string()),
        })
    }
}
