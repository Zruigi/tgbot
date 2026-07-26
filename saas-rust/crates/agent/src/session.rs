// Agent Session Layer — in-process TTL-aware session store
// 对应 Python: web_admin.py _ai_agent_sessions dict + session management logic

use crate::schema::{AgentContext, AgentResponse};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{interval, Duration};

/// Session 过期时间（秒）— 默认 6 小时（Python 中为 6*3600 秒）
const SESSION_TTL_SECS: i64 = 6 * 3600;

/// Session 过期清理间隔（秒）
const CLEANUP_INTERVAL_SECS: u64 = 300; // 5 分钟检查一次

/// Agent Session 结构体 — 包含上下文、待确认操作和时间戳
#[derive(Debug, Clone)]
pub struct AgentSession {
    pub ctx: AgentContext,
    pub pending: Option<AgentResponse>,
    pub updated_at: i64, // Unix timestamp in seconds
}

impl AgentSession {
    /// 创建新的会话
    pub fn new(ctx: AgentContext) -> Self {
        Self {
            ctx,
            pending: None,
            updated_at: current_timestamp(),
        }
    }

    /// 生成会话 key
    pub fn session_key(tenant_id: &str, user_id: i64, merchant_id: Option<&str>) -> String {
        format!(
            "{}:{}:{}",
            tenant_id,
            user_id,
            merchant_id.unwrap_or("")
        )
    }

    /// 更新会话时间戳
    pub fn update_timestamp(&mut self) {
        self.updated_at = current_timestamp();
    }

    /// 检查会话是否已过期（基于 TTL）
    pub fn is_expired(&self) -> bool {
        let now = current_timestamp();
        now - self.updated_at > SESSION_TTL_SECS
    }

    /// 清空待确认操作
    pub fn clear_pending(&mut self) {
        self.pending = None;
    }

    /// 设置待确认操作
    pub fn set_pending(&mut self, response: AgentResponse) {
        self.pending = Some(response);
        self.update_timestamp();
    }
}

/// Agent Session Store — 线程安全的会话存储
pub struct AgentSessionStore {
    sessions: Arc<Mutex<HashMap<String, AgentSession>>>,
}

impl AgentSessionStore {
    /// 创建新的 session store 并启动后台清理任务
    pub fn new() -> Self {
        let store = Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        };

        // 启动后台清理任务
        let sessions_clone = Arc::clone(&store.sessions);
        tokio::spawn(async move {
            let mut cleanup_interval = interval(Duration::from_secs(CLEANUP_INTERVAL_SECS));
            loop {
                cleanup_interval.tick().await;

                let mut sessions = sessions_clone.lock().unwrap();
                let now = current_timestamp();

                // 清理过期会话
                sessions.retain(|_, session| {
                    now - session.updated_at <= SESSION_TTL_SECS
                });
            }
        });

        store
    }

    /// 生成会话 key
    pub fn session_key(tenant_id: &str, user_id: i64, merchant_id: Option<&str>) -> String {
        format!(
            "{}:{}:{}",
            tenant_id,
            user_id,
            merchant_id.unwrap_or("")
        )
    }

    /// 获取或创建会话
    pub fn get_or_create(&self, key: String, ctx: AgentContext) -> AgentSession {
        let mut sessions = self.sessions.lock().unwrap();
        sessions
            .entry(key)
            .or_insert_with(|| AgentSession::new(ctx))
            .clone()
    }

    /// 获取现有会话
    pub fn get(&self, key: &str) -> Option<AgentSession> {
        let sessions = self.sessions.lock().unwrap();
        sessions.get(key).cloned()
    }

    /// 更新会话（包含 pending 和 metadata）
    pub fn update(&self, key: &str, session: AgentSession) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.insert(key.to_string(), session);
    }

    /// 清理单个会话的 pending
    pub fn clear_pending(&self, key: &str) {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get_mut(key) {
            session.clear_pending();
            session.update_timestamp();
        }
    }

    /// 清理所有会话
    #[allow(dead_code)]
    pub fn clear_all(&self) {
        self.sessions.lock().unwrap().clear();
    }

    /// 获取当前活跃会话数（用于监控）
    #[allow(dead_code)]
    pub fn active_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }
}

impl Default for AgentSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

/// 获取当前时间戳（秒）
fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// ---------------------------------------------------------------------------
// Global Session Store Singleton
// ---------------------------------------------------------------------------

static GLOBAL_SESSION_STORE: OnceLock<Arc<AgentSessionStore>> = OnceLock::new();

/// Get the global agent session store (lazy initialized)
pub fn get_session_store() -> Arc<AgentSessionStore> {
    Arc::clone(
        GLOBAL_SESSION_STORE.get_or_init(|| Arc::new(AgentSessionStore::new()))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_key_generation() {
        let key1 = AgentSessionStore::session_key("tenant1", 123, Some("merchant1"));
        assert_eq!(key1, "tenant1:123:merchant1");

        let key2 = AgentSessionStore::session_key("tenant1", 123, None);
        assert_eq!(key2, "tenant1:123:");
    }

    #[test]
    fn test_session_expiry() {
        let session = AgentSession {
            ctx: AgentContext::new(123),
            pending: None,
            updated_at: current_timestamp() - SESSION_TTL_SECS - 1,
        };
        assert!(session.is_expired());
    }

    #[test]
    fn test_session_not_expired() {
        let session = AgentSession {
            ctx: AgentContext::new(123),
            pending: None,
            updated_at: current_timestamp(),
        };
        assert!(!session.is_expired());
    }
}
