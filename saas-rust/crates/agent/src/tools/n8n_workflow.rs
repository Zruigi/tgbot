// n8n Workflow Tool - 触发 n8n 工作流
// 对应 Python: services/ai_agent/tools.py 中的 n8n 集成功能

use crate::schema::{AgentContext, ToolExecuteResult};
use crate::tools::Tool;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tgbot_integrations::n8n::{N8NClient, N8NConfig, WebhookTriggerRequest};
use tracing::{error, info};

/// n8n 工作流触发工具
pub struct N8NWorkflowTool {
    client: Arc<N8NClient>,
}

impl N8NWorkflowTool {
    /// 创建新的 n8n 工作流工具实例
    pub fn new(client: Arc<N8NClient>) -> Self {
        Self { client }
    }

    /// 使用默认配置创建
    pub fn with_default_config() -> Result<Self, anyhow::Error> {
        let config = N8NConfig::from_env()?;
        let client = Arc::new(N8NClient::new(config)?);
        Ok(Self::new(client))
    }
}

#[async_trait]
impl Tool for N8NWorkflowTool {
    fn name(&self) -> &str {
        "n8n_workflow"
    }

    fn description(&self) -> &str {
        "触发 n8n 工作流。通过 webhook 路径触发指定的工作流，可传递自定义参数。"
    }

    fn category(&self) -> &str {
        "system"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "properties": {
                "workflow_path": {
                    "type": "string",
                    "description": "n8n webhook 路径（webhook URL 中的路径部分）"
                },
                "payload": {
                    "type": "object",
                    "description": "传递给工作流的 JSON 数据"
                },
                "test_mode": {
                    "type": "boolean",
                    "description": "是否使用测试 webhook（webhook-test），默认 false"
                }
            },
            "required": ["workflow_path", "payload"]
        })
    }

    async fn execute(&self, params: &Value, _context: &AgentContext) -> ToolExecuteResult {
        let workflow_path = params
            .get("workflow_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let payload = params.get("payload").cloned().unwrap_or(serde_json::json!({}));
        let test_mode = params
            .get("test_mode")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if workflow_path.is_empty() {
            return ToolExecuteResult::fail("workflow_path 不能为空");
        }

        info!(
            "触发 n8n 工作流: path={}, test_mode={}",
            workflow_path, test_mode
        );

        let request = WebhookTriggerRequest {
            path: workflow_path.to_string(),
            test: test_mode,
            body: payload,
        };

        match self.client.trigger_webhook(&request).await {
            Ok(response) => {
                // 将响应数据转换为适合展示的格式
                let data_value = &response.data;
                let message = "工作流触发成功".to_string();

                ToolExecuteResult::ok_with_data(data_value.clone(), &message)
            }
            Err(e) => {
                error!("n8n 工作流触发失败: {}", e);
                ToolExecuteResult::fail(&format!("工作流触发失败: {}", e))
            }
        }
    }
}