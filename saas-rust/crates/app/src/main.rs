use std::sync::Arc;
use axum::Router;
use tgbot_domain::config::Config;
use tgbot_http::middleware;
use tgbot_http::routes;
use tgbot_http::routes::auth::{OtpStore, CsrfStore};
use tgbot_http::routes::miniapp::GrantStore;
use tgbot_http::routes::telegram_webhook::{BotRegistry, UpdateDispatcher, install_default_handlers, load_bots_from_db};
use tgbot_http::state::AppState;
use tgbot_integrations::supabase_rest::SupabaseRestClient;
use tower_http::{
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::from_env()?;

    // --- Supabase REST client (HTTPS, always works) ---
    let supabase = SupabaseRestClient::new(&config.supabase_url, &config.supabase_service_role_key);

    // --- Try PostgreSQL pool; degrade to REST-only if blocked ---
    let pg_result = sqlx::postgres::PgPoolOptions::new()
        .max_connections(config.database_max_connections)
        .connect(&config.database_url)
        .await;

    let pool = match pg_result {
        Ok(p) => {
            tracing::info!("PostgreSQL direct connection established");
            Some(p)
        }
        Err(e) => {
            tracing::warn!("PostgreSQL unavailable ({e}), running in REST-only mode");
            None
        }
    };

    // Initialize webhook bot registry
    let bot_registry = BotRegistry::new();

    // Load active bots from database (PG-only)
    if let Some(ref pg) = pool {
        if let Err(e) = load_bots_from_db(pg, &bot_registry).await {
            tracing::warn!(error = %e, "Failed to load bots from DB at startup");
        }
    }

    // Initialize update dispatcher with default handlers
    let dispatcher = Arc::new(UpdateDispatcher::new());
    install_default_handlers(&dispatcher).await;

    // Spawn background rate limiter cleanup
    tokio::spawn(middleware::rate_limiter_cleanup_task());

    let state = AppState {
        config: config.clone(),
        pool,
        supabase,
        otp_store: OtpStore::new(),
        csrf_store: CsrfStore::new(),
        grant_store: GrantStore::new(),
        bot_registry,
        update_dispatcher: Some(dispatcher),
    };

    // CORS
    let cors = if config.cors_allowed_origin == "*" {
        CorsLayer::permissive()
    } else {
        CorsLayer::new()
            .allow_origin(config.cors_allowed_origin.parse::<axum::http::HeaderValue>().unwrap())
            .allow_methods(Any)
            .allow_headers(Any)
    };

    // Middleware chain: outermost runs first
    let app = Router::new()
        .merge(routes::router())
        .layer(axum::middleware::from_fn(middleware::rate_limit_middleware))
        .layer(axum::middleware::from_fn(middleware::security_headers_middleware))
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    tracing::info!("tgbot Rust backend listening on {}", config.bind_addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
