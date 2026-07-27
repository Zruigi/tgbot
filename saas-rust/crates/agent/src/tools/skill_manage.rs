// Skill Manage Tool - 管理 AI 技能
// 对应 Python: services/ai_agent/tools.py 中的 SkillManageTool

use crate::schema::{AgentContext, Tool, ToolExecuteResult};
use async_trait::async_trait;
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info, warn};

/// AI 技能管理工具
pub struct SkillManageTool {
    pool: Arc<PgPool>,
    project_root: PathBuf,
}

impl SkillManageTool {
    /// 创建新的技能管理工具实例
    pub fn new(pool: Arc<PgPool>, project_root: PathBuf) -> Self {
        Self { pool, project_root }
    }
}

#[async_trait]
impl Tool for SkillManageTool {
    fn name(&self) -> &str {
        "skill_manage"
    }

    fn description(&self) -> &str {
        "管理 AI 助手技能。支持：列出技能、新增技能、更新技能、启用/停用技能、导入本地技能模板。"
    }

    fn category(&self) -> &str {
        "system"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "add", "update", "enable", "disable", "import_builtin"],
                    "description": "操作类型：list=列出技能, add=新增技能, update=更新技能, enable=启用技能, disable=停用技能, import_builtin=导入本地技能"
                },
                "name": {
                    "type": "string",
                    "description": "技能名称"
                },
                "description": {
                    "type": "string",
                    "description": "技能描述"
                },
                "prompt_template": {
                    "type": "string",
                    "description": "技能提示词模板"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: &Value, context: &AgentContext) -> ToolExecuteResult {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list")
            .to_lowercase();
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let description = params
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let prompt_template = params
            .get("prompt_template")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let tenant_id = context
            .metadata
            .get("tenant_id")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        info!("技能管理操作: action={}", action);

        let result = match action.as_str() {
            "list" => self.list_skills(tenant_id).await,
            "add" => {
                if name.is_empty() {
                    return ToolExecuteResult::fail("新增技能需要提供 name 参数");
                }
                self.add_or_update_skill(
                    &name,
                    &description,
                    prompt_template,
                    "custom",
                    true,
                    tenant_id,
                )
                .await
            }
            "update" => {
                if name.is_empty() {
                    return ToolExecuteResult::fail("更新技能需要提供 name 参数");
                }
                self.add_or_update_skill(
                    &name,
                    &description,
                    prompt_template,
                    "custom",
                    true,
                    tenant_id,
                )
                .await
            }
            "enable" => {
                if name.is_empty() {
                    return ToolExecuteResult::fail("启用技能需要提供 name 参数");
                }
                self.set_skill_enabled(&name, true, tenant_id).await
            }
            "disable" => {
                if name.is_empty() {
                    return ToolExecuteResult::fail("停用技能需要提供 name 参数");
                }
                self.set_skill_enabled(&name, false, tenant_id).await
            }
            "import_builtin" => self.import_builtin_skills(tenant_id).await,
            _ => Err(anyhow::anyhow!("未知操作类型: {}", action)),
        };

        match result {
            Ok(data) => ToolExecuteResult::ok_with_data(data.0, &data.1),
            Err(e) => {
                error!("技能管理失败: {}", e);
                ToolExecuteResult::fail(&format!("技能管理错误: {}", e))
            }
        }
    }
}

impl SkillManageTool {
    /// 列出所有技能（不包括内置文件系统技能）
    async fn list_skills(&self, tenant_id: &str) -> Result<(Value, String), anyhow::Error> {
        let rows = sqlx::query(
            "SELECT id, name, description, prompt_template, source, is_enabled, created_at, updated_at \
             FROM ai_custom_skills \
             WHERE tenant_id = $1 AND (source IS NULL OR source != 'builtin_fs') \
             ORDER BY name \
             LIMIT 50",
        )
        .bind(tenant_id)
        .fetch_all(&*self.pool)
        .await?;

        let skills: Vec<Value> = rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "id": row.try_get::<i64, _>("id").unwrap_or(0),
                    "name": row.try_get::<String, _>("name").unwrap_or_default(),
                    "description": row.try_get::<Option<String>, _>("description").unwrap_or_default(),
                    "prompt_template": row.try_get::<Option<String>, _>("prompt_template").unwrap_or_default(),
                    "source": row.try_get::<Option<String>, _>("source").unwrap_or_default(),
                    "is_enabled": row.try_get::<Option<bool>, _>("is_enabled").unwrap_or(Some(true)),
                    "created_at": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("created_at").unwrap_or(None),
                    "updated_at": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("updated_at").unwrap_or(None),
                })
            })
            .collect();

        Ok((
            serde_json::json!(skills),
            format!("共 {} 个技能", skills.len()),
        ))
    }

    /// 新增或更新技能
    async fn add_or_update_skill(
        &self,
        name: &str,
        description: &str,
        prompt_template: &str,
        source: &str,
        is_enabled: bool,
        tenant_id: &str,
    ) -> Result<(Value, String), anyhow::Error> {
        let now = chrono::Utc::now();

        sqlx::query(
            r#"
            INSERT INTO ai_custom_skills (name, description, prompt_template, source, is_enabled, tenant_id, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (name, tenant_id) DO UPDATE SET
                description = EXCLUDED.description,
                prompt_template = EXCLUDED.prompt_template,
                is_enabled = EXCLUDED.is_enabled,
                updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(name)
        .bind(description)
        .bind(prompt_template)
        .bind(source)
        .bind(is_enabled)
        .bind(tenant_id)
        .bind(now)
        .bind(now)
        .execute(&*self.pool)
        .await?;

        Ok((
            serde_json::json!({ "name": name }),
            format!(
                "技能已{}: {}",
                if source == "custom" {
                    "新增"
                } else {
                    "更新"
                },
                name
            ),
        ))
    }

    /// 设置技能启用/停用状态
    async fn set_skill_enabled(
        &self,
        name: &str,
        is_enabled: bool,
        tenant_id: &str,
    ) -> Result<(Value, String), anyhow::Error> {
        let now = chrono::Utc::now();

        let result = sqlx::query(
            "UPDATE ai_custom_skills SET is_enabled = $1, updated_at = $2 \
             WHERE name = $3 AND tenant_id = $4",
        )
        .bind(is_enabled)
        .bind(now)
        .bind(name)
        .bind(tenant_id)
        .execute(&*self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(anyhow::anyhow!("技能不存在: {}", name));
        }

        Ok((
            serde_json::json!({ "name": name, "is_enabled": is_enabled }),
            format!(
                "技能已{}: {}",
                if is_enabled { "启用" } else { "停用" },
                name
            ),
        ))
    }

    /// 扫描并导入本地技能模板
    async fn import_builtin_skills(
        &self,
        tenant_id: &str,
    ) -> Result<(Value, String), anyhow::Error> {
        // 获取技能目录
        let skill_dirs = self.get_skill_dirs();

        let mut imported = 0;
        let mut detected = 0;

        for base_dir in skill_dirs {
            if !base_dir.is_dir() {
                warn!("技能目录不存在: {:?}", base_dir);
                continue;
            }

            // 遍历目录
            match std::fs::read_dir(&base_dir) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if !path.is_dir() {
                            continue;
                        }

                        let skill_md = path.join("SKILL.md");
                        if !skill_md.is_file() {
                            continue;
                        }

                        detected += 1;

                        // 获取技能名称
                        let skill_name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .trim()
                            .to_string();

                        if skill_name.is_empty() {
                            continue;
                        }

                        // 读取 SKILL.md 获取描述
                        let skill_desc = self.read_skill_description(&skill_md);

                        // 导入数据库
                        let now = chrono::Utc::now();
                        let result = sqlx::query(
                            r#"
                            INSERT INTO ai_custom_skills (name, description, prompt_template, source, is_enabled, tenant_id, created_at, updated_at)
                            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                            ON CONFLICT (name, tenant_id) DO NOTHING
                            "#,
                        )
                        .bind(&skill_name)
                        .bind(skill_desc)
                        .bind(format!("引用本地技能: {}", skill_name))
                        .bind("builtin_fs")
                        .bind(true)
                        .bind(tenant_id)
                        .bind(now)
                        .bind(now)
                        .execute(&*self.pool)
                        .await;

                        if let Ok(r) = result {
                            if r.rows_affected() > 0 {
                                imported += 1;
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("读取技能目录失败: {:?}, error: {}", base_dir, e);
                }
            }
        }

        Ok((
            serde_json::json!({
                "imported": imported,
                "detected": detected
            }),
            format!("已导入 {}/{} 个本地技能", imported, detected),
        ))
    }

    /// 获取技能目录列表
    fn get_skill_dirs(&self) -> Vec<PathBuf> {
        // 从环境变量获取自定义技能目录
        let env_dirs = std::env::var("AI_SKILL_LOCAL_DIRS")
            .unwrap_or_default()
            .trim()
            .to_string();

        let mut dirs = Vec::new();

        if !env_dirs.is_empty() {
            for p in env_dirs.split(',') {
                let p = p.trim();
                if p.is_empty() {
                    continue;
                }
                let path = if PathBuf::from(p).is_absolute() {
                    PathBuf::from(p)
                } else {
                    self.project_root.join(p)
                };
                dirs.push(path);
            }
        }

        // 默认添加 skills 目录
        let default_dir = self.project_root.join("skills");
        if !dirs.iter().any(|d| d == &default_dir) {
            dirs.push(default_dir);
        }

        dirs
    }

    /// 读取 SKILL.md 文件获取描述
    fn read_skill_description(&self, path: &PathBuf) -> String {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                // 读取前 2000 字符，查找 Description: 行
                for line in content.lines() {
                    let line = line.trim();
                    if line.to_lowercase().starts_with("description:") {
                        return line
                            .splitn(2, ':')
                            .nth(1)
                            .map(|s| s.trim().trim_matches('"'))
                            .unwrap_or("")
                            .to_string();
                    }
                }
                "本地技能模板".to_string()
            }
            Err(_) => "本地技能模板".to_string(),
        }
    }
}
