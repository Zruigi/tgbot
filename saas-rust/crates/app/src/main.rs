use std::sync::Arc;
use axum::Router;
use tgbot_domain::config::Config;
use tgbot_http::routes;
use tgbot_http::state::AppState;
use tgbot_http::routes::auth::{OtpStore, CsrfStore};
use tgbot_http::routes::miniapp::GrantStore;
use tgbot_http::routes::telegram_webhook::{BotRegistry, UpdateDispatcher, install_default_handlers, load_bots_from_db};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
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
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(config.database_max_connections)
        .connect(&config.database_url)
        .await?;

    // Initialize webhook bot registry
    let bot_registry = BotRegistry::new();

    // Load active bots from database into registry
    if let Err(e) = load_bots_from_db(&pool, &bot_registry).await {
        tracing::warn!(error = %e, "Failed to load bots from DB at startup (continuing anyway)");
    }

    // Initialize update dispatcher with default handlers
    let dispatcher = Arc::new(UpdateDispatcher::new());
    install_default_handlers(&dispatcher).await;

    let state = AppState {
        config: config.clone(),
        pool,
        otp_store: OtpStore::new(),
        csrf_store: CsrfStore::new(),
        grant_store: GrantStore::new(),
        bot_registry,
        update_dispatcher: Some(dispatcher),
    };

    let app = Router::new()
        .merge(routes::router())
        .layer(CorsLayer::permissive())
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
