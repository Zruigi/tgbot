pub mod agent_tools;
pub mod ai;
pub mod auth;
pub mod broadcast;
pub mod chat;
pub mod config;
pub mod finance;
pub mod group;
pub mod group_listen;
pub mod group_members;
pub mod group_verify;
pub mod merchant;
pub mod miniapp;
pub mod monitor;
pub mod platform;
pub mod potential_users;
pub mod session;
pub mod telegram_webhook;

use axum::{
    routing::{delete, get, post},
    Router,
};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        // Health
        .route("/healthz", get(session::health))
        .route("/api/health", get(session::health))

        // Auth routes - Login, OTP, CSRF Token, TMA login
        .route("/api/login", post(auth::login))
        .route("/api/login/otp", post(auth::login_otp))
        .route("/api/login/tma", post(auth::login_tma))
        .route("/api/csrf-token", get(auth::csrf_token))

        // System Config routes
        .route("/api/config", get(config::get_config).post(config::save_config))
        .route("/api/config/vertex", post(config::vertex_oneclick_config))
        .route("/api/config/vertex/oneclick", post(config::vertex_oneclick_config)) // alias for frontend
        .route("/api/config/gemini_models", get(config::get_gemini_models))
        .route("/api/config/gemini-models", get(config::get_gemini_models)) // alias for frontend

        // Session context
        .route("/api/session/context", get(session::session_context_get).post(session::session_context_post))

        // MiniApp routes
        .route("/api/miniapp/bootstrap", post(miniapp::bootstrap))
        .route("/api/miniapp/context", get(miniapp::context))
        .route("/api/miniapp/logout_revoke", post(miniapp::logout_revoke))

        // Platform routes
        .route(
            "/api/platform/merchants",
            get(platform::list_merchants).post(platform::create_merchant),
        )
        .route(
            "/api/platform/merchants/{merchant_id}",
            delete(platform::delete_merchant),
        )
        .route(
            "/api/platform/merchants/confirm_onboarding",
            post(platform::confirm_onboarding),
        )
        .route(
            "/api/platform/plans",
            get(platform::list_plans).post(platform::upsert_plan),
        )
        .route(
            "/api/platform/subscriptions",
            get(platform::list_subscriptions),
        )
        .route(
            "/api/platform/subscriptions/downgrade",
            post(platform::run_subscription_downgrade),
        )
        .route(
            "/api/platform/payment_orders",
            get(platform::list_payment_orders),
        )
        .route(
            "/api/platform/payment_orders/confirm",
            post(platform::confirm_payment_order),
        )
        .route(
            "/api/platform/payment_orders/reject",
            post(platform::reject_payment_order),
        )
        .route(
            "/api/platform/payment_config",
            get(platform::get_payment_config).post(platform::save_payment_config),
        )
        .route(
            "/api/platform/runtime_statuses",
            get(platform::runtime_statuses),
        )

        // Merchant routes
        .route("/api/merchant/profile", get(merchant::profile))
        .route("/api/merchant/status", post(merchant::update_status))
        .route(
            "/api/merchant/bot_token",
            post(merchant::save_bot_token_ref),
        )
        .route("/api/merchant/subscription", get(merchant::subscription))
        .route("/api/merchant/plans", get(merchant::plans))
        .route(
            "/api/merchant/payment_orders",
            post(merchant::create_payment_order),
        )
        .route(
            "/api/merchant/payment_orders/latest",
            get(merchant::latest_payment_order),
        )
        .route(
            "/api/merchant/payment_orders/mark_paid",
            post(merchant::mark_payment_order_paid),
        )
        .route(
            "/api/merchant/payment_orders/submit_tx",
            post(merchant::submit_payment_tx),
        )
        .route("/api/merchant/features", get(merchant::features))
        .route("/api/merchant/ai/quota", get(merchant::ai_quota))
        .route("/api/merchant/ai/usage", get(merchant::ai_usage))
        .route("/api/merchant/audit_logs", get(merchant::audit_logs))
        .route(
            "/api/merchant/payment_config",
            get(platform::get_payment_config),
        )

        // Chat / Messages routes
        .route("/api/users", get(chat::get_users))
        .route("/api/messages/{user_id}", get(chat::get_messages).post(chat::send_message))
        .route("/api/messages/{user_id}/delete", post(chat::delete_single_message))
        .route("/api/messages/{user_id}/ai", post(chat::ai_reply))
        .route("/api/mark_read/{user_id}", post(chat::mark_read))
        .route("/api/delete_chat/{user_id}", post(chat::delete_chat))
        .route("/api/search", get(chat::search))
        .route("/api/file/{file_id}", get(chat::get_file))
        .route("/api/upload/{user_id}", post(chat::upload_image))
        .route("/ws", get(chat::ws_handler))

        // Group routes
        .route("/api/groups", get(group::list_groups))
        .route("/api/groups/config", post(group::update_group_config))

        // Group Members routes
        .route("/api/group_members", get(group_members::list_members))
        .route("/api/group_members/permissions", get(group_members::get_permissions))
        .route("/api/group_members/action", post(group_members::member_action))
        .route("/api/group_members/action_batch", post(group_members::member_action_batch))
        .route("/api/group_members/admin_batch", post(group_members::admin_batch))
        .route("/api/group_members/promote_admin", post(group_members::promote_admin))
        .route("/api/group_members/demote_admin", post(group_members::demote_admin))

        // Group Messages routes
        .route("/api/group_messages", get(group_members::list_group_messages))
        .route("/api/group_messages/send", post(group_members::send_group_message))
        .route("/api/group_messages/reply", post(group_members::reply_group_message))
        .route("/api/group_messages/delete", post(group_members::delete_group_message))

        // Group Verify Config routes
        .route("/api/group_verify_config", get(group_verify::get_verify_config).post(group_verify::update_verify_config))
        .route("/api/verify/stats", get(group_verify::verify_stats))
        .route("/api/verify/growth", get(group_verify::verify_growth))
        // Aliases for frontend compatibility
        .route("/api/group_stats/verify_stats", get(group_verify::verify_stats))
        .route("/api/group_stats/verify_growth", get(group_verify::verify_growth))

        // Group Listen API — frontend compatibility aliases
        .route("/api/group_listen/list", get(group::list_groups))
        .route("/api/group_listen/stats", get(group_listen::get_monitor_stats))
        .route("/api/group_listen/toggle", post(group_listen::toggle_group_listen))
        .route("/api/group_listen/revoke_sync", post(group_listen::toggle_revoke_sync))
        .route("/api/group_listen/create_sync_chat", post(group_listen::create_revoke_sync_chat))
        .route("/api/group_listen/sync_chat_info", get(group_listen::get_revoke_sync_chat))

        // Group photo & member count
        .route("/api/group_photo", get(group_listen::get_group_photo))
        .route("/api/group_member_count", get(group_listen::get_group_member_count))

        // Group Listen extended endpoints
        .route("/api/group_listen/verified_users", get(group_listen::get_verified_users))
        .route("/api/group_listen/unverified_users", get(group_listen::get_unverified_users))
        .route("/api/group_listen/send_channel_reminder", post(group_listen::send_channel_reminder))
        .route("/api/group_listen/sync_chat_assignments", get(group_listen::get_sync_chat_assignments))

        // Potential Users (leads) API
        .route("/api/potential_users", get(potential_users::list_potential_users))
        .route("/api/potential_users/tags", get(potential_users::get_available_tags))
        .route("/api/potential_users/sync", post(potential_users::sync_potential_users))
        .route("/api/potential_users/{user_id}/promote", post(potential_users::promote_user))

        // Broadcast / Push Task routes
        .route(
            "/api/broadcast/targets",
            get(broadcast::list_broadcast_targets),
        )
        .route(
            "/api/broadcast/targets/sync",
            post(broadcast::sync_broadcast_targets),
        )
        .route("/api/broadcast", post(broadcast::send_broadcast))
        .route("/api/broadcast/chats", post(broadcast::broadcast_chats))
        .route("/api/broadcast/send", post(broadcast::send_broadcast_alias))
        .route("/api/broadcast/unified", post(broadcast::broadcast_unified))
        .route("/api/push_tasks", get(broadcast::list_push_tasks).post(broadcast::create_push_task))
        .route(
            "/api/push_tasks/status",
            post(broadcast::update_push_task_status),
        )
        .route(
            "/api/push_tasks/run_now",
            post(broadcast::run_push_task_now),
        )

        // Monitor routes - Wallet monitoring
        .route(
            "/api/monitors",
            get(monitor::list_monitors).post(monitor::create_monitor),
        )
        .route(
            "/api/monitors/{monitor_id}",
            delete(monitor::delete_monitor),
        )
        .route(
            "/api/monitors/{monitor_id}/remark",
            post(monitor::update_monitor_remark),
        )
        .route(
            "/api/monitors/pnl_summary",
            get(monitor::pnl_summary),
        )
        .route(
            "/api/monitors/tx_meta",
            post(monitor::update_tx_meta),
        )
        .route(
            "/api/monitors/export",
            get(monitor::export_transactions),
        )
        .route(
            "/api/monitors/transactions",
            get(monitor::list_transactions),
        )
        .route(
            "/api/monitors/year_summary",
            get(monitor::year_summary),
        )
        .route(
            "/api/monitors/{monitor_id}/claim",
            post(monitor::claim_monitor),
        )
        .route(
            "/api/monitors/tx_meta/batch",
            post(monitor::batch_update_tx_meta),
        )
        .route(
            "/api/monitors/counterparty_aliases",
            get(monitor::list_counterparty_aliases).post(monitor::upsert_counterparty_alias),
        )
        .route(
            "/api/monitors/counterparty_aliases/{alias_id}",
            delete(monitor::delete_counterparty_alias),
        )
        .route(
            "/api/monitors/user_queries",
            get(monitor::list_user_query_monitors),
        )

        // Finance routes - Financial summary, reconciliation
        .route(
            "/api/finance/summary",
            get(finance::get_summary),
        )
        .route(
            "/api/finance/reconcile",
            post(finance::reconcile),
        )
        .route(
            "/api/finance/categories",
            get(finance::get_categories),
        )
        .route(
            "/api/finance/calculate_pnl",
            post(finance::calculate_pnl),
        )
        .route(
            "/api/finance/report",
            post(finance::generate_report),
        )

        // AI / Agent routes
        .route("/api/ai/chat", post(ai::ai_chat))
        .route("/api/ai/detect_ad", post(ai::detect_ad))
        .route("/api/ai/pipeline_mode", get(ai::get_pipeline_mode).post(ai::set_pipeline_mode))
        .route("/api/openai/models", get(ai::list_openai_models))
        .route("/api/openai/chat/completions", post(ai::openai_chat_completions))
        .route("/api/openai/exec", post(ai::openai_exec))

        // AI Skills routes — frontend & n8n compatibility
        .route("/api/ai/skills", get(ai::list_skills).post(ai::upsert_skill))
        .route("/api/ai/skills/toggle", post(ai::toggle_skill))
        .route("/api/ai/skills/import_builtin", post(ai::import_builtin_skills))
        .route("/api/ai/skills/discovered", get(ai::list_discovered_skills))

        // Ad pipeline routes — n8n AdCluster compatibility
        .route("/api/ad-pipeline/events/group-message", post(ai::ad_pipeline_group_message))
        .route("/api/ad-pipeline/backfill", post(ai::ad_pipeline_backfill))

        // Agent tool endpoints (for n8n integration)
        .route("/api/agent/tools/finance-query", post(agent_tools::tool_finance_query))
        .route("/api/agent/tools/system-query", post(agent_tools::tool_system_query))
        .route("/api/agent/tools/telegram-sdk-query", post(agent_tools::tool_telegram_sdk_query))
        .route("/api/agent/tools/query-ad-clusters", post(agent_tools::tool_query_ad_clusters))
        .route("/api/agent/tools/delete-ad-cluster", post(agent_tools::tool_delete_ad_cluster))

        // n8n advertisement compatibility
        .route("/api/advertisements", get(ai::list_advertisements))
        .route("/api/advertisements/{ad_id}", get(ai::get_advertisement))

        // Telegram Webhook routes — multi-tenant bot webhook endpoint
        .route(
            "/telegram/webhook/{bot_id}",
            post(telegram_webhook::webhook_handler),
        )
        .route(
            "/telegram/webhook/{bot_id}/register",
            post(telegram_webhook::register_bot)
                .delete(telegram_webhook::unregister_bot),
        )
        .route(
            "/telegram/webhook/bots",
            get(telegram_webhook::list_bots),
        )
        .route(
            "/telegram/webhook/dispatch-test",
            post(telegram_webhook::dispatch_test),
        )
}
