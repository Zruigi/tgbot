use axum::{
    routing::{delete, get, post},
    Router,
};

use crate::state::AppState;

mod merchant;
mod platform;
mod session;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(session::health))
        .route("/api/health", get(session::health))
        .route("/api/session/context", get(session::session_context))
        .route(
            "/api/platform/merchants",
            get(platform::list_merchants).post(platform::create_merchant),
        )
        .route(
            "/api/platform/merchants/:merchant_id",
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
}
