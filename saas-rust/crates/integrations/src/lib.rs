pub mod binance;
// BscScan was an empty historical placeholder and is not part of the formal
// app dependency graph. Compile it only for an explicit legacy audit.
#[cfg(feature = "legacy-stubs")]
pub mod bscscan;
pub mod litellm;
pub mod n8n;
pub mod paddleocr;
pub mod supabase_rest;
pub mod telegram;
pub mod trongrid;
