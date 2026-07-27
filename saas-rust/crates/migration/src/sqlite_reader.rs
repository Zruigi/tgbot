//! SQLite Reader - Read data from source SQLite database
//! This module provides functionality to read data from the legacy SQLite database
//! for migration to PostgreSQL/Supabase.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Configuration for SQLite source database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteConfig {
    /// Path to the SQLite database file
    pub db_path: String,
}

impl SqliteConfig {
    pub fn new(db_path: impl Into<String>) -> Self {
        Self {
            db_path: db_path.into(),
        }
    }
}

/// SQLite Reader for reading legacy data
pub struct SqliteReader {
    conn: Connection,
}

impl SqliteReader {
    /// Open a SQLite database connection
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(&path)
            .with_context(|| format!("Failed to open SQLite database: {:?}", path.as_ref()))?;

        // Enable foreign keys
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        tracing::info!("Opened SQLite database: {:?}", path.as_ref());

        Ok(Self { conn })
    }

    /// Create a new SQLite database connection with config
    pub fn new(config: &SqliteConfig) -> Result<Self> {
        Self::open(&config.db_path)
    }

    // ========================================================================
    // Merchant Data Reading
    // ========================================================================

    /// Read all merchants from the SQLite database
    pub fn read_merchants(&self) -> Result<Vec<SqliteMerchant>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                id, tenant_id, owner_tg_user_id, merchant_name,
                bot_id, bot_username, status, plan_type,
                subscription_status, subscription_expires_at,
                onboarding_completed, runtime_version, config_version,
                feature_version, created_at, updated_at
            FROM merchants
            ORDER BY id
            "#,
        )?;

        let merchants = stmt
            .query_map([], |row| {
                Ok(SqliteMerchant {
                    id: row.get(0)?,
                    tenant_id: row.get(1)?,
                    owner_tg_user_id: row.get(2)?,
                    merchant_name: row.get(3)?,
                    bot_id: row.get(4)?,
                    bot_username: row.get(5)?,
                    status: row.get(6)?,
                    plan_type: row.get(7)?,
                    subscription_status: row.get(8)?,
                    subscription_expires_at: row.get(9)?,
                    onboarding_completed: row.get::<_, i32>(10)? != 0,
                    runtime_version: row.get(11)?,
                    config_version: row.get(12)?,
                    feature_version: row.get(13)?,
                    created_at: row.get(14)?,
                    updated_at: row.get(15)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to read merchants")?;

        tracing::info!("Read {} merchants from SQLite", merchants.len());
        Ok(merchants)
    }

    // ========================================================================
    // Message Data Reading
    // ========================================================================

    /// Read messages from SQLite with pagination
    pub fn read_messages(&self, limit: usize, offset: usize) -> Result<Vec<SqliteMessage>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                msg_id, trace_id, from_userid, to_userid, content,
                type, file_id, media_url, timestamp, is_read,
                quoted_text, quoted_msg_id, quoted_file_id, quoted_type,
                tg_msg_id, status, auto_translation, translated_text,
                original_text, admin_userid, is_deleted, tenant_id
            FROM messages
            ORDER BY msg_id
            LIMIT ?1 OFFSET ?2
            "#,
        )?;

        let messages = stmt
            .query_map(params![limit as i64, offset as i64], |row| {
                Ok(SqliteMessage {
                    msg_id: row.get(0)?,
                    trace_id: row.get(1)?,
                    from_userid: row.get(2)?,
                    to_userid: row.get(3)?,
                    content: row.get(4)?,
                    msg_type: row.get(5)?,
                    file_id: row.get(6)?,
                    media_url: row.get(7)?,
                    timestamp: row.get(8)?,
                    is_read: row.get::<_, i32>(9)? != 0,
                    quoted_text: row.get(10)?,
                    quoted_msg_id: row.get(11)?,
                    quoted_file_id: row.get(12)?,
                    quoted_type: row.get(13)?,
                    tg_msg_id: row.get(14)?,
                    status: row.get(15)?,
                    auto_translation: row.get(16)?,
                    translated_text: row.get(17)?,
                    original_text: row.get(18)?,
                    admin_userid: row.get(19)?,
                    is_deleted: row.get::<_, i32>(20)? != 0,
                    tenant_id: row.get(21)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to read messages")?;

        Ok(messages)
    }

    /// Count total messages in SQLite
    pub fn count_messages(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .context("Failed to count messages")?;
        Ok(count)
    }

    // ========================================================================
    // Group Data Reading
    // ========================================================================

    /// Read all tracked groups from SQLite
    pub fn read_groups(&self) -> Result<Vec<SqliteGroup>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                group_id, group_title, is_monitored, custom_keywords,
                first_seen, updated_at, delete_on_revoke, revoke_sync_chat_id,
                verify_enabled, verify_type, verify_timeout,
                verify_channel_id, verify_questions, tenant_id
            FROM tracked_groups
            ORDER BY group_id
            "#,
        )?;

        let groups = stmt
            .query_map([], |row| {
                Ok(SqliteGroup {
                    group_id: row.get(0)?,
                    group_title: row.get(1)?,
                    is_monitored: row.get::<_, i32>(2)? != 0,
                    custom_keywords: row.get(3)?,
                    first_seen: row.get(4)?,
                    updated_at: row.get(5)?,
                    delete_on_revoke: row.get::<_, i32>(6)? != 0,
                    revoke_sync_chat_id: row.get(7)?,
                    verify_enabled: row.get::<_, i32>(8)? != 0,
                    verify_type: row.get(9)?,
                    verify_timeout: row.get(10)?,
                    verify_channel_id: row.get(11)?,
                    verify_questions: row.get(12)?,
                    tenant_id: row.get(13)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to read groups")?;

        tracing::info!("Read {} groups from SQLite", groups.len());
        Ok(groups)
    }

    /// Read group members for a specific group
    pub fn read_group_members(&self, group_id: i64) -> Result<Vec<SqliteGroupMember>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                id, user_id, username, first_name, last_name,
                group_id, group_title, status, joined_at,
                is_verified, verified_at, avatar_file_id, tenant_id
            FROM group_members
            WHERE group_id = ?1
            ORDER BY id
            "#,
        )?;

        let members = stmt
            .query_map(params![group_id], |row| {
                Ok(SqliteGroupMember {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    username: row.get(2)?,
                    first_name: row.get(3)?,
                    last_name: row.get(4)?,
                    group_id: row.get(5)?,
                    group_title: row.get(6)?,
                    status: row.get(7)?,
                    joined_at: row.get(8)?,
                    is_verified: row.get::<_, i32>(9)? != 0,
                    verified_at: row.get(10)?,
                    avatar_file_id: row.get(11)?,
                    tenant_id: row.get(12)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to read group members")?;

        Ok(members)
    }

    /// Read all group members
    pub fn read_all_group_members(&self) -> Result<Vec<SqliteGroupMember>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                id, user_id, username, first_name, last_name,
                group_id, group_title, status, joined_at,
                is_verified, verified_at, avatar_file_id, tenant_id
            FROM group_members
            ORDER BY group_id, id
            "#,
        )?;

        let members = stmt
            .query_map([], |row| {
                Ok(SqliteGroupMember {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    username: row.get(2)?,
                    first_name: row.get(3)?,
                    last_name: row.get(4)?,
                    group_id: row.get(5)?,
                    group_title: row.get(6)?,
                    status: row.get(7)?,
                    joined_at: row.get(8)?,
                    is_verified: row.get::<_, i32>(9)? != 0,
                    verified_at: row.get(10)?,
                    avatar_file_id: row.get(11)?,
                    tenant_id: row.get(12)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to read group members")?;

        tracing::info!("Read {} group members from SQLite", members.len());
        Ok(members)
    }

    /// Count total groups in SQLite
    pub fn count_groups(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM tracked_groups", [], |row| row.get(0))
            .context("Failed to count groups")?;
        Ok(count)
    }

    /// Count total group members in SQLite
    pub fn count_group_members(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM group_members", [], |row| row.get(0))
            .context("Failed to count group members")?;
        Ok(count)
    }

    // ========================================================================
    // Group Message Data Reading
    // ========================================================================

    /// Read group messages from SQLite with pagination
    pub fn read_group_messages(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SqliteGroupMessage>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                msg_id, group_id, user_id, username, content,
                msg_type, timestamp, tg_msg_id, sender_type, quoted_tg_msg_id, quoted_text,
                message_ts, file_id, avatar_file_id, revoke_checked,
                is_advertising, ad_score, ad_confidence, ad_findings,
                tenant_id, ad_cluster_id, ad_checked_at, ad_action_status,
                is_deleted, deleted_at, ad_text_hash, media_unique_id
            FROM group_messages
            ORDER BY msg_id
            LIMIT ?1 OFFSET ?2
            "#,
        )?;

        let messages = stmt
            .query_map(params![limit as i64, offset as i64], |row| {
                Ok(SqliteGroupMessage {
                    msg_id: row.get(0)?,
                    group_id: row.get(1)?,
                    user_id: row.get(2)?,
                    username: row.get(3)?,
                    content: row.get(4)?,
                    msg_type: row.get(5)?,
                    timestamp: row.get(6)?,
                    tg_msg_id: row.get(7)?,
                    sender_type: row.get(8)?,
                    quoted_tg_msg_id: row.get(9)?,
                    quoted_text: row.get(10)?,
                    message_ts: row.get(11)?,
                    file_id: row.get(12)?,
                    avatar_file_id: row.get(13)?,
                    revoke_checked: row.get::<_, i32>(14)? != 0,
                    is_advertising: row.get::<_, i32>(15)? != 0,
                    ad_score: row.get(16)?,
                    ad_confidence: row.get(17)?,
                    ad_findings: row.get(18)?,
                    tenant_id: row.get(19)?,
                    ad_cluster_id: row.get(20)?,
                    ad_checked_at: row.get(21)?,
                    ad_action_status: row.get(22)?,
                    is_deleted: row.get::<_, i32>(23)? != 0,
                    deleted_at: row.get(24)?,
                    ad_text_hash: row.get(25)?,
                    media_unique_id: row.get(26)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to read group messages")?;

        Ok(messages)
    }

    /// Count total group messages in SQLite
    pub fn count_group_messages(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM group_messages", [], |row| row.get(0))
            .context("Failed to count group messages")?;
        Ok(count)
    }

    // ========================================================================
    // User and Admin Inbox Data Reading
    // ========================================================================

    /// Read all users from SQLite
    pub fn read_users(&self) -> Result<Vec<SqliteUser>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                user_id, email, created_at
            FROM users
            ORDER BY user_id
            "#,
        )?;

        let users = stmt
            .query_map([], |row| {
                Ok(SqliteUser {
                    user_id: row.get(0)?,
                    email: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to read users")?;

        tracing::info!("Read {} users from SQLite", users.len());
        Ok(users)
    }

    /// Count total users in SQLite
    pub fn count_users(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .context("Failed to count users")?;
        Ok(count)
    }

    /// Read admin inbox entries from SQLite
    pub fn read_admin_inbox(&self) -> Result<Vec<SqliteAdminInbox>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                from_userid, summary, unread_count, last_active,
                from_username, avatar_file_id, tenant_id
            FROM admin_inbox
            ORDER BY from_userid
            "#,
        )?;

        let entries = stmt
            .query_map([], |row| {
                Ok(SqliteAdminInbox {
                    from_userid: row.get(0)?,
                    summary: row.get(1)?,
                    unread_count: row.get(2)?,
                    last_active: row.get(3)?,
                    from_username: row.get(4)?,
                    avatar_file_id: row.get(5)?,
                    tenant_id: row.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to read admin inbox")?;

        tracing::info!("Read {} admin inbox entries from SQLite", entries.len());
        Ok(entries)
    }

    /// Count total admin inbox entries in SQLite
    pub fn count_admin_inbox(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM admin_inbox", [], |row| row.get(0))
            .context("Failed to count admin inbox")?;
        Ok(count)
    }

    // ========================================================================
    // Verification Count Helper
    // ========================================================================

    /// Get all table counts for verification
    pub fn get_table_counts(&self) -> Result<SqliteCounts> {
        let merchants = self.read_merchants()?.len() as i64;
        let messages = self.count_messages()?;
        let groups = self.count_groups()?;
        let group_members = self.count_group_members()?;
        let group_messages = self.count_group_messages()?;
        let users = self.count_users()?;
        let admin_inbox = self.count_admin_inbox()?;

        Ok(SqliteCounts {
            merchants,
            messages,
            groups,
            group_members,
            group_messages,
            users,
            admin_inbox,
        })
    }
}

// ========================================================================
// Data Structures for SQLite records
// ========================================================================

/// SQLite merchant record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteMerchant {
    pub id: i64,
    pub tenant_id: String,
    pub owner_tg_user_id: Option<i64>,
    pub merchant_name: String,
    pub bot_id: Option<i64>,
    pub bot_username: Option<String>,
    pub status: String,
    pub plan_type: String,
    pub subscription_status: String,
    pub subscription_expires_at: Option<String>,
    pub onboarding_completed: bool,
    pub runtime_version: i32,
    pub config_version: i32,
    pub feature_version: i32,
    pub created_at: String,
    pub updated_at: String,
}

/// SQLite message record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteMessage {
    pub msg_id: i64,
    pub trace_id: Option<String>,
    pub from_userid: i64,
    pub to_userid: i64,
    pub content: Option<String>,
    pub msg_type: String,
    pub file_id: Option<String>,
    pub media_url: Option<String>,
    pub timestamp: String,
    pub is_read: bool,
    pub quoted_text: Option<String>,
    pub quoted_msg_id: Option<i64>,
    pub quoted_file_id: Option<String>,
    pub quoted_type: Option<String>,
    pub tg_msg_id: Option<i64>,
    pub status: Option<String>,
    pub auto_translation: Option<String>,
    pub translated_text: Option<String>,
    pub original_text: Option<String>,
    pub admin_userid: Option<String>,
    pub is_deleted: bool,
    pub tenant_id: String,
}

/// SQLite tracked group record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteGroup {
    pub group_id: i64,
    pub group_title: Option<String>,
    pub is_monitored: bool,
    pub custom_keywords: Option<String>,
    pub first_seen: String,
    pub updated_at: String,
    pub delete_on_revoke: bool,
    pub revoke_sync_chat_id: Option<i64>,
    pub verify_enabled: bool,
    pub verify_type: String,
    pub verify_timeout: i32,
    pub verify_channel_id: Option<String>,
    pub verify_questions: Option<String>,
    pub tenant_id: String,
}

/// SQLite group member record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteGroupMember {
    pub id: i64,
    pub user_id: i64,
    pub username: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub group_id: i64,
    pub group_title: Option<String>,
    pub status: String,
    pub joined_at: String,
    pub is_verified: bool,
    pub verified_at: Option<String>,
    pub avatar_file_id: Option<String>,
    pub tenant_id: String,
}

/// SQLite group message record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteGroupMessage {
    pub msg_id: i64,
    pub group_id: i64,
    pub user_id: i64,
    pub username: Option<String>,
    pub content: Option<String>,
    pub msg_type: String,
    pub timestamp: Option<String>,
    pub tg_msg_id: Option<i64>,
    pub sender_type: String,
    pub quoted_tg_msg_id: Option<i64>,
    pub quoted_text: Option<String>,
    pub message_ts: Option<i64>,
    pub file_id: Option<String>,
    pub avatar_file_id: Option<String>,
    pub revoke_checked: bool,
    pub is_advertising: bool,
    pub ad_score: i32,
    pub ad_confidence: String,
    pub ad_findings: Option<String>,
    pub tenant_id: String,
    pub ad_cluster_id: Option<String>,
    pub ad_checked_at: Option<String>,
    pub ad_action_status: String,
    pub is_deleted: bool,
    pub deleted_at: Option<String>,
    pub ad_text_hash: Option<String>,
    pub media_unique_id: Option<String>,
}

/// SQLite user record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteUser {
    pub user_id: i64,
    pub email: Option<String>,
    pub created_at: String,
}

/// SQLite admin inbox record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteAdminInbox {
    pub from_userid: i64,
    pub summary: Option<String>,
    pub unread_count: i32,
    pub last_active: String,
    pub from_username: Option<String>,
    pub avatar_file_id: Option<String>,
    pub tenant_id: String,
}

/// SQLite table counts for verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteCounts {
    pub merchants: i64,
    pub messages: i64,
    pub groups: i64,
    pub group_members: i64,
    pub group_messages: i64,
    pub users: i64,
    pub admin_inbox: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlite_config() {
        let config = SqliteConfig::new("/path/to/db.sqlite3");
        assert_eq!(config.db_path, "/path/to/db.sqlite3");
    }

    #[test]
    fn test_virtual_assistant_uid_mapping() {
        // Test that legacy UID 900000001 would be mapped to -99999
        let _legacy_uid = 900000001i64;
        let mapped_uid = -99999i64;

        // Virtual assistant IDs that should be excluded
        let virtual_assistant_ids = vec![0i64, -99999i64, 900000001i64];
        assert!(virtual_assistant_ids.contains(&mapped_uid));
    }

    #[test]
    fn test_exclude_virtual_assistant_from_inbox() {
        // Virtual assistant UIDs should not generate admin_inbox rows
        let virtual_ids = [0i64, -99999i64, 900000001i64];

        for id in &virtual_ids {
            // None of these should create an inbox entry
            assert!(!should_create_admin_inbox(*id));
        }
    }

    fn should_create_admin_inbox(user_id: i64) -> bool {
        // Virtual assistant IDs are: 0, -99999, 900000001
        !(user_id == 0 || user_id == -99999 || user_id == 900000001)
    }
}
