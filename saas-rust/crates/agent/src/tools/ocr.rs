// OCR Tool - 调用 PaddleOCR HTTP API 进行文字识别
// 对应 Python: services/ai_agent/tools.py 中的 OCR 功能

use crate::schema::{AgentContext, Tool, ToolExecuteResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tgbot_integrations::paddleocr::{OcrRequest, PaddleOCRClient, PaddleOCRConfig};
use tgbot_integrations::telegram::{GetFileParams, TelegramClient};
use tracing::{error, info};

const MAX_TELEGRAM_OCR_BYTES: usize = 10 * 1024 * 1024;

/// OCR 工具 - 图像文字识别
pub struct OcrTool {
    client: Arc<PaddleOCRClient>,
    telegram_client: Option<Arc<TelegramClient>>,
}

impl OcrTool {
    /// 创建新的 OCR 工具实例
    pub fn new(client: Arc<PaddleOCRClient>) -> Self {
        Self {
            client,
            telegram_client: None,
        }
    }

    pub fn with_telegram_client(mut self, client: Arc<TelegramClient>) -> Self {
        self.telegram_client = Some(client);
        self
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
                let Some(telegram) = self.telegram_client.as_ref() else {
                    return ToolExecuteResult::fail("Telegram 客户端未配置，无法下载 OCR 文件");
                };
                let file = match telegram
                    .get_file(GetFileParams {
                        file_id: source.to_string(),
                    })
                    .await
                {
                    Ok(file) => file,
                    Err(error) => {
                        return ToolExecuteResult::fail(&format!("Telegram 文件查询失败: {error}"));
                    }
                };
                let bytes = match telegram
                    .download_file_bytes(&file, MAX_TELEGRAM_OCR_BYTES)
                    .await
                {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        return ToolExecuteResult::fail(&format!("Telegram 文件下载失败: {error}"));
                    }
                };
                self.client.ocr_image_bytes(&bytes, Some(lang)).await
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Tool;

    #[tokio::test]
    async fn file_id_ocr_fails_closed_without_telegram_client() {
        let client = Arc::new(PaddleOCRClient::new(PaddleOCRConfig::default()).unwrap());
        let tool = OcrTool::new(client);
        let result = tool
            .execute(
                &serde_json::json!({
                    "source_type": "file_id",
                    "source": "beta-file",
                }),
                &crate::schema::AgentContext::new(1),
            )
            .await;
        assert!(!result.success);
        assert!(result.message.unwrap().contains("Telegram 客户端未配置"));
    }
}
