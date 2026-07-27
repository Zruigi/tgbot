// OCR Tool - 调用 PaddleOCR HTTP API 进行文字识别
// 对应 Python: services/ai_agent/tools.py 中的 OCR 功能

use crate::schema::{AgentContext, Tool, ToolExecuteResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tgbot_integrations::paddleocr::{OcrRequest, PaddleOCRClient, PaddleOCRConfig};
use tracing::{error, info};

/// OCR 工具 - 图像文字识别
pub struct OcrTool {
    client: Arc<PaddleOCRClient>,
}

impl OcrTool {
    /// 创建新的 OCR 工具实例
    pub fn new(client: Arc<PaddleOCRClient>) -> Self {
        Self { client }
    }

    /// 使用默认配置创建
    pub fn with_default_config() -> Result<Self, anyhow::Error> {
        let config = PaddleOCRConfig::from_env()?;
        let client = Arc::new(PaddleOCRClient::new(config)?);
        Ok(Self::new(client))
    }
}

#[async_trait]
impl Tool for OcrTool {
    fn name(&self) -> &str {
        "ocr"
    }

    fn description(&self) -> &str {
        "对图片进行 OCR 文字识别。支持从图片URL、文件ID或base64数据识别文字。"
    }

    fn category(&self) -> &str {
        "system"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "properties": {
                "source_type": {
                    "type": "string",
                    "enum": ["url", "file_id", "base64"],
                    "description": "图片来源类型：url=网络图片URL, file_id=Telegram文件ID, base64=Base64编码图片"
                },
                "source": {
                    "type": "string",
                    "description": "图片来源数据（URL/文件ID/Base64字符串）"
                },
                "lang": {
                    "type": "string",
                    "description": "识别语言：ch=中文, en=英文, japan=日语, korean=韩语，默认ch"
                }
            },
            "required": ["source_type", "source"]
        })
    }

    async fn execute(&self, params: &Value, _context: &AgentContext) -> ToolExecuteResult {
        let source_type = params
            .get("source_type")
            .and_then(|v| v.as_str())
            .unwrap_or("url");
        let source = params.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let lang = params.get("lang").and_then(|v| v.as_str()).unwrap_or("ch");

        if source.is_empty() {
            return ToolExecuteResult::fail("图片 source 不能为空");
        }

        info!("OCR 识别请求: source_type={}, lang={}", source_type, lang);

        let ocr_result = match source_type {
            "url" => {
                // 直接传递 URL 给 OCR 服务（服务会自动下载）
                // 构建 PaddleOCR 请求
                let req = OcrRequest {
                    image: source.to_string(),
                    lang: Some(lang.to_string()),
                    use_angle_cls: Some(true),
                    use_gpu: None,
                };
                self.client.ocr(&req).await
            }
            "base64" => {
                let req = OcrRequest {
                    image: source.to_string(),
                    lang: Some(lang.to_string()),
                    use_angle_cls: Some(true),
                    use_gpu: None,
                };
                self.client.ocr(&req).await
            }
            "file_id" => {
                // 文件ID需要先下载，这里先返回提示
                return ToolExecuteResult::fail(
                    "file_id 类型的 OCR 暂不支持，请使用 URL 或 Base64 方式",
                );
            }
            _ => {
                return ToolExecuteResult::fail(&format!("未知的 source_type: {}", source_type));
            }
        };

        match ocr_result {
            Ok(resp) => {
                let texts: Vec<Value> = resp
                    .result
                    .unwrap_or_default()
                    .iter()
                    .map(|line| {
                        serde_json::json!({
                            "text": line.text,
                            "confidence": line.confidence.unwrap_or(0.0),
                            "bbox": line.text_region
                        })
                    })
                    .collect();

                let full_text = texts
                    .iter()
                    .map(|t| t.get("text").and_then(|v| v.as_str()).unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("\n");

                let message = if texts.is_empty() {
                    "未识别到文字".to_string()
                } else {
                    format!("识别到 {} 行文字", texts.len())
                };

                ToolExecuteResult::ok_with_data(
                    serde_json::json!({
                        "texts": texts,
                        "full_text": full_text,
                        "count": texts.len()
                    }),
                    &message,
                )
            }
            Err(e) => {
                error!("OCR 识别失败: {}", e);
                ToolExecuteResult::fail(&format!("OCR 识别失败: {}", e))
            }
        }
    }
}
