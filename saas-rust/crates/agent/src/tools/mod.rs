pub mod broadcast_message;
pub mod delete_recent_ad_messages;
pub mod detect_advertising;
pub mod finance_query;
// The historical module is empty; the real content-generation tool lives in
// core.rs and is registered there. Keep this only for explicit legacy audits.
#[cfg(feature = "legacy-stubs")]
pub mod generate_content;
pub mod n8n_workflow;
pub mod ocr;
pub mod query_recent_ad_messages;
pub mod skill_manage;
pub mod system_query;
