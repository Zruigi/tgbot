pub mod auth;
pub mod broadcast;
pub mod chat;
pub mod config;
pub mod finance;
pub mod group;
pub mod group_members;
pub mod group_verify;
pub mod merchant;
pub mod miniapp;
pub mod monitor;
pub mod platform;
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
        .route("/api/config/gemini_models", get(config::get_gemini_models))

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
