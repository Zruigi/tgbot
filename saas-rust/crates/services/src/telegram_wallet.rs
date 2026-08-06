use crate::telegram_preferences::{normalize_telegram_locale, DEFAULT_TELEGRAM_LOCALE};
use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedWalletAddress {
    pub address: String,
    pub network: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletRegistrationStatus {
    Created,
    Reactivated,
    AlreadyActive,
    AlreadyRegistered,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletMonitorRegistration {
    pub monitor_id: Option<i64>,
    pub address: String,
    pub network: String,
    pub status: WalletRegistrationStatus,
}

pub fn normalize_wallet_address(raw: &str) -> Result<NormalizedWalletAddress> {
    let address = raw.trim();
    if address.starts_with("0x") || address.starts_with("0X") {
        let body = &address[2..];
        if body.len() != 40 || !body.chars().all(|value| value.is_ascii_hexdigit()) {
            return Err(anyhow!("invalid BSC address"));
        }
        return Ok(NormalizedWalletAddress {
            address: format!("0x{}", body.to_ascii_lowercase()),
            network: "BSC".to_string(),
        });
    }

    if address.len() != 34
        || !address.starts_with('T')
        || !address.chars().all(is_tron_base58_character)
    {
        return Err(anyhow!("invalid TRC20 address"));
    }
    Ok(NormalizedWalletAddress {
        address: address.to_string(),
        network: "TRC20".to_string(),
    })
}

fn is_tron_base58_character(value: char) -> bool {
    value.is_ascii_alphanumeric() && !matches!(value, '0' | 'O' | 'I' | 'l')
}

pub async fn register_wallet_monitor(
    pool: &PgPool,
    tenant_id: &str,
    user_id: i64,
    raw_address: &str,
) -> Result<WalletMonitorRegistration> {
    if tenant_id.trim().is_empty() || user_id <= 0 {
        return Err(anyhow!("wallet monitor scope is invalid"));
    }
    let normalized = normalize_wallet_address(raw_address)?;
    let baseline_ms = Utc::now().timestamp_millis();
    let mut transaction = pool.begin().await?;
    let existing = sqlx::query(
        "SELECT monitor_id, user_id, is_active FROM address_monitors \
         WHERE tenant_id = $1 AND address = $2 AND network = $3 FOR UPDATE",
    )
    .bind(tenant_id)
    .bind(&normalized.address)
    .bind(&normalized.network)
    .fetch_optional(&mut *transaction)
    .await?;

    let registration = if let Some(row) = existing {
        let monitor_id: i64 = row.try_get("monitor_id")?;
        let owner_id: i64 = row.try_get("user_id")?;
        let is_active: bool = row.try_get("is_active")?;
        if owner_id != user_id {
            WalletMonitorRegistration {
                monitor_id: None,
                address: normalized.address.clone(),
                network: normalized.network.clone(),
                status: WalletRegistrationStatus::AlreadyRegistered,
            }
        } else if is_active {
            WalletMonitorRegistration {
                monitor_id: Some(monitor_id),
                address: normalized.address.clone(),
                network: normalized.network.clone(),
                status: WalletRegistrationStatus::AlreadyActive,
            }
        } else {
            sqlx::query(
                "UPDATE address_monitors SET is_active = true, last_tx_timestamp = $4, updated_at = now() \
                 WHERE monitor_id = $1 AND tenant_id = $2 AND user_id = $3",
            )
            .bind(monitor_id)
            .bind(tenant_id)
            .bind(user_id)
            .bind(baseline_ms)
            .execute(&mut *transaction)
            .await?;
            WalletMonitorRegistration {
                monitor_id: Some(monitor_id),
                address: normalized.address.clone(),
                network: normalized.network.clone(),
                status: WalletRegistrationStatus::Reactivated,
            }
        }
    } else {
        let monitor_id: i64 = sqlx::query_scalar(
            "INSERT INTO address_monitors \
             (user_id, tenant_id, address, network, is_active, last_tx_timestamp) \
             VALUES ($1, $2, $3, $4, true, $5) \
             RETURNING monitor_id",
        )
        .bind(user_id)
        .bind(tenant_id)
        .bind(&normalized.address)
        .bind(&normalized.network)
        .bind(baseline_ms)
        .fetch_one(&mut *transaction)
        .await?;
        WalletMonitorRegistration {
            monitor_id: Some(monitor_id),
            address: normalized.address.clone(),
            network: normalized.network.clone(),
            status: WalletRegistrationStatus::Created,
        }
    };

    transaction.commit().await?;
    Ok(registration)
}

pub fn wallet_monitor_prompt(locale: &str) -> String {
    if normalize_telegram_locale(locale).unwrap_or(DEFAULT_TELEGRAM_LOCALE) == "en" {
        "Send `/wallet_monitor <TRC20-or-BSC-address>` to add an address monitor. No chain history is queried by this command.".to_string()
    } else {
        "请发送 `/wallet_monitor <TRC20或BSC地址>` 添加地址监听。该命令不会读取历史链上账单。"
            .to_string()
    }
}

pub fn wallet_registration_reply(registration: &WalletMonitorRegistration, locale: &str) -> String {
    let is_english = normalize_telegram_locale(locale).unwrap_or(DEFAULT_TELEGRAM_LOCALE) == "en";
    let status = match registration.status {
        WalletRegistrationStatus::Created => {
            if is_english {
                "registered"
            } else {
                "已登记"
            }
        }
        WalletRegistrationStatus::Reactivated => {
            if is_english {
                "reactivated"
            } else {
                "已重新启用"
            }
        }
        WalletRegistrationStatus::AlreadyActive => {
            if is_english {
                "already active"
            } else {
                "已经在监听"
            }
        }
        WalletRegistrationStatus::AlreadyRegistered => {
            if is_english {
                "already registered in this tenant"
            } else {
                "当前租户已登记该地址"
            }
        }
    };
    if is_english {
        format!(
            "✅ Address {status}.\nNetwork: `{}`\nAddress: `{}`\n\nThe monitor baseline starts at registration time; transaction synchronization and Telegram alerts require the enabled Beta monitor worker.",
            registration.network, registration.address
        )
    } else {
        format!(
            "✅ 地址{status}。\n网络：`{}`\n地址：`{}`\n\n监听基线从登记时间开始；交易同步和 Telegram 推送需要启用 Beta 监听 Worker。",
            registration.network, registration.address
        )
    }
}

pub async fn wallet_list_reply(
    pool: &PgPool,
    tenant_id: &str,
    user_id: i64,
) -> Result<(String, Option<Value>)> {
    wallet_list_reply_for_locale(pool, tenant_id, user_id, DEFAULT_TELEGRAM_LOCALE).await
}

pub async fn wallet_list_reply_for_locale(
    pool: &PgPool,
    tenant_id: &str,
    user_id: i64,
    locale: &str,
) -> Result<(String, Option<Value>)> {
    let is_english = normalize_telegram_locale(locale).unwrap_or(DEFAULT_TELEGRAM_LOCALE) == "en";
    let rows = sqlx::query(
        "SELECT monitor_id, address, network, remark \
         FROM address_monitors \
         WHERE tenant_id = $1 AND user_id = $2 AND is_active = true \
         ORDER BY created_at DESC \
         LIMIT 25",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let row_count = rows.len();
    let mut text = if row_count == 0 {
        if is_english {
            "There are no active address monitors.".to_string()
        } else {
            "当前没有启用的地址监听。".to_string()
        }
    } else {
        if is_english {
            "📡 Active address monitors:".to_string()
        } else {
            "📡 当前启用的地址监听：".to_string()
        }
    };
    let mut keyboard = Vec::with_capacity(row_count + 2);
    let mut displayed_count = 0usize;

    for row in rows {
        let monitor_id: i64 = row.try_get("monitor_id")?;
        let address: String = row.try_get("address")?;
        let network: String = row.try_get("network")?;
        let remark: Option<String> = row.try_get("remark")?;
        let address = truncate_display(&address, 120);
        let network = truncate_display(&network, 32);
        let remark = remark
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| truncate_display(value, 120));
        let mut item = format!("\n\n#{monitor_id} · {network}\n{address}");
        if let Some(remark) = remark {
            item.push_str(if is_english {
                "\nRemark: "
            } else {
                "\n备注："
            });
            item.push_str(&remark);
        }
        if text.len() + item.len() > 3500 {
            break;
        }
        text.push_str(&item);
        keyboard.push(vec![json!({
            "text": format!("取消 #{monitor_id}"),
            "callback_data": format!("uw_u_{monitor_id}"),
        })]);
        displayed_count += 1;
    }

    if displayed_count < row_count {
        text.push_str(if is_english {
            "\n\nThe list is truncated. View the full list in the web console."
        } else {
            "\n\n列表已截断，请通过网页端查看完整列表。"
        });
    }
    keyboard.push(vec![json!({
        "text": if is_english { "Refresh" } else { "刷新" },
        "callback_data": "wallet_my_list",
    })]);
    keyboard.push(vec![json!({
        "text": if is_english { "Back to main menu" } else { "返回主菜单" },
        "callback_data": "back_to_start",
    })]);

    Ok((text, Some(json!({ "inline_keyboard": keyboard }))))
}

pub async fn wallet_admin_list_reply(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<(String, Option<Value>)> {
    wallet_admin_list_reply_for_locale(pool, tenant_id, DEFAULT_TELEGRAM_LOCALE).await
}

pub async fn wallet_admin_list_reply_for_locale(
    pool: &PgPool,
    tenant_id: &str,
    locale: &str,
) -> Result<(String, Option<Value>)> {
    let is_english = normalize_telegram_locale(locale).unwrap_or(DEFAULT_TELEGRAM_LOCALE) == "en";
    let rows = sqlx::query(
        "SELECT monitor_id, user_id, address, network, remark \
         FROM address_monitors \
         WHERE tenant_id = $1 AND is_active = true \
         ORDER BY created_at DESC \
         LIMIT 100",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    let row_count = rows.len();
    let mut text = if row_count == 0 {
        if is_english {
            "There are no active address monitors in this tenant.".to_string()
        } else {
            "当前租户没有启用的地址监听。".to_string()
        }
    } else {
        if is_english {
            "📡 Active address monitors in this tenant:".to_string()
        } else {
            "📡 当前租户启用的地址监听：".to_string()
        }
    };
    let mut keyboard = Vec::with_capacity(row_count + 1);
    let mut displayed_count = 0usize;

    for row in rows {
        let monitor_id: i64 = row.try_get("monitor_id")?;
        let user_id: i64 = row.try_get("user_id")?;
        let address: String = row.try_get("address")?;
        let network: String = row.try_get("network")?;
        let remark: Option<String> = row.try_get("remark")?;
        let address = truncate_display(&address, 120);
        let network = truncate_display(&network, 32);
        let remark = remark
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| truncate_display(value, 120));
        let user_label = if is_english { "User" } else { "用户" };
        let mut item = format!("\n\n#{monitor_id} · {user_label} {user_id} · {network}\n{address}");
        if let Some(remark) = remark {
            item.push_str(if is_english {
                "\nRemark: "
            } else {
                "\n备注："
            });
            item.push_str(&remark);
        }
        if text.len() + item.len() > 3500 {
            break;
        }
        text.push_str(&item);
        keyboard.push(vec![json!({
            "text": format!("取消 #{monitor_id}"),
            "callback_data": format!("uw_a_{monitor_id}"),
        })]);
        displayed_count += 1;
    }

    if displayed_count < row_count {
        text.push_str(if is_english {
            "\n\nThe list is truncated. View the full list in the web console."
        } else {
            "\n\n列表已截断，请通过网页端查看完整列表。"
        });
    }
    keyboard.push(vec![json!({
        "text": if is_english { "Back to main menu" } else { "返回主菜单" },
        "callback_data": "back_to_start",
    })]);

    Ok((text, Some(json!({ "inline_keyboard": keyboard }))))
}

fn truncate_display(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let result: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{result}…")
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_supported_wallet_addresses() {
        assert_eq!(
            normalize_wallet_address(" 0X000000000000000000000000000000000000000A ").unwrap(),
            NormalizedWalletAddress {
                address: "0x000000000000000000000000000000000000000a".to_string(),
                network: "BSC".to_string(),
            }
        );
        assert_eq!(
            normalize_wallet_address("TQJm3eQp8j7k2vH9cA6bK5xN4wR3sD2fG1").unwrap(),
            NormalizedWalletAddress {
                address: "TQJm3eQp8j7k2vH9cA6bK5xN4wR3sD2fG1".to_string(),
                network: "TRC20".to_string(),
            }
        );
    }

    #[test]
    fn rejects_malformed_wallet_addresses() {
        assert!(normalize_wallet_address("0x1234").is_err());
        assert!(normalize_wallet_address("T0Jm3eQp8j7k2vH9cA6bK5xN4wR3sD2fG1").is_err());
    }
}
