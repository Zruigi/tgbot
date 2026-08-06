use anyhow::anyhow;
use async_trait::async_trait;
use axum::response::IntoResponse;
use axum::Router;
use std::{sync::Arc, time::Duration};
use tgbot_domain::config::Config;
use tgbot_domain::worker_telemetry::WorkerTelemetry;
use tgbot_http::middleware;
use tgbot_http::routes;
use tgbot_http::routes::auth::{CsrfStore, OtpStore};
use tgbot_http::routes::miniapp::GrantStore;
use tgbot_http::routes::telegram_webhook::{
    install_default_handlers, load_bots_from_db, BotRegistry, UpdateDispatcher,
};
use tgbot_http::state::AppState;
use tgbot_integrations::bscscan::BscScanClient;
use tgbot_integrations::litellm::LiteLLMClient;
use tgbot_integrations::supabase_rest::SupabaseRestClient;
use tgbot_integrations::telegram::{TelegramClient, TelegramConfig};
use tgbot_integrations::trongrid::TronGridClient;
use tgbot_repos::user_repo;
use tgbot_workers::{
    AdGovernanceWorker, AddressMonitorAlertWorker, AddressMonitorWorker,
    Dispatcher as WorkerDispatcher, GroupMessagePolicyWorker, PushTaskWorker, Scheduler,
    SchedulerConfig, TelegramAutoReplyWorker, TelegramClientResolver, TelegramCommandWorker,
    Worker, WorkerHandle,
};
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
    tgbot_http::secret_crypto::SecretCipher::from_encoded_key(&config.secret_encryption_key)
        .map_err(|error| anyhow!("invalid TGBOT_SECRET_ENCRYPTION_KEY: {error}"))?;

    // --- Supabase REST client (HTTPS, always works) ---
    let supabase = SupabaseRestClient::new(&config.supabase_url, &config.supabase_service_role_key);
    let llm = LiteLLMClient::from_env()?;

    // --- Connect PostgreSQL; REST-only is an explicit local diagnostic mode ---
    let telegram_auto_replies_enabled = config.telegram_auto_replies_enabled;
    let telegram_commands_enabled = config.telegram_commands_enabled;
    let telegram_wallet_monitors_enabled = config.telegram_wallet_monitors_enabled;
    let pg_result = sqlx::postgres::PgPoolOptions::new()
        .max_connections(config.database_max_connections)
        .after_connect(move |connection, _metadata| {
            Box::pin(async move {
                sqlx::query(
                    "SET search_path TO public, chat, groups, broadcast, tenant_saas, platform, oneapi, chain_sync, auth",
                )
                .execute(&mut *connection)
                .await?;
                sqlx::query("SELECT set_config('tgbot.telegram_commands_enabled', $1, false)")
                    .bind(if telegram_commands_enabled {
                        "true"
                    } else {
                        "false"
                    })
                .execute(&mut *connection)
                .await?;
                sqlx::query("SELECT set_config('tgbot.telegram_auto_replies_enabled', $1, false)")
                    .bind(if telegram_auto_replies_enabled {
                        "true"
                    } else {
                        "false"
                    })
                .execute(&mut *connection)
                .await?;
                sqlx::query("SELECT set_config('tgbot.telegram_wallet_monitors_enabled', $1, false)")
                    .bind(if telegram_wallet_monitors_enabled {
                        "true"
                    } else {
                        "false"
                    })
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&config.database_url)
        .await;

    let pool = match pg_result {
        Ok(p) => {
            tracing::info!("PostgreSQL direct connection established");
            Some(p)
        }
        Err(error) if config.allow_rest_only => {
            tracing::warn!(
                error = %error,
                "PostgreSQL unavailable; running in explicitly enabled REST-only diagnostic mode"
            );
            None
        }
        Err(error) => {
            return Err(anyhow!(
                "PostgreSQL is required for the MVP runtime; connection failed: {error}"
            ));
        }
    };

    // Initialize webhook bot registry
    let bot_registry = BotRegistry::new();
    let worker_telemetry = WorkerTelemetry::default();

    // Load active bots from database (PG-only)
    if let Some(ref pg) = pool {
        if let Err(e) = load_bots_from_db(pg, &bot_registry, &config.secret_encryption_key).await {
            tracing::warn!(error = %e, "Failed to load bots from DB at startup");
        }
    }

    // Initialize update dispatcher with default handlers
    let dispatcher = Arc::new(UpdateDispatcher::new());
    install_default_handlers(
        &dispatcher,
        pool.clone(),
        std::env::var("TELEGRAM_API_BASE_URL")
            .unwrap_or_else(|_| "https://api.telegram.org".into()),
    )
    .await;

    // Spawn background rate limiter cleanup
    tokio::spawn(middleware::rate_limiter_cleanup_task());

    let wallet_monitor_clients = if config.telegram_wallet_monitors_enabled {
        Some((
            Arc::new(TronGridClient::from_env()?),
            Arc::new(BscScanClient::from_env()?),
        ))
    } else {
        None
    };

    let worker_handle = pool.as_ref().map(|pg| {
        let resolver = Arc::new(RegistryTelegramResolver {
            bot_registry: bot_registry.clone(),
        });
        let mut workers = vec![
            Arc::new(PushTaskWorker::new(resolver.clone(), Arc::new(llm.clone())))
                as Arc<dyn Worker>,
            Arc::new(GroupMessagePolicyWorker::new()) as Arc<dyn Worker>,
            Arc::new(AdGovernanceWorker::new(
                resolver.clone(),
                Arc::new(llm.clone()),
            )) as Arc<dyn Worker>,
        ];
        if let Some((trongrid, bscscan)) = wallet_monitor_clients.as_ref() {
            workers.push(
                Arc::new(AddressMonitorWorker::new(trongrid.clone(), bscscan.clone()))
                    as Arc<dyn Worker>,
            );
            workers.push(
                Arc::new(AddressMonitorAlertWorker::new(resolver.clone())) as Arc<dyn Worker>
            );
        }
        if config.telegram_commands_enabled {
            workers.push(Arc::new(TelegramCommandWorker::new(
                resolver.clone(),
                config.telegram_wallet_monitors_enabled,
            )) as Arc<dyn Worker>);
        }
        if config.telegram_auto_replies_enabled {
            workers
                .push(Arc::new(TelegramAutoReplyWorker::new(resolver.clone())) as Arc<dyn Worker>);
        }
        let mut scheduler_config = SchedulerConfig::default();
        scheduler_config.poll_interval = env_duration("WORKER_POLL_INTERVAL_SECS", 5);
        scheduler_config.cron_check_interval = env_duration("PUSH_TASK_CRON_INTERVAL_SECS", 60);
        scheduler_config.heartbeat_interval = env_duration("WORKER_HEARTBEAT_INTERVAL_SECS", 10);
        scheduler_config.worker_concurrency =
            env_usize("WORKER_CONCURRENCY", scheduler_config.worker_concurrency);
        let mut scheduler =
            Scheduler::new(pg.clone(), scheduler_config, WorkerDispatcher::new(workers))
                .with_telemetry(worker_telemetry.clone())
                .with_job_type_filter("push_task")
                .with_job_type_filter("group_message_policy")
                .with_job_type_filter("ad_scan_job")
                .with_push_task_cron();
        if config.telegram_commands_enabled {
            scheduler = scheduler.with_job_type_filter("telegram_command_reply");
        }
        if config.telegram_auto_replies_enabled {
            scheduler = scheduler.with_job_type_filter("telegram_auto_reply");
        }
        if config.telegram_wallet_monitors_enabled {
            scheduler = scheduler
                .with_job_type_filter("monitor_check")
                .with_job_type_filter("monitor_alert")
                .with_address_monitor_cron();
        }
        scheduler
            .with_channel_reminder_cleanup(
                resolver,
                env_duration("CHANNEL_REMINDER_CLEANUP_INTERVAL_SECS", 60),
            )
            .start()
    });

    let worker_enabled = worker_handle.is_some();
    let state = AppState {
        config: config.clone(),
        pool,
        supabase,
        llm,
        otp_store: OtpStore::new(),
        csrf_store: CsrfStore::new(),
        grant_store: GrantStore::new(),
        bot_registry,
        worker_enabled,
        worker_telemetry,
        update_dispatcher: Some(dispatcher),
    };

    // CORS
    let cors = if config.cors_allowed_origin == "*" {
        CorsLayer::permissive()
    } else {
        CorsLayer::new()
            .allow_origin(
                config
                    .cors_allowed_origin
                    .parse::<axum::http::HeaderValue>()
                    .map_err(|error| anyhow!("invalid CORS_ALLOWED_ORIGIN: {error}"))?,
            )
            .allow_methods(Any)
            .allow_headers(Any)
    };

    // Middleware chain: outermost runs first
    // Static frontend (vite dist) is served from FRONTEND_DIST_DIR when set;
    // otherwise the backend is API-only (deployed behind nginx).
    let mut app = Router::new()
        .merge(routes::router())
        .layer(axum::middleware::from_fn(middleware::rate_limit_middleware))
        .layer(axum::middleware::from_fn(
            middleware::security_headers_middleware,
        ))
        .layer(RequestBodyLimitLayer::new(11 * 1024 * 1024))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    if let Ok(dist_dir) = std::env::var("FRONTEND_DIST_DIR") {
        if !dist_dir.is_empty() {
            tracing::info!(%dist_dir, "Serving frontend static files");
            // Static assets are served directly; SPA routes fall back to
            // index.html with a 200 status (tower-http's not_found_service
            // would force a 404 status, which breaks SPA navigation).
            let assets_dir = format!("{dist_dir}/assets");
            app = app
                .nest_service(
                    "/assets",
                    tower_http::services::ServeDir::new(&assets_dir),
                )
                .fallback({
                    let index_path = format!("{dist_dir}/index.html");
                    axum::routing::get(move || {
                        let index_path = index_path.clone();
                        async move {
                            match tokio::fs::read(&index_path).await {
                                Ok(bytes) => (
                                    axum::http::StatusCode::OK,
                                    [(axum::http::header::CONTENT_TYPE, "text/html")],
                                    bytes,
                                )
                                    .into_response(),
                                Err(_) => axum::http::StatusCode::NOT_FOUND.into_response(),
                            }
                        }
                    })
                });
        }
    }

    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    tracing::info!("tgbot Rust backend listening on {}", config.bind_addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(worker_handle))
        .await?;
    Ok(())
}

#[derive(Clone)]
struct RegistryTelegramResolver {
    bot_registry: BotRegistry,
}

#[async_trait]
impl TelegramClientResolver for RegistryTelegramResolver {
    async fn resolve(
        &self,
        pool: &sqlx::PgPool,
        tenant_id: &str,
    ) -> anyhow::Result<TelegramClient> {
        let bot_id = user_repo::get_bot_id_for_tenant(pool, tenant_id)
            .await?
            .ok_or_else(|| anyhow!("tenant {tenant_id} has no bot configured"))?;
        let entry = self
            .bot_registry
            .get(bot_id)
            .await
            .filter(|entry| entry.tenant_id == tenant_id)
            .ok_or_else(|| anyhow!("tenant {tenant_id} bot is not active"))?;
        TelegramClient::new(TelegramConfig {
            bot_token: entry.bot_token,
            api_base_url: std::env::var("TELEGRAM_API_BASE_URL")
                .unwrap_or_else(|_| "https://api.telegram.org".into()),
            timeout_secs: 30,
        })
    }
}

fn env_duration(name: &str, default_secs: u64) -> Duration {
    Duration::from_secs(
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|value| *value > 0)
            .unwrap_or(default_secs),
    )
}

fn env_usize(name: &str, default_value: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default_value)
}

async fn shutdown_signal(worker_handle: Option<WorkerHandle>) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    if let Some(worker_handle) = worker_handle {
        worker_handle.shutdown().await;
    }
}
