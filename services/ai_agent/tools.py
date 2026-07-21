# -*- coding: utf-8 -*-
"""
AI Agent 内置工具集 — 每个工具封装一个系统功能
工具通过 Function Calling 机制被 LLM 调用
"""

import logging
import os
import json
import time
from datetime import datetime, timezone
from typing import Any, Dict, List, Optional
import httpx
import database
import config
from .ad_detection import detect_advertising_signals

logger = logging.getLogger(__name__)
_PROJECT_ROOT = os.path.realpath(os.path.join(os.path.dirname(__file__), "..", ".."))


def _context_metadata(context: Any) -> Dict[str, Any]:
    metadata = getattr(context, "metadata", None)
    return metadata if isinstance(metadata, dict) else {}


def _context_scoped_tenant_id(context: Any) -> Optional[str]:
    """Return tenant_id only when the agent is running in merchant scope."""
    metadata = _context_metadata(context)
    role = str(metadata.get("role") or "").strip()
    tenant_id = str(metadata.get("tenant_id") or "").strip()
    if role == "merchant_owner" and tenant_id and tenant_id != "platform":
        return tenant_id
    return None


def _context_merchant_id(context: Any) -> Optional[int]:
    merchant_id = _context_metadata(context).get("merchant_id")
    try:
        return int(merchant_id) if merchant_id is not None and str(merchant_id).strip() else None
    except Exception:
        return None


async def _resolve_scope_bot_id(
    db: Any,
    *,
    tenant_id: Optional[str],
    merchant_id: Optional[int],
) -> int:
    if merchant_id is not None:
        cursor = await db.execute("SELECT bot_id FROM merchants WHERE id = ? LIMIT 1", (int(merchant_id),))
        row = await cursor.fetchone()
        if row and row[0]:
            return int(row[0])

    if tenant_id:
        cursor = await db.execute("SELECT bot_id FROM merchants WHERE tenant_id = ? LIMIT 1", (tenant_id,))
        row = await cursor.fetchone()
        if row and row[0]:
            return int(row[0])

    return int(config.ADMIN_ID)


def _is_path_inside_project(path: str) -> bool:
    try:
        proj = os.path.realpath(_PROJECT_ROOT)
        target = os.path.realpath(path)
        return os.path.commonpath([proj, target]) == proj
    except Exception:
        return False


def _get_project_skill_dirs() -> List[str]:
    raw = (os.getenv("AI_SKILL_LOCAL_DIRS") or "").strip()
    candidates = [p.strip() for p in raw.split(",") if p.strip()] if raw else ["skills"]
    out: List[str] = []
    seen = set()
    for p in candidates:
        abs_path = p if os.path.isabs(p) else os.path.join(_PROJECT_ROOT, p)
        abs_path = os.path.realpath(abs_path)
        if not _is_path_inside_project(abs_path):
            logger.warning("[AI Skill] 忽略项目外技能目录: %s", p)
            continue
        if abs_path in seen:
            continue
        seen.add(abs_path)
        out.append(abs_path)
    return out


# ===== 基类 =====

class BaseTool:
    """所有 Agent 工具的基类"""
    
    name: str = ""
    description: str = ""
    category: str = "system"
    
    @property
    def parameters(self) -> dict:
        """JSON Schema 参数定义（子类覆写）"""
        return {"properties": {}, "required": []}
    
    @property
    def schema_for_llm(self) -> dict:
        """转换为 LLM 可用的 function calling schema"""
        return {
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": {
                    "type": "object",
                    "properties": self.parameters.get("properties", {}),
                    "required": self.parameters.get("required", []),
                }
            }
        }
    
    async def execute(self, params: Dict[str, Any], context=None) -> dict:
        """
        执行工具逻辑，返回结构化结果
        子类必须实现此方法
        
        返回格式: {
            "success": bool,
            "data": Any,
            "message": str (可选),
            "action_taken": bool (可选),
            "needs_confirmation": bool (可选),
        }
        """
        raise NotImplementedError


# ===== 工具 1：财务系统 =====

class FinanceTool(BaseTool):
    """财务数据查询与整理工具"""
    name = "finance_query"
    description = (
        "查询和整理财务数据。支持：获取财务摘要、查看未核销流水、"
        "获取科目树、生成报表摘要、测试 TRC20/BSC 链上 API。"
        "可以自动整理财务数据并给出分析建议。"
    )
    category = "finance"
    
    @property
    def parameters(self) -> dict:
        return {
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["summary", "unreconciled", "categories", "analysis", "onchain_api_test"],
                    "description": "操作类型：summary=财务摘要, unreconciled=未核销流水, categories=科目树, analysis=综合分析, onchain_api_test=链上API测试"
                },
                "month": {
                    "type": "string",
                    "description": "月份筛选（如 '2026-04'），不填则查全部"
                },
            },
            "required": ["action"]
        }

    async def execute(self, params: Dict[str, Any], context=None) -> dict:
        action = params.get("action", "summary")
        
        try:
            if action == "summary":
                summary = await database.get_finance_summary_by_currency(config.ADMIN_ID)
                rows = summary.get("by_currency", []) if isinstance(summary, dict) else []
                return {
                    "success": True,
                    "data": summary,
                    "message": f"已查询到 {len(rows)} 个币种的财务摘要"
                }
            
            elif action == "unreconciled":
                rows = await database.get_finance_unreconciled_with_fx(
                    user_id=config.ADMIN_ID,
                    monitor_id=params.get("monitor_id"),
                )
                return {
                    "success": True,
                    "data": rows[:50],  # 限制返回量
                    "count": len(rows),
                    "message": f"共 {len(rows)} 条未核销记录"
                }
            
            elif action == "categories":
                tree = await database.get_finance_category_tree()
                return {
                    "success": True,
                    "data": tree,
                    "message": f"共 {len(tree)} 个科目分类"
                }
                
            elif action == "analysis":
                # 综合分析：汇总 + 未核销 + 建议
                summary = await database.get_finance_summary_by_currency(config.ADMIN_ID)
                unreconciled = await database.get_finance_unreconciled_with_fx(
                    user_id=config.ADMIN_ID
                )
                
                analysis_text = self._build_analysis_text(summary, unreconciled)
                return {
                    "success": True,
                    "data": {
                        "summary": summary,
                        "unreconciled_count": len(unreconciled),
                        "total_unreconciled_value": sum(
                            float(r.get('base_fiat_value', 0) or 0) for r in unreconciled
                        ),
                    },
                    "analysis_text": analysis_text,
                    "message": "综合分析已完成"
                }

            elif action == "onchain_api_test":
                test_result = await self._run_onchain_api_test()
                return {
                    "success": True,
                    "data": test_result,
                    "message": test_result.get("summary", "链上 API 测试完成")
                }
            else:
                return {"success": False, "message": f"未知操作: {action}"}
                
        except Exception as e:
            logger.error(f"FinanceTool 执行失败: {e}")
            return {"success": False, "message": f"财务系统错误: {e}"}

    def _build_analysis_text(self, summary, unreconciled) -> str:
        """构建分析文本供 LLM 参考"""
        lines = ["📊 财务分析报告\n" + "="*30]

        summary_rows = summary.get("by_currency", []) if isinstance(summary, dict) else (summary or [])
        for s in summary_rows:
            currency = s.get('currency', '?')
            total_in = float(s.get('total_income', 0) or 0)
            total_out = float(s.get('total_expense', 0) or 0)
            balance = float(s.get('balance', 0) or 0)
            lines.append(f"\n💰 {currency}: 收入={total_in:.2f} | 支出={total_out:.2f} | 净额={balance:.2f}")
        
        total_unreconciled = sum(float(r.get('base_fiat_value', 0) or 0) for r in unreconciled)
        lines.append(f"\n⚠️ 待核销金额: ${total_unreconciled:,.2f} ({len(unreconciled)}笔)")
        
        if len(unreconciled) > 10:
            lines.append("📌 建议尽快处理待核销流水，避免积压影响账目准确性")
        
        return "\n".join(lines)

    async def _get_secret_with_aliases(
        self,
        alias_keys: List[str],
        fallback: str = "",
    ) -> Dict[str, str]:
        """
        按优先级获取密钥:
        1) tenant_configs (SecretManager)
        2) .env 变量
        3) config.py 中已解析后的 fallback
        """
        try:
            from services.secret_manager import SecretManager
            for key in alias_keys:
                val = await SecretManager.get_config(key, default=None)
                if isinstance(val, str) and val.strip():
                    return {"value": val.strip(), "source": f"tenant:{key}"}
        except Exception:
            pass

        for key in alias_keys:
            val = os.getenv(key, "").strip()
            if val:
                return {"value": val, "source": f"env:{key}"}

        if isinstance(fallback, str) and fallback.strip():
            return {"value": fallback.strip(), "source": "config:fallback"}

        return {"value": "", "source": "none"}

    async def _run_onchain_api_test(self) -> Dict[str, Any]:
        """测试 TRC20 / BSC API 连通性和凭证有效性（不回显敏感 key）。"""
        trc = await self._get_secret_with_aliases(
            ["TRONGRID_API_KEY", "DMV", "DMV_API_KEY", "DMV_API_TOKEN", "DMV_TOKEN"],
            fallback=config.TRONGRID_API_KEY,
        )
        bsc = await self._get_secret_with_aliases(
            ["BSCSCAN_API_KEY", "BC", "BC_API_KEY", "BC_API_TOKEN", "BC_TOKEN"],
            fallback=config.BSCSCAN_API_KEY,
        )

        result: Dict[str, Any] = {
            "trc20": {
                "configured": bool(trc["value"]),
                "source": trc["source"],
                "ok": False,
                "detail": "未配置 TRC20 API Key",
            },
            "bsc": {
                "configured": bool(bsc["value"]),
                "source": bsc["source"],
                "ok": False,
                "detail": "未配置 BSC API Key",
            },
        }

        async with httpx.AsyncClient(timeout=12.0) as client:
            if trc["value"]:
                try:
                    trc_url = (
                        "https://api.trongrid.io/v1/contracts/"
                        "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t/events?limit=1"
                    )
                    trc_resp = await client.get(
                        trc_url,
                        headers={"TRON-PRO-API-KEY": trc["value"]},
                    )
                    trc_json = trc_resp.json() if trc_resp.headers.get("content-type", "").startswith("application/json") else {}
                    trc_ok = trc_resp.status_code == 200 and bool(trc_json.get("success", True))
                    result["trc20"].update(
                        {
                            "ok": trc_ok,
                            "http_status": trc_resp.status_code,
                            "detail": "TronGrid 连通正常" if trc_ok else str(trc_json.get("error") or trc_json.get("message") or f"HTTP {trc_resp.status_code}"),
                        }
                    )
                except Exception as e:
                    result["trc20"].update({"ok": False, "detail": f"请求异常: {e}"})

            if bsc["value"]:
                try:
                    bsc_url = (
                        "https://api.bscscan.com/api"
                        f"?module=proxy&action=eth_blockNumber&apikey={bsc['value']}"
                    )
                    bsc_resp = await client.get(bsc_url)
                    bsc_json = bsc_resp.json() if bsc_resp.headers.get("content-type", "").startswith("application/json") else {}
                    bsc_msg = str(bsc_json.get("message") or "")
                    bsc_result = bsc_json.get("result")
                    bsc_ok = bsc_resp.status_code == 200 and bool(bsc_result) and "invalid" not in bsc_msg.lower()
                    result["bsc"].update(
                        {
                            "ok": bsc_ok,
                            "http_status": bsc_resp.status_code,
                            "latest_block": bsc_result if bsc_ok else None,
                            "detail": "BscScan 连通正常" if bsc_ok else (bsc_msg or str(bsc_json)[:120] or f"HTTP {bsc_resp.status_code}"),
                        }
                    )
                except Exception as e:
                    result["bsc"].update({"ok": False, "detail": f"请求异常: {e}"})

        ok_count = int(bool(result["trc20"]["ok"])) + int(bool(result["bsc"]["ok"]))
        result["summary"] = f"链上 API 测试完成：{ok_count}/2 可用 (TRC20={result['trc20']['ok']}, BSC={result['bsc']['ok']})"
        return result


# ===== 工具 2：广播/群发 =====

class BroadcastTool(BaseTool):
    """消息群发与内容分发工具"""
    name = "broadcast_message"
    description = (
        "向用户或群组发送广播消息。支持纯文本、带图片、自定义按钮。"
        "可用于一键群发通知、营销文案推送、活动公告等。"
    )
    category = "broadcast"

    @property
    def parameters(self) -> dict:
        return {
            "properties": {
                "content": {
                    "type": "string",
                    "description": "要发送的消息正文（支持 HTML 格式）"
                },
                "target_type": {
                    "type": "string",
                    "enum": ["users", "groups", "all"],
                    "description": "目标类型：users=私聊用户, groups=群组频道, all=全部目标"
                },
                "target_ids": {
                    "type": "array",
                    "items": {"type": "integer"},
                    "description": "指定目标 ID 列表（可选，不传则使用全部）"
                },
                "include_image": {
                    "type": "boolean",
                    "description": "是否包含图片（如有上传的话）"
                },
                "do_translate": {
                    "type": "boolean",
                    "description": "是否自动翻译为多语言"
                },
                "buttons": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "text": {"type": "string"},
                            "url": {"type": "string"}
                        },
                        "required": ["text", "url"]
                    },
                    "description": "自定义跳转按钮列表"
                },
            },
            "required": ["content"]
        }

    async def execute(self, params: Dict[str, Any], context=None) -> dict:
        content = params.get("content", "")
        if not content.strip():
            return {"success": False, "message": "消息内容不能为空"}

        # 多级 Bot 实例获取策略
        bot = None
        # 1. 优先从 context 获取
        if context and hasattr(context, 'bot_instance') and context.bot_instance:
            bot = context.bot_instance
        # 2. 回退到 web_admin 全局变量
        if not bot:
            try:
                import web_admin
                bot = web_admin._bot_instance
            except Exception:
                pass
        # 3. 都失败则返回明确错误
        if not bot:
            return {
                "success": False, 
                "message": "Bot 实例未初始化，无法发送消息。请确保机器人已正常启动。",
                "action_taken": False,
            }
        
        # 确定目标列表
        target_type = params.get("target_type", "users")
        target_ids = params.get("target_ids")
        
        try:
            if target_type in ("users", "all") and not target_ids:
                # 获取所有来访用户
                users = await database.get_inbox_list()
                user_ids = [int(u['from_userid']) for u in users if u.get('from_userid')]
                # 排除 AI 助手自身
                user_ids = [uid for uid in user_ids if uid != getattr(config, 'AI_ASSISTANT_USER_ID', -1)]
            elif target_type in ("groups", "all") and not target_ids:
                # 获取群发目标
                targets = await database.get_broadcast_targets(enabled_only=True)
                chat_ids = [t['chat_id'] for t in targets]
                if target_type == "all":
                    # all 模式下同时发送给用户和群组
                    users = await database.get_inbox_list()
                    user_ids = [int(u['from_userid']) for u in users if u.get('from_userid')]
                    # 这里先只处理群发部分
                    user_ids = []
                else:
                    user_ids = []
                chat_ids = list(set(chat_ids))
            else:
                user_ids = target_ids or []
                chat_ids = []

            buttons = params.get("buttons") or []
            do_translate = bool(params.get("do_translate"))
            
            # 调用实际广播函数
            result = None
            if chat_ids:
                from web_admin import _send_broadcast_to_chats
                result = await _send_broadcast_to_chats(
                    chat_ids=chat_ids,
                    content=content,
                    image_file=params.get("image_bytes"),
                    do_translate=do_translate,
                    buttons=buttons,
                )
            elif user_ids:
                # 使用 web_admin.py 的用户广播逻辑（复用 api_broadcast_message 内部逻辑）
                try:
                    # 直接通过 bot 实例逐个发送给用户
                    sent_count = 0
                    failed_items = []
                    for uid in user_ids:
                        try:
                            if bot:
                                await bot.send_message(chat_id=uid, text=content)
                                sent_count += 1
                            else:
                                failed_items.append({"chat_id": uid, "reason": "Bot不可用"})
                        except Exception as ex:
                            failed_items.append({"chat_id": uid, "reason": str(ex)})
                    result = {
                        "total": len(user_ids),
                        "success": sent_count,
                        "failed": len(failed_items),
                        "failed_items": failed_items,
                    }
                except Exception as be:
                    return {"success": False, "message": f"用户群发错误: {be}"}
            
            if result:
                return {
                    "success": True,
                    "data": result,
                    "action_taken": True,
                    "needs_confirmation": True,  # 群发总是需要确认
                    "message": (
                        f"✅ 广播完成: 总计{result.get('total',0)}个目标, "
                        f"成功{result.get('success',0)}个, 失败{result.get('failed',0)}个"
                    ),
                }
            else:
                return {"success": False, "message": "没有有效的发送目标"}

        except Exception as e:
            logger.error(f"BroadcastTool 执行失败: {e}")
            return {"success": False, "message": f"广播执行错误: {e}"}


# ===== 工具 3：内容生成 =====

class ContentGeneratorTool(BaseTool):
    """文案生成工具 — 利用 LLM 生成各类文案"""
    name = "generate_content"
    description = (
        "生成各类文案内容。支持：营销文案、产品介绍、公告通知、社群话术、"
        "回复模板等。可指定风格、长度、目标受众。"
    )
    category = "content"

    @property
    def parameters(self) -> dict:
        return {
            "properties": {
                "topic": {
                    "type": "string",
                    "description": "文案主题或核心信息"
                },
                "style": {
                    "type": "string",
                    "enum": ["professional", "casual", "enthusiastic", "urgent", "friendly"],
                    "description": "文案风格：专业正式/轻松随意/热情洋溢/紧急通知/亲切友好"
                },
                "length": {
                    "type": "string",
                    "enum": ["short", "medium", "long"],
                    "description": "长度：短(100字内)/中(300字内)/长(500字内)"
                },
                "target_audience": {
                    "type": "string",
                    "description": "目标受众描述（如'新用户''VIP客户'）"
                },
                "purpose": {
                    "type": "string",
                    "enum": ["announcement", "marketing", "notification", "greeting", "reply_template"],
                    "description": "用途：公告/营销/通知/问候/回复模板"
                },
            },
            "required": ["topic"]
        }

    async def execute(self, params: Dict[str, Any], context=None) -> dict:
        topic = params.get("topic", "")
        style = params.get("style", "professional")
        length = params.get("length", "medium")
        purpose = params.get("purpose", "announcement")
        audience = params.get("target_audience", "通用用户")

        # 构造 prompt 让 LLM 生成文案
        style_map = {
            "professional": "专业正式的语气，用词精准，结构清晰",
            "casual": "轻松随意的口语化表达，像朋友聊天一样自然",
            "enthusiastic": "充满热情活力的语气，多用感叹号和表情符号",
            "urgent": "紧急重要感，强调时效性和行动必要性",
            "friendly": "亲切友好的语气，温暖且富有同理心",
        }
        length_map = {
            "short": "控制在100字以内",
            "medium": "控制在200-300字左右",
            "long": "控制在400-600字左右",
        }
        purpose_map = {
            "announcement": "官方公告/通知",
            "marketing": "产品推广/营销文案",
            "notification": "日常提醒/状态通知",
            "greeting": "欢迎语/开场白",
            "reply_template": "客服回复模板/FAQ 回答",
        }

        generate_prompt = f"""请生成一篇{purpose_map.get(purpose, '')}文案。

要求：
- 主题：{topic}
- 风格：{style_map.get(style, '')}
- 字数要求：{length_map.get(length, '')}
- 目标受众：{audience}

请直接输出文案内容，不需要额外说明。"""

        # 这个工具需要调用 LLM 来生成
        # 返回一个特殊的标记，告诉引擎需要先生成再返回
        return {
            "success": True,
            "data": {
                "generate_prompt": generate_prompt,
                "meta": {
                    "topic": topic,
                    "style": style,
                    "length": length,
                    "purpose": purpose,
                    "audience": audience,
                }
            },
            "requires_llm_generation": True,
            "message": f"正在生成{purpose_map.get(purpose,'')}文案...",
        }


# ===== 工具 4：广告检测 =====

class AdDetectorTool(BaseTool):
    """广告/垃圾消息识别工具"""
    name = "detect_advertising"
    description = (
        "检测文本中是否包含广告、垃圾信息、违规内容。支持：广告关键词匹配、"
        "可疑链接检测、重复内容判断、推广意图评分。"
    )
    category = "security"

    @property
    def parameters(self) -> dict:
        return {
            "properties": {
                "text": {
                    "type": "string",
                    "description": "需要检测的文本内容"
                },
                "context_type": {
                    "type": "string",
                    "enum": ["group_message", "private_message", "bio", "username"],
                    "description": "上下文类型：群消息/私聊/简介/用户名"
                },
            },
            "required": ["text"]
        }

    async def execute(self, params: Dict[str, Any], context=None) -> dict:
        text = params.get("text", "")
        context_type = params.get("context_type", "group_message")

        if not text.strip():
            return {"success": False, "message": "检测文本不能为空"}
        result = detect_advertising_signals(text=text, context_type=context_type)
        is_ad = bool(result.get("is_advertising"))
        ad_score = int(result.get("ad_score", 0) or 0)
        confidence = str(result.get("confidence") or "low")

        return {
            "success": True,
            "data": result,
            "message": (
                f"{'⚠️ 检测到广告内容' if is_ad else '✅ 未检出广告'} "
                f"(得分: {ad_score}/100, 置信度: {confidence})"
            ),
        }


# ===== 工具 4a：最近广告记录查询 =====

class QueryRecentAdMessagesTool(BaseTool):
    """查询最近时间窗内可安全处理的广告消息快照。"""
    name = "query_recent_ad_messages"
    description = (
        "查询最近24小时内已落库、已标记为广告的用户消息，返回精确消息引用、"
        "最早/最晚记录时间以及随机时间样本。默认排除管理员、bot 和频道转发。"
    )
    category = "security"

    @property
    def parameters(self) -> dict:
        return {
            "properties": {
                "hours": {"type": "integer", "description": "查询窗口，1-24 小时，默认24"},
                "group_id": {"type": "integer", "description": "可选，限定某个群"},
                "limit": {"type": "integer", "description": "候选上限，默认20，最大50"},
                "sample_size": {"type": "integer", "description": "随机时间样本数，默认5，最大10"},
                "include_sender_messages": {
                    "type": "boolean",
                    "description": "仅在用户明确要求时，将已确认广告发送者在同一时间窗内的其他用户消息加入快照",
                },
            },
            "required": [],
        }

    @staticmethod
    def _iso_time(ts: Any) -> Optional[str]:
        try:
            return datetime.fromtimestamp(int(ts), tz=timezone.utc).astimezone().isoformat()
        except Exception:
            return None

    async def _deletable_group_ids(self, rows: List[Dict[str, Any]], context: Any) -> tuple[Optional[set[int]], List[Dict[str, Any]]]:
        bot = getattr(context, "bot_instance", None) if context is not None else None
        if not bot:
            return None, []
        try:
            bot_user = await bot.get_me()
            bot_id = int(getattr(bot_user, "id", 0) or 0)
        except Exception as exc:
            return set(), [{"group_id": None, "reason": f"bot_identity_check_failed:{str(exc)[:120]}"}]

        allowed: set[int] = set()
        excluded: List[Dict[str, Any]] = []
        for group_id in sorted({int(row["group_id"]) for row in rows}):
            try:
                member = await bot.get_chat_member(group_id, bot_id)
                status = str(getattr(member, "status", "") or "")
                can_delete = status == "creator" or (
                    status == "administrator" and bool(getattr(member, "can_delete_messages", False))
                )
                if can_delete:
                    allowed.add(group_id)
                else:
                    excluded.append({
                        "group_id": group_id,
                        "reason": "bot_missing_delete_permission",
                        "bot_status": status,
                        "can_delete_messages": bool(getattr(member, "can_delete_messages", False)),
                    })
            except Exception as exc:
                excluded.append({
                    "group_id": group_id,
                    "reason": f"permission_check_failed:{str(exc)[:120]}",
                })
        return allowed, excluded

    async def execute(self, params: Dict[str, Any], context=None) -> dict:
        import aiosqlite

        hours = max(1, min(int(params.get("hours") or 24), 24))
        limit = max(1, min(int(params.get("limit") or 20), 50))
        sample_size = max(1, min(int(params.get("sample_size") or 5), 10))
        include_sender_messages = bool(params.get("include_sender_messages", False))
        group_id = params.get("group_id")
        cutoff_ts = int(time.time()) - hours * 3600
        tenant_id = _context_scoped_tenant_id(context)

        async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
            db.row_factory = aiosqlite.Row
            schema_cur = await db.execute("PRAGMA table_info(group_messages)")
            columns = {row[1] for row in await schema_cur.fetchall()}
            required = {"msg_id", "group_id", "tg_msg_id", "user_id", "content", "message_ts", "is_advertising"}
            if not required.issubset(columns):
                return {
                    "success": False,
                    "message": "group_messages 缺少广告查询所需字段",
                    "data_complete": False,
                    "analysis_allowed": False,
                    "evidence_count": 0,
                }

            where = [
                "COALESCE(is_advertising, 0) = 1",
                "tg_msg_id IS NOT NULL",
                "COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) >= ?",
                "COALESCE(sender_type, 'user') = 'user'",
                "user_id != ?",
            ]
            sql_params: List[Any] = [cutoff_ts, int(config.ADMIN_ID)]
            if "ad_score" in columns:
                where.append("COALESCE(ad_score, 0) >= 40")
            if "is_deleted" in columns:
                where.append("COALESCE(is_deleted, 0) = 0")
            if "ad_action_status" in columns:
                where.append("COALESCE(ad_action_status, 'none') NOT IN ('deleted', 'deleting')")
            if tenant_id and "tenant_id" in columns:
                where.append("tenant_id = ?")
                sql_params.append(tenant_id)
            if str(group_id or "").strip():
                where.append("group_id = ?")
                sql_params.append(int(group_id))

            where_sql = " AND ".join(where)
            stats_cur = await db.execute(
                f"""
                SELECT COUNT(*) AS total_count,
                       MIN(COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER))) AS earliest_ts,
                       MAX(COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER))) AS latest_ts
                FROM group_messages
                WHERE {where_sql}
                """,
                tuple(sql_params),
            )
            stats = await stats_cur.fetchone()
            total_count = int(stats["total_count"] or 0)
            earliest_ts = int(stats["earliest_ts"] or 0)
            latest_ts = int(stats["latest_ts"] or 0)

            sample_cur = await db.execute(
                f"""
                SELECT COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) AS effective_ts
                FROM group_messages
                WHERE {where_sql}
                ORDER BY RANDOM()
                LIMIT ?
                """,
                (*sql_params, sample_size),
            )
            sampled_times = [
                self._iso_time(row["effective_ts"])
                for row in await sample_cur.fetchall()
                if row["effective_ts"]
            ]

            sender_type_expr = "sender_type" if "sender_type" in columns else "'user' AS sender_type"
            tenant_expr = "tenant_id" if "tenant_id" in columns else "'default' AS tenant_id"
            score_expr = "COALESCE(ad_score, 0) AS ad_score" if "ad_score" in columns else "0 AS ad_score"
            cur = await db.execute(
                f"""
                SELECT msg_id, group_id, tg_msg_id, user_id, username, content,
                       {sender_type_expr}, {tenant_expr}, {score_expr},
                       COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) AS effective_ts
                FROM group_messages
                WHERE {where_sql}
                ORDER BY effective_ts DESC, msg_id DESC
                LIMIT ?
                """,
                (*sql_params, limit),
            )
            rows = [dict(row) for row in await cur.fetchall()]
            for row in rows:
                row["match_reason"] = "advertisement"

            confirmed_sender_ids = sorted({int(row.get("user_id") or 0) for row in rows if row.get("user_id")})
            sender_expanded_count = 0
            if include_sender_messages and confirmed_sender_ids and len(rows) < limit:
                existing_ids = sorted({int(row["msg_id"]) for row in rows})
                sender_where = [
                    f"user_id IN ({','.join('?' for _ in confirmed_sender_ids)})",
                    "tg_msg_id IS NOT NULL",
                    "COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) >= ?",
                    "COALESCE(sender_type, 'user') = 'user'",
                    "user_id != ?",
                ]
                sender_params: List[Any] = [*confirmed_sender_ids, cutoff_ts, int(config.ADMIN_ID)]
                if existing_ids:
                    sender_where.append(f"msg_id NOT IN ({','.join('?' for _ in existing_ids)})")
                    sender_params.extend(existing_ids)
                if "is_deleted" in columns:
                    sender_where.append("COALESCE(is_deleted, 0) = 0")
                if "ad_action_status" in columns:
                    sender_where.append("COALESCE(ad_action_status, 'none') NOT IN ('deleted', 'deleting')")
                if tenant_id and "tenant_id" in columns:
                    sender_where.append("tenant_id = ?")
                    sender_params.append(tenant_id)
                if str(group_id or "").strip():
                    sender_where.append("group_id = ?")
                    sender_params.append(int(group_id))
                sender_params.append(limit - len(rows))
                sender_cur = await db.execute(
                    f"""
                    SELECT msg_id, group_id, tg_msg_id, user_id, username, content,
                           {sender_type_expr}, {tenant_expr}, {score_expr},
                           COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) AS effective_ts
                    FROM group_messages
                    WHERE {' AND '.join(sender_where)}
                    ORDER BY effective_ts DESC, msg_id DESC
                    LIMIT ?
                    """,
                    tuple(sender_params),
                )
                for sender_row in [dict(row) for row in await sender_cur.fetchall()]:
                    sender_row["match_reason"] = "confirmed_sender_history"
                    rows.append(sender_row)
                    sender_expanded_count += 1

        allowed_group_ids, permission_excluded_groups = await self._deletable_group_ids(rows, context)
        authorized_sender_ids = {
            int(row.get("user_id") or 0)
            for row in rows
            if row.get("match_reason") == "advertisement"
            and (allowed_group_ids is None or int(row["group_id"]) in allowed_group_ids)
            and row.get("user_id")
        }
        refs = []
        records = []
        permission_excluded_count = 0
        sender_scope_excluded_count = 0
        for row in rows:
            ref = {
                "msg_id": int(row["msg_id"]),
                "group_id": int(row["group_id"]),
                "chat_id": int(row["group_id"]),
                "tg_msg_id": int(row["tg_msg_id"]),
                "user_id": int(row.get("user_id") or 0),
                "message_ts": int(row.get("effective_ts") or 0),
                "tenant_id": str(row.get("tenant_id") or "default"),
                "match_reason": str(row.get("match_reason") or "advertisement"),
            }
            group_is_deletable = allowed_group_ids is None or int(row["group_id"]) in allowed_group_ids
            sender_scope_allowed = (
                row.get("match_reason") != "confirmed_sender_history"
                or int(row.get("user_id") or 0) in authorized_sender_ids
            )
            is_deletable = group_is_deletable and sender_scope_allowed
            if not group_is_deletable:
                permission_excluded_count += 1
            elif not sender_scope_allowed:
                sender_scope_excluded_count += 1
            if is_deletable:
                refs.append(ref)
            records.append({
                **ref,
                "username": row.get("username") or "",
                "ad_score": int(row.get("ad_score") or 0),
                "recorded_at": self._iso_time(row.get("effective_ts")),
                "content_preview": str(row.get("content") or "")[:120],
                "deletable": is_deletable,
            })

        snapshot_ts = int(time.time())
        return {
            "success": True,
            "source": "sqlite.group_messages",
            "data_complete": True,
            "analysis_allowed": bool(records),
            "evidence_count": len(records),
            "data": {
                "hours": hours,
                "cutoff_ts": cutoff_ts,
                "snapshot_created_at": snapshot_ts,
                "total_eligible_count": total_count,
                "snapshot_count": len(records),
                "deletable_count": len(refs),
                "permission_excluded_count": permission_excluded_count,
                "permission_excluded_groups": permission_excluded_groups,
                "sender_scope_excluded_count": sender_scope_excluded_count,
                "include_sender_messages": include_sender_messages,
                "confirmed_sender_ids": sorted(authorized_sender_ids),
                "sender_expanded_count": sender_expanded_count,
                "earliest_recorded_at": self._iso_time(earliest_ts),
                "latest_recorded_at": self._iso_time(latest_ts),
                "random_sample_times": sampled_times,
                "records": records,
                "candidate_refs": refs,
                "safety_policy": {
                    "max_batch": 50,
                    "admin_messages_excluded": True,
                    "bot_messages_excluded": True,
                    "channel_posts_excluded": True,
                    "minimum_ad_score": 40,
                    "delete_permission_prechecked": allowed_group_ids is not None,
                },
            },
            "count": len(records),
            "limitation": (
                None if records else
                f"最近 {hours} 小时没有找到符合安全删除条件的已标记用户广告；不代表群历史中没有广告。"
            ),
            "message": f"最近 {hours} 小时查到 {total_count} 条合格广告，本次快照锁定 {len(records)} 条",
        }


class DeleteRecentAdMessagesTool(BaseTool):
    """删除查询快照中的最近广告消息。"""
    name = "delete_recent_ad_messages"
    description = (
        "按 query_recent_ad_messages 生成的精确快照删除广告。"
        "必须二次确认，最多50条，且会逐条复核租户、时间、广告标记、发送者和 bot 权限。"
    )
    category = "security"

    @property
    def parameters(self) -> dict:
        return {
            "properties": {
                "message_refs": {"type": "array", "items": {"type": "object"}, "description": "查询快照的精确消息引用"},
                "cutoff_ts": {"type": "integer", "description": "快照时间窗下界"},
                "snapshot_created_at": {"type": "integer", "description": "快照生成时间"},
                "hours": {"type": "integer", "description": "查询小时数"},
                "include_sender_messages": {"type": "boolean", "description": "快照是否明确包含广告发送者的其他消息"},
                "confirmed_sender_ids": {"type": "array", "items": {"type": "integer"}},
            },
            "required": ["message_refs", "cutoff_ts", "snapshot_created_at"],
        }

    async def _audit(self, *, context: Any, action: str, payload: Dict[str, Any]) -> None:
        import aiosqlite

        metadata = _context_metadata(context)
        tenant_id = _context_scoped_tenant_id(context) or str(metadata.get("tenant_id") or "default")
        try:
            async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
                table_cur = await db.execute(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name='tenant_audit_logs'"
                )
                if not await table_cur.fetchone():
                    return
                await db.execute(
                    """
                    INSERT INTO tenant_audit_logs (
                        tenant_id, merchant_id, operator_role, operator_id,
                        action, resource_type, resource_id, payload_json, created_at
                    ) VALUES (?, ?, ?, ?, ?, 'group_messages', ?, ?, CURRENT_TIMESTAMP)
                    """,
                    (
                        tenant_id,
                        _context_merchant_id(context),
                        str(metadata.get("role") or "agent"),
                        str(getattr(context, "user_id", "") or ""),
                        action,
                        str(payload.get("batch_id") or "recent_ads"),
                        json.dumps(payload, ensure_ascii=False, default=str),
                    ),
                )
                await db.commit()
        except Exception as exc:
            logger.warning("广告删除审计写入失败: %s", exc)

    async def execute(self, params: Dict[str, Any], context=None) -> dict:
        import aiosqlite

        refs = params.get("message_refs") or []
        if not isinstance(refs, list) or not refs:
            return {"success": False, "action_taken": False, "message": "缺少经过查询锁定的广告消息快照"}
        if len(refs) > 50:
            return {"success": False, "action_taken": False, "message": "单次删除不能超过50条"}
        if not bool(getattr(config, "AD_DELETE_ENABLED", False)):
            return {
                "success": False,
                "action_taken": False,
                "blocked": True,
                "message": "AD_DELETE_ENABLED 未开启，已阻止真实删除",
            }

        bot = getattr(context, "bot_instance", None) if context is not None else None
        if not bot:
            return {"success": False, "action_taken": False, "message": "bot 未初始化，无法删除 Telegram 消息"}

        try:
            cutoff_ts = int(params.get("cutoff_ts") or 0)
            snapshot_created_at = int(params.get("snapshot_created_at") or 0)
        except Exception:
            return {"success": False, "action_taken": False, "message": "快照时间参数无效"}
        now_ts = int(time.time())
        if cutoff_ts <= 0 or snapshot_created_at <= 0 or snapshot_created_at > now_ts + 60:
            return {"success": False, "action_taken": False, "message": "快照时间参数不合法"}
        if now_ts - snapshot_created_at > 15 * 60:
            return {"success": False, "action_taken": False, "message": "删除快照已超过15分钟，请重新查询"}

        tenant_id = _context_scoped_tenant_id(context)
        include_sender_messages = bool(params.get("include_sender_messages", False))
        confirmed_sender_ids = {
            int(sender_id) for sender_id in (params.get("confirmed_sender_ids") or [])
            if str(sender_id or "").strip()
        }
        verified: List[Dict[str, Any]] = []
        rejected: List[Dict[str, Any]] = []
        async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
            db.row_factory = aiosqlite.Row
            schema_cur = await db.execute("PRAGMA table_info(group_messages)")
            columns = {row[1] for row in await schema_cur.fetchall()}
            for ref in refs:
                try:
                    msg_id = int(ref.get("msg_id"))
                    group_id = int(ref.get("group_id") or ref.get("chat_id"))
                    tg_msg_id = int(ref.get("tg_msg_id"))
                    user_id = int(ref.get("user_id") or 0)
                    match_reason = str(ref.get("match_reason") or "advertisement")
                except Exception:
                    rejected.append({"ref": ref, "reason": "invalid_reference"})
                    continue

                where = [
                    "msg_id = ?", "group_id = ?", "tg_msg_id = ?", "user_id = ?",
                    "COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) >= ?",
                    "COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) <= ?",
                    "COALESCE(sender_type, 'user') = 'user'",
                    "user_id != ?",
                ]
                sql_params: List[Any] = [msg_id, group_id, tg_msg_id, user_id, cutoff_ts, snapshot_created_at, int(config.ADMIN_ID)]
                if match_reason == "confirmed_sender_history":
                    if not include_sender_messages or user_id not in confirmed_sender_ids:
                        rejected.append({"msg_id": msg_id, "reason": "sender_expansion_not_authorized"})
                        continue
                else:
                    where.append("COALESCE(is_advertising, 0) = 1")
                    if "ad_score" in columns:
                        where.append("COALESCE(ad_score, 0) >= 40")
                if "is_deleted" in columns:
                    where.append("COALESCE(is_deleted, 0) = 0")
                if "ad_action_status" in columns:
                    where.append("COALESCE(ad_action_status, 'none') NOT IN ('deleted', 'deleting')")
                if tenant_id and "tenant_id" in columns:
                    where.append("tenant_id = ?")
                    sql_params.append(tenant_id)
                cur = await db.execute(
                    f"SELECT * FROM group_messages WHERE {' AND '.join(where)} LIMIT 1",
                    tuple(sql_params),
                )
                row = await cur.fetchone()
                if not row:
                    rejected.append({"msg_id": msg_id, "reason": "snapshot_revalidation_failed"})
                    continue
                verified.append(dict(row))

        try:
            bot_user = await bot.get_me()
            bot_id = int(getattr(bot_user, "id", 0) or 0)
        except Exception as exc:
            return {"success": False, "action_taken": False, "message": f"无法校验 bot 身份: {str(exc)[:160]}"}
        if not bot_id:
            return {"success": False, "action_taken": False, "message": "无法获取 bot ID"}

        permission_cache: Dict[int, bool] = {}
        deleted: List[Dict[str, Any]] = []
        failures: List[Dict[str, Any]] = []
        for row in verified:
            group_id = int(row["group_id"])
            tg_msg_id = int(row["tg_msg_id"])
            user_id = int(row.get("user_id") or 0)

            if group_id not in permission_cache:
                try:
                    bot_member = await bot.get_chat_member(group_id, bot_id)
                    status = str(getattr(bot_member, "status", "") or "")
                    permission_cache[group_id] = status == "creator" or (
                        status == "administrator" and bool(getattr(bot_member, "can_delete_messages", False))
                    )
                except Exception:
                    permission_cache[group_id] = False
            if not permission_cache[group_id]:
                failures.append({"msg_id": row.get("msg_id"), "reason": "bot_missing_delete_permission"})
                continue

            try:
                sender_member = await bot.get_chat_member(group_id, user_id)
                if str(getattr(sender_member, "status", "") or "") in {"administrator", "creator"}:
                    failures.append({"msg_id": row.get("msg_id"), "reason": "sender_is_admin"})
                    continue
            except Exception as exc:
                failures.append({"msg_id": row.get("msg_id"), "reason": f"sender_check_failed:{str(exc)[:120]}"})
                continue

            try:
                claimed = True
                if "ad_action_status" in row:
                    async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
                        claim_cur = await db.execute(
                            """
                            UPDATE group_messages
                            SET ad_action_status = 'deleting'
                            WHERE msg_id = ?
                              AND COALESCE(ad_action_status, 'none') NOT IN ('deleted', 'deleting')
                            """,
                            (int(row["msg_id"]),),
                        )
                        await db.commit()
                        claimed = claim_cur.rowcount == 1
                if not claimed:
                    failures.append({"msg_id": row.get("msg_id"), "reason": "already_claimed_or_deleted"})
                    continue

                await bot.delete_message(chat_id=group_id, message_id=tg_msg_id)
            except Exception as exc:
                if "ad_action_status" in row:
                    async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
                        await db.execute(
                            "UPDATE group_messages SET ad_action_status = 'failed' WHERE msg_id = ? AND ad_action_status = 'deleting'",
                            (int(row["msg_id"]),),
                        )
                        await db.commit()
                failures.append({"msg_id": row.get("msg_id"), "reason": f"telegram_delete_failed:{str(exc)[:160]}"})
                continue

            async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
                set_parts = []
                if "is_deleted" in row:
                    set_parts.append("is_deleted = 1")
                if "deleted_at" in row:
                    set_parts.append("deleted_at = CURRENT_TIMESTAMP")
                if "ad_action_status" in row:
                    set_parts.append("ad_action_status = 'deleted'")
                if set_parts:
                    await db.execute(
                        f"UPDATE group_messages SET {', '.join(set_parts)} WHERE msg_id = ?",
                        (int(row["msg_id"]),),
                    )
                    await db.commit()
            if bool(row.get("is_advertising")) and int(row.get("ad_score") or 0) >= 40:
                try:
                    await database.confirm_ad_fingerprint(
                        tenant_id=row.get("tenant_id"),
                        sender_id=user_id,
                        content=str(row.get("content") or ""),
                        source_msg_id=int(row["msg_id"]),
                        confirmed_by=f"agent:{getattr(context, 'user_id', '')}",
                    )
                except Exception as exc:
                    logger.warning("已删除广告写入指纹库失败 msg_id=%s: %s", row.get("msg_id"), exc)
            deleted.append({"msg_id": row.get("msg_id"), "group_id": group_id, "tg_msg_id": tg_msg_id})

        batch_id = f"recent_ads:{snapshot_created_at}:{getattr(context, 'user_id', 'agent')}"
        await self._audit(
            context=context,
            action="agent_delete_recent_ads",
            payload={
                "batch_id": batch_id,
                "snapshot_created_at": snapshot_created_at,
                "cutoff_ts": cutoff_ts,
                "requested_count": len(refs),
                "verified_count": len(verified),
                "deleted": deleted,
                "rejected": rejected,
                "failures": failures,
            },
        )
        return {
            "success": not failures and not rejected,
            "action_taken": bool(deleted),
            "data": {
                "batch_id": batch_id,
                "requested_count": len(refs),
                "verified_count": len(verified),
                "deleted_count": len(deleted),
                "deleted": deleted,
                "rejected": rejected,
                "failures": failures,
            },
            "message": f"删除完成：成功 {len(deleted)} 条，拒绝 {len(rejected)} 条，失败 {len(failures)} 条",
        }


# ===== 工具 4b：广告删除 =====

class DeleteAdMessageTool(BaseTool):
    """删除已经识别为广告的群消息"""
    name = "delete_ad_message"
    description = (
        "删除群内广告消息。可以按 group_id + tg_msg_id 精确删除，或删除最近一条已标记广告消息。"
        "只处理广告消息，删除前需要管理员确认。"
    )
    category = "security"

    @property
    def parameters(self) -> dict:
        return {
            "properties": {
                "group_id": {"type": "integer", "description": "群组 ID，可选"},
                "tg_msg_id": {"type": "integer", "description": "Telegram 消息 ID，可选"},
                "msg_id": {"type": "integer", "description": "数据库消息 ID，可选"},
                "delete_latest_ad": {"type": "boolean", "description": "未指定消息时删除最近一条广告消息"},
            },
            "required": []
        }

    async def _find_candidate(self, params: Dict[str, Any]) -> Optional[Dict[str, Any]]:
        import aiosqlite

        group_id = params.get("group_id")
        tg_msg_id = params.get("tg_msg_id")
        msg_id = params.get("msg_id")
        where = ["COALESCE(is_advertising, 0) = 1", "tg_msg_id IS NOT NULL"]
        sql_params: List[Any] = []

        if msg_id:
            where.append("msg_id = ?")
            sql_params.append(int(msg_id))
        if group_id:
            where.append("group_id = ?")
            sql_params.append(int(group_id))
        if tg_msg_id:
            where.append("tg_msg_id = ?")
            sql_params.append(int(tg_msg_id))

        async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
            db.row_factory = aiosqlite.Row
            cur = await db.execute(
                f"""
                SELECT msg_id, group_id, tg_msg_id, user_id, username, content,
                       COALESCE(ad_score, 0) AS ad_score,
                       COALESCE(ad_confidence, 'low') AS ad_confidence
                FROM group_messages
                WHERE {' AND '.join(where)}
                ORDER BY COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) DESC, msg_id DESC
                LIMIT 1
                """,
                tuple(sql_params),
            )
            row = await cur.fetchone()
            return dict(row) if row else None

    async def execute(self, params: Dict[str, Any], context=None) -> dict:
        candidate = await self._find_candidate(params)
        if not candidate:
            return {
                "success": False,
                "message": "没有找到可删除的已标记广告消息；请先执行广告检测或提供 group_id/tg_msg_id",
            }

        bot = getattr(context, "bot_instance", None) if context is not None else None
        if not bot:
            return {"success": False, "message": "bot 未初始化，无法删除 Telegram 群消息"}

        group_id = int(candidate["group_id"])
        tg_msg_id = int(candidate["tg_msg_id"])
        user_id = int(candidate.get("user_id") or 0)

        if user_id:
            try:
                member = await bot.get_chat_member(group_id, user_id)
                if getattr(member, "status", "") in ("administrator", "creator"):
                    return {"success": False, "message": "不能删除群管理员或群主消息"}
            except Exception as exc:
                logger.warning("检查群成员身份失败，继续按机器人权限删除: %s", exc)

        await bot.delete_message(chat_id=group_id, message_id=tg_msg_id)
        deleted = await database.delete_group_message(group_id, tg_msg_id)
        return {
            "success": True,
            "action_taken": True,
            "data": {
                "group_id": group_id,
                "tg_msg_id": tg_msg_id,
                "msg_id": candidate.get("msg_id"),
                "db_deleted": bool(deleted),
                "ad_score": candidate.get("ad_score"),
                "content_preview": str(candidate.get("content") or "")[:120],
            },
            "message": f"已删除广告消息 group_id={group_id}, tg_msg_id={tg_msg_id}",
        }


# ===== 工具 4c：广告聚合查询 =====

class QueryAdClustersTool(BaseTool):
    """查询广告聚合候选摘要，供 LLM 先取证再判断。"""
    name = "query_ad_clusters"
    description = (
        "查询当前已落库群/频道消息的广告聚合候选。会按 MD5 去重、实时广告评分、重复次数聚合，"
        "PostgreSQL/pgvector 可用时读取 sidecar cluster，不可用时降级查询 SQLite group_messages。"
    )
    category = "security"

    @property
    def parameters(self) -> dict:
        return {
            "properties": {
                "group_id": {"type": "integer", "description": "可选，限制某个群/频道"},
                "status": {"type": "string", "enum": ["pending", "ad", "not_ad", "all"], "description": "cluster 状态"},
                "limit": {"type": "integer", "description": "返回 cluster 数量，默认20"},
                "window_minutes": {"type": "integer", "description": "SQLite fallback 查询窗口，默认60分钟"},
            },
            "required": []
        }

    async def execute(self, params: Dict[str, Any], context=None) -> dict:
        from services.ad_pipeline import get_ad_pipeline

        tenant_id = _context_scoped_tenant_id(context)
        group_id = params.get("group_id")
        result = await get_ad_pipeline().query_clusters(
            tenant_id=tenant_id,
            group_id=int(group_id) if str(group_id or "").strip() else None,
            status=str(params.get("status") or "pending"),
            limit=max(1, int(params.get("limit") or 20)),
            window_minutes=max(1, int(params.get("window_minutes") or 60)),
        )
        return result


class DeleteAdClusterTool(BaseTool):
    """按广告聚合 cluster 批量删除对应 Telegram 群/频道消息。"""
    name = "delete_ad_cluster"
    description = (
        "删除一个广告 cluster 下的所有消息。默认 dry-run，只有 AD_DELETE_ENABLED=true 且参数 dry_run=false 时才实际删除。"
        "执行前会跳过管理员/群主消息。"
    )
    category = "security"

    @property
    def parameters(self) -> dict:
        return {
            "properties": {
                "cluster_id": {"type": "string", "description": "query_ad_clusters 返回的 cluster_id"},
                "window_minutes": {"type": "integer", "description": "后端重新解析 cluster 的时间窗，默认1440，最大1440"},
                "dry_run": {"type": "boolean", "description": "是否模拟执行，默认 true"},
                "ban_senders": {"type": "boolean", "description": "是否同时封禁发送者，默认 false 且受 AD_KICK_ENABLED 控制"},
            },
            "required": ["cluster_id"]
        }

    async def execute(self, params: Dict[str, Any], context=None) -> dict:
        from services.ad_pipeline import cluster_is_confirmed_ad, delete_cluster_messages, get_ad_pipeline

        cluster_id = str(params.get("cluster_id") or "").strip()
        if not cluster_id:
            return {"success": False, "message": "缺少 cluster_id"}

        clusters_result = await get_ad_pipeline().query_clusters(
            tenant_id=_context_scoped_tenant_id(context),
            status="all",
            limit=100,
            window_minutes=max(1, min(int(params.get("window_minutes") or 1440), 1440)),
        )
        clusters = (clusters_result.get("data") or {}).get("clusters") or []
        matched = next((c for c in clusters if str(c.get("cluster_id")) == cluster_id), None)
        refs = (matched or {}).get("message_refs") or []

        if not refs:
            return {"success": False, "message": "未从后端聚类查询到该 cluster，已拒绝删除"}
        if not cluster_is_confirmed_ad(matched):
            return {"success": False, "message": "该 cluster 仅为待判定候选，尚未确认是广告，已拒绝删除"}

        bot = getattr(context, "bot_instance", None) if context is not None else None
        if not bot:
            try:
                import web_admin
                bot = web_admin._bot_instance
            except Exception:
                bot = None

        result = await delete_cluster_messages(
            cluster={"cluster_id": cluster_id, "message_refs": refs},
            bot=bot,
            dry_run=bool(params.get("dry_run", True)),
            ban_senders=bool(params.get("ban_senders", False)),
        )
        return result


# ===== 工具 5：系统查询 =====

class SystemQueryTool(BaseTool):
    """系统状态与配置查询工具"""
    name = "system_query"
    description = (
        "查询系统状态和数据统计。支持：在线用户数、监听地址数、绑定群/频道、"
        "群聊消息、频道消息、最近消息统计、系统运行时间等。"
    )
    category = "system"

    @property
    def parameters(self) -> dict:
        return {
            "properties": {
                "query_type": {
                    "type": "string",
                    "enum": [
                        "stats_overview",      # 统计概览
                        "bound_chats",         # 机器人绑定的群组和频道
                        "active_groups",       # 活跃群组
                        "group_messages",      # 群聊消息
                        "channel_messages",    # 频道消息
                        "monitors_list",       # 监听列表
                        "recent_senders",      # 最近发消息的用户
                        "recent_messages",     # 最近消息
                        "group_ad_messages",   # 群广告消息
                        "user_stats",          # 用户统计
                        "potential_users",     # 潜在用户
                    ],
                    "description": "查询类型"
                },
                "limit": {
                    "type": "integer",
                    "description": "返回数量限制（默认10）"
                },
                "group_id": {
                    "type": "integer",
                    "description": "群组ID（可选，不填则查询全部群组）"
                },
                "group_name": {
                    "type": "string",
                    "description": "群组/频道名称关键词（可选，用于先解析 chat_id）"
                },
            },
            "required": ["query_type"]
        }

    def _tenant_id_from_context(self, context) -> Optional[str]:
        return _context_scoped_tenant_id(context)

    async def _table_columns(self, db, table_name: str) -> set:
        cursor = await db.execute(f"PRAGMA table_info({table_name})")
        return {row[1] for row in await cursor.fetchall()}

    def _serialize_chat_target(self, row: Dict[str, Any], chat_type: str) -> Dict[str, Any]:
        return {
            "chat_id": row.get("chat_id") or row.get("group_id"),
            "title": row.get("chat_title") or row.get("group_title") or "",
            "chat_type": chat_type,
            "username": row.get("chat_username"),
            "is_monitored": bool(row.get("is_monitored", 0)) if "is_monitored" in row else None,
            "can_broadcast": bool(row.get("can_broadcast", 1)) if "can_broadcast" in row else None,
            "tenant_id": row.get("tenant_id"),
            "updated_at": row.get("updated_at"),
        }

    async def _query_bound_chats(self, tenant_id: Optional[str], limit: int) -> dict:
        groups = await database.get_tracked_groups_with_stats(tenant_id=tenant_id)
        targets = await database.get_broadcast_targets(enabled_only=False, tenant_id=tenant_id)

        tracked_by_id: Dict[int, Dict[str, Any]] = {}
        for group in groups:
            try:
                chat_id = int(group.get("group_id"))
            except Exception:
                continue
            tracked_by_id[chat_id] = self._serialize_chat_target(group, "supergroup")

        channels: List[Dict[str, Any]] = []
        broadcast_only_groups: List[Dict[str, Any]] = []
        for target in targets:
            try:
                chat_id = int(target.get("chat_id"))
            except Exception:
                continue
            chat_type = str(target.get("chat_type") or "")
            serialized = self._serialize_chat_target(target, chat_type)
            if chat_id in tracked_by_id:
                tracked_by_id[chat_id].update({
                    "chat_type": chat_type or tracked_by_id[chat_id].get("chat_type"),
                    "username": serialized.get("username") or tracked_by_id[chat_id].get("username"),
                    "can_broadcast": serialized.get("can_broadcast"),
                    "updated_at": serialized.get("updated_at") or tracked_by_id[chat_id].get("updated_at"),
                })
            elif chat_type == "channel":
                channels.append(serialized)
            else:
                serialized["source"] = "broadcast_targets_only"
                broadcast_only_groups.append(serialized)

        groups_only = list(tracked_by_id.values())
        all_targets = groups_only + channels + broadcast_only_groups

        return {
            "success": True,
            "data_complete": True,
            "evidence_count": len(all_targets),
            "data": {
                "group_count": len(groups_only),
                "channel_count": len(channels),
                "broadcast_only_group_count": len(broadcast_only_groups),
                "total_count": len(all_targets),
                "groups": groups_only[:limit],
                "channels": channels[:limit],
                "broadcast_only_groups": broadcast_only_groups[:limit],
            },
            "count": len(all_targets),
            "message": (
                f"机器人绑定聊天：群组 {len(groups_only)} 个，频道 {len(channels)} 个"
                + (f"，另有仅广播目标群 {len(broadcast_only_groups)} 个" if broadcast_only_groups else "")
            ),
        }

    async def _query_group_message_rows(
        self,
        *,
        tenant_id: Optional[str],
        limit: int,
        group_id: Optional[int],
        channel_only: bool,
    ) -> dict:
        import aiosqlite

        lim = max(1, min(int(limit or 10), 100))
        where_parts = []
        params_sql: List[Any] = []

        if group_id is not None:
            where_parts.append("gm.group_id = ?")
            params_sql.append(group_id)
        if channel_only:
            where_parts.append("COALESCE(gm.sender_type, '') = 'channel'")

        async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
            db.row_factory = aiosqlite.Row
            gm_cols = await self._table_columns(db, "group_messages")
            if not gm_cols:
                return {
                    "success": True,
                    "data_complete": False,
                    "analysis_allowed": False,
                    "evidence_count": 0,
                    "data": [],
                    "count": 0,
                    "message": "当前数据库尚未创建群消息表",
                }
            if tenant_id and "tenant_id" in gm_cols:
                where_parts.append("gm.tenant_id = ?")
                params_sql.append(tenant_id)

            tg_cols = await self._table_columns(db, "tracked_groups")
            join_group_title = bool(tg_cols)
            where_sql = f"WHERE {' AND '.join(where_parts)}" if where_parts else ""
            title_select = "tg.group_title AS group_title" if join_group_title else "NULL AS group_title"
            title_join = "LEFT JOIN tracked_groups tg ON tg.group_id = gm.group_id" if join_group_title else ""

            cursor = await db.execute(
                f"""
                SELECT gm.msg_id, gm.group_id, gm.tg_msg_id, gm.user_id, gm.username,
                       gm.content, gm.msg_type, gm.sender_type, gm.message_ts, gm.timestamp,
                       COALESCE(gm.is_advertising, 0) AS is_advertising,
                       COALESCE(gm.ad_score, 0) AS ad_score,
                       COALESCE(gm.ad_confidence, 'low') AS ad_confidence,
                       gm.ad_findings,
                       {title_select}
                FROM group_messages gm
                {title_join}
                {where_sql}
                ORDER BY COALESCE(gm.message_ts, CAST(strftime('%s', gm.timestamp) AS INTEGER)) DESC,
                         gm.tg_msg_id DESC,
                         gm.msg_id DESC
                LIMIT ?
                """,
                (*params_sql, lim),
            )
            rows = [dict(row) for row in await cursor.fetchall()]

        for row in rows:
            raw_findings = row.get("ad_findings")
            if isinstance(raw_findings, str) and raw_findings.strip():
                try:
                    row["ad_findings"] = json.loads(raw_findings)
                except Exception:
                    pass
            live_text = str(row.get("content") or "").strip()
            if live_text and row.get("sender_type") in {"user", "channel", None, ""}:
                try:
                    live_flags = detect_advertising_signals(
                        text=live_text,
                        context_type="channel_message" if channel_only else "group_message",
                    )
                    row["live_is_advertising"] = bool(live_flags.get("is_advertising"))
                    row["live_ad_score"] = int(live_flags.get("ad_score", 0) or 0)
                    row["live_ad_confidence"] = str(live_flags.get("confidence") or "low")
                    row["live_ad_findings"] = live_flags.get("findings") or []
                    row["live_ad_recommendation"] = live_flags.get("recommendation") or ""
                    if row["live_ad_score"] > int(row.get("ad_score") or 0):
                        row["ad_score"] = row["live_ad_score"]
                        row["is_advertising"] = 1 if row["live_is_advertising"] else row.get("is_advertising", 0)
                        row["ad_confidence"] = row["live_ad_confidence"]
                except Exception:
                    row["live_ad_error"] = "live_detection_failed"

        label = "频道消息" if channel_only else "群聊消息"
        has_evidence = len(rows) > 0
        live_ad_count = sum(1 for row in rows if row.get("live_is_advertising"))
        distinct_senders = len({row.get("user_id") for row in rows if row.get("user_id")})
        return {
            "success": True,
            "source": "sql.group_messages",
            "data_complete": has_evidence,
            "analysis_allowed": has_evidence,
            "evidence_count": len(rows),
            "summary": {
                "distinct_senders": distinct_senders,
                "live_ad_count": live_ad_count,
                "live_detection_applied": True,
            },
            "limitation": (
                None if has_evidence else
                "没有 SQL 消息证据；不能据此判断该群/频道没有广告或没有异常。"
            ),
            "data": rows,
            "count": len(rows),
            "message": (
                f"{label}列表 (前{len(rows)}条)"
                if has_evidence else
                f"未查询到可见的{label}；只能说明当前 SQL 没有 bot 已收到并落库的消息"
            ),
        }

    async def _resolve_group_name(self, *, tenant_id: Optional[str], group_name: str) -> dict:
        import aiosqlite

        name = str(group_name or "").strip().strip("*「」『』“”\"' ")
        if not name:
            return {"status": "empty", "matches": []}

        async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
            db.row_factory = aiosqlite.Row
            tg_cols = await self._table_columns(db, "tracked_groups")
            if not tg_cols:
                return {"status": "not_found", "matches": []}
            where = ["group_title LIKE ?"]
            params: List[Any] = [f"%{name}%"]
            if tenant_id and "tenant_id" in tg_cols:
                where.append("tenant_id = ?")
                params.append(tenant_id)
            monitored_expr = "is_monitored" if "is_monitored" in tg_cols else "0 AS is_monitored"
            tenant_expr = "tenant_id" if "tenant_id" in tg_cols else "NULL AS tenant_id"
            updated_expr = "updated_at" if "updated_at" in tg_cols else "CURRENT_TIMESTAMP AS updated_at"
            updated_order = "updated_at" if "updated_at" in tg_cols else "CURRENT_TIMESTAMP"
            cursor = await db.execute(
                f"""
                SELECT group_id, group_title, {monitored_expr}, {tenant_expr}, {updated_expr}
                FROM tracked_groups
                WHERE {' AND '.join(where)}
                ORDER BY
                    CASE WHEN group_title = ? THEN 0 ELSE 1 END,
                    {updated_order} DESC
                LIMIT 10
                """,
                (*params, name),
            )
            matches = [dict(row) for row in await cursor.fetchall()]

        if not matches:
            return {"status": "not_found", "matches": [], "query": name}
        if len(matches) > 1:
            return {"status": "ambiguous", "matches": matches, "query": name}
        return {"status": "ok", "matches": matches, "query": name, "group_id": int(matches[0]["group_id"])}

    async def execute(self, params: Dict[str, Any], context=None) -> dict:
        query_type = params.get("query_type", "stats_overview")
        limit = params.get("limit", 10)
        group_id = params.get("group_id")
        group_name = params.get("group_name")
        tenant_id = self._tenant_id_from_context(context)
        merchant_id = _context_merchant_id(context)

        try:
            if query_type == "stats_overview":
                # 多维度聚合统计
                groups = await database.get_tracked_groups_with_stats(tenant_id=tenant_id)
                users = await database.get_inbox_list(tenant_id=tenant_id, merchant_id=merchant_id)
                targets = await database.get_broadcast_targets(enabled_only=True, tenant_id=tenant_id)
                channels = [t for t in targets if t.get("chat_type") == "channel"]

                import aiosqlite
                async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
                    monitor_cols = await self._table_columns(db, "address_monitors")
                    monitor_tenant_sql = " AND tenant_id = ?" if tenant_id and "tenant_id" in monitor_cols else ""
                    monitor_params = (tenant_id,) if monitor_tenant_sql else ()
                    cursor = await db.execute(
                        f"SELECT COUNT(*) FROM address_monitors WHERE is_active=1{monitor_tenant_sql}",
                        monitor_params,
                    )
                    monitor_count = (await cursor.fetchone())[0]

                return {
                    "success": True,
                    "data": {
                        "group_count": len(groups),
                        "channel_count": len(channels),
                        "user_count": len([u for u in users if u.get('from_userid')]),
                        "monitor_count": monitor_count,
                        "ai_enabled": True,
                    },
                    "message": "系统统计概览",
                }

            elif query_type == "bound_chats":
                return await self._query_bound_chats(tenant_id, max(1, int(limit or 10)))

            elif query_type == "active_groups":
                groups = await database.get_tracked_groups_with_stats(tenant_id=tenant_id)
                return {
                    "success": True,
                    "data": groups[:limit],
                    "count": len(groups),
                    "message": f"活跃群组列表 (前{min(limit, len(groups))}个)",
                }

            elif query_type == "group_messages":
                gid = int(group_id) if str(group_id or "").strip() else None
                if gid is None and str(group_name or "").strip():
                    resolved = await self._resolve_group_name(tenant_id=tenant_id, group_name=str(group_name))
                    if resolved.get("status") != "ok":
                        return {
                            "success": True,
                            "source": "sql.tracked_groups",
                            "data_complete": False,
                            "analysis_allowed": False,
                            "evidence_count": 0,
                            "data": resolved,
                            "count": len(resolved.get("matches") or []),
                            "message": "群名称无法唯一解析，不能继续判断最近消息",
                        }
                    gid = int(resolved["group_id"])
                return await self._query_group_message_rows(
                    tenant_id=tenant_id,
                    limit=max(1, int(limit or 10)),
                    group_id=gid,
                    channel_only=False,
                )

            elif query_type == "channel_messages":
                gid = int(group_id) if str(group_id or "").strip() else None
                if gid is None and str(group_name or "").strip():
                    resolved = await self._resolve_group_name(tenant_id=tenant_id, group_name=str(group_name))
                    if resolved.get("status") == "ok":
                        gid = int(resolved["group_id"])
                return await self._query_group_message_rows(
                    tenant_id=tenant_id,
                    limit=max(1, int(limit or 10)),
                    group_id=gid,
                    channel_only=True,
                )

            elif query_type == "monitors_list":
                import aiosqlite
                async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
                    db.row_factory = aiosqlite.Row
                    c = await db.execute("PRAGMA table_info(address_monitors)")
                    cols = {row[1] for row in await c.fetchall()}
                    inflow_expr = "inflow_20" if "inflow_20" in cols else "0 AS inflow_20"
                    outflow_expr = "outflow_20" if "outflow_20" in cols else "0 AS outflow_20"
                    profit_expr = "profit_20" if "profit_20" in cols else "0 AS profit_20"
                    tenant_sql = "AND tenant_id = ?" if tenant_id and "tenant_id" in cols else ""
                    sql_params = (tenant_id, max(1, int(limit or 10))) if tenant_sql else (max(1, int(limit or 10)),)

                    cursor = await db.execute(
                        f"""
                        SELECT monitor_id, address, network, remark,
                               {inflow_expr}, {outflow_expr}, {profit_expr}
                        FROM address_monitors WHERE is_active=1
                        {tenant_sql}
                        ORDER BY monitor_id DESC LIMIT ?
                    """,
                        sql_params,
                    )
                    monitors = [dict(row) for row in await cursor.fetchall()]
                return {
                    "success": True,
                    "data": monitors,
                    "count": len(monitors),
                    "message": f"监听地址列表 (前{len(monitors)}个)",
                }

            elif query_type == "recent_senders":
                # 最近给管理员发消息的用户（优先走 inbox，性能更稳）
                inbox_rows = await database.get_inbox_list(tenant_id=tenant_id, merchant_id=merchant_id)
                ai_uid = int(config.AI_ASSISTANT_USER_ID)

                senders = []
                for row in inbox_rows:
                    uid = int(row.get("from_userid", 0) or 0)
                    if not uid or uid == ai_uid:
                        continue
                    senders.append({
                        "user_id": uid,
                        "username": row.get("from_username") or f"User {uid}",
                        "unread_count": int(row.get("unread_count", 0) or 0),
                        "last_active": row.get("last_active"),
                        "summary": row.get("summary") or "",
                    })

                senders = senders[: max(1, int(limit or 10))]
                return {
                    "success": True,
                    "data": senders,
                    "count": len(senders),
                    "message": f"最近发消息用户 {len(senders)} 人",
                }

            elif query_type == "recent_messages":
                import aiosqlite
                async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
                    db.row_factory = aiosqlite.Row
                    msg_cols = await self._table_columns(db, "messages")
                    counterpart_id = await _resolve_scope_bot_id(db, tenant_id=tenant_id, merchant_id=merchant_id)
                    tenant_sql = "AND tenant_id = ?" if tenant_id and "tenant_id" in msg_cols else ""
                    sql_params: List[Any] = [counterpart_id, counterpart_id]
                    if tenant_sql:
                        sql_params.append(tenant_id)
                    sql_params.append(max(1, int(limit or 10)))
                    cursor = await db.execute(
                        f"""
                        SELECT msg_id, from_userid, to_userid, content, type AS msg_type, timestamp, status
                        FROM messages
                        WHERE is_deleted = 0
                          AND (to_userid = ? OR from_userid = ?)
                          {tenant_sql}
                        ORDER BY timestamp DESC
                        LIMIT ?
                        """,
                        tuple(sql_params),
                    )
                    rows = [dict(row) for row in await cursor.fetchall()]

                ai_uid = int(config.AI_ASSISTANT_USER_ID)
                messages = []
                for row in rows:
                    from_uid = int(row.get("from_userid", 0) or 0)
                    to_uid = int(row.get("to_userid", 0) or 0)
                    direction = "inbound" if to_uid == int(counterpart_id) else "outbound"
                    if from_uid == ai_uid or to_uid == ai_uid:
                        continue
                    messages.append({
                        "msg_id": row.get("msg_id"),
                        "from_userid": from_uid,
                        "to_userid": to_uid,
                        "direction": direction,
                        "msg_type": row.get("msg_type") or "text",
                        "content": row.get("content") or "",
                        "timestamp": row.get("timestamp"),
                        "status": row.get("status") or "",
                    })
                return {
                    "success": True,
                    "data": messages,
                    "count": len(messages),
                    "message": f"最近 {len(messages)} 条消息",
                }

            elif query_type == "potential_users":
                import aiosqlite
                async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
                    db.row_factory = aiosqlite.Row
                    p_cols = await self._table_columns(db, "potential_users")
                    group_title_expr = "group_title" if "group_title" in p_cols else "NULL AS group_title"
                    captured_select = "captured_at" if "captured_at" in p_cols else "NULL AS captured_at"
                    captured_order = "captured_at" if "captured_at" in p_cols else "CURRENT_TIMESTAMP"
                    tenant_sql = "WHERE tenant_id = ?" if tenant_id and "tenant_id" in p_cols else ""
                    sql_params = (tenant_id, max(1, int(limit or 10))) if tenant_sql else (max(1, int(limit or 10)),)
                    cursor = await db.execute(f"""
                        SELECT user_id, username, first_name, group_id, {group_title_expr},
                               profile_tags, keywords, {captured_select}
                        FROM potential_users
                        {tenant_sql}
                        ORDER BY COALESCE({captured_order}, CURRENT_TIMESTAMP) DESC LIMIT ?
                    """, sql_params)
                    potentials = [dict(row) for row in await cursor.fetchall()]
                return {
                    "success": True,
                    "data": potentials,
                    "count": len(potentials),
                    "message": f"潜在用户列表 (前{len(potentials)}个)",
                }

            elif query_type == "group_ad_messages":
                import aiosqlite
                lim = max(1, min(int(limit or 10), 50))
                gid = int(group_id) if str(group_id or "").strip() else None
                if gid is None and str(group_name or "").strip():
                    resolved = await self._resolve_group_name(tenant_id=tenant_id, group_name=str(group_name))
                    if resolved.get("status") == "ok":
                        gid = int(resolved["group_id"])
                    else:
                        return {
                            "success": True,
                            "source": "sql.tracked_groups",
                            "data_complete": False,
                            "analysis_allowed": False,
                            "evidence_count": 0,
                            "data": resolved,
                            "count": len(resolved.get("matches") or []),
                            "message": "群名称无法唯一解析，不能继续判断广告消息",
                        }
                where_sql = "WHERE COALESCE(is_advertising, 0) = 1"
                params_sql: List[Any] = []
                if gid is not None:
                    where_sql += " AND group_id = ?"
                    params_sql.append(gid)

                async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
                    db.row_factory = aiosqlite.Row
                    schema_cur = await db.execute("PRAGMA table_info(group_messages)")
                    gm_cols = {row[1] for row in await schema_cur.fetchall()}
                    if "is_advertising" not in gm_cols:
                        return {
                            "success": True,
                            "data": [],
                            "count": 0,
                            "message": "当前数据库尚未启用广告标记字段",
                        }
                    if tenant_id and "tenant_id" in gm_cols:
                        where_sql += " AND tenant_id = ?"
                        params_sql.append(tenant_id)

                    cursor = await db.execute(
                        f"""
                        SELECT msg_id, group_id, tg_msg_id, user_id, username, content,
                               msg_type, sender_type, message_ts, timestamp,
                               COALESCE(is_advertising, 0) AS is_advertising,
                               COALESCE(ad_score, 0) AS ad_score,
                               COALESCE(ad_confidence, 'low') AS ad_confidence,
                               ad_findings
                        FROM group_messages
                        {where_sql}
                        ORDER BY COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) DESC, msg_id DESC
                        LIMIT ?
                        """,
                        (*params_sql, lim),
                    )
                    rows = [dict(row) for row in await cursor.fetchall()]

                for row in rows:
                    raw_findings = row.get("ad_findings")
                    if isinstance(raw_findings, str) and raw_findings.strip():
                        try:
                            row["ad_findings"] = json.loads(raw_findings)
                        except Exception:
                            pass
                    live_text = str(row.get("content") or "").strip()
                    if live_text:
                        try:
                            live_flags = detect_advertising_signals(text=live_text, context_type="group_message")
                            row["live_is_advertising"] = bool(live_flags.get("is_advertising"))
                            row["live_ad_score"] = int(live_flags.get("ad_score", 0) or 0)
                            row["live_ad_findings"] = live_flags.get("findings") or []
                        except Exception:
                            pass

                if not rows and gid is not None:
                    recent = await self._query_group_message_rows(
                        tenant_id=tenant_id,
                        limit=max(lim, 50),
                        group_id=gid,
                        channel_only=False,
                    )
                    candidate_rows = recent.get("data") or []
                    rows = [
                        row for row in candidate_rows
                        if row.get("live_is_advertising") or int(row.get("live_ad_score") or 0) >= 40
                    ][:lim]

                return {
                    "success": True,
                    "source": "sql.group_messages",
                    "data_complete": len(rows) > 0,
                    "analysis_allowed": len(rows) > 0,
                    "evidence_count": len(rows),
                    "summary": {
                        "live_detection_applied": True,
                        "matched_live_ads": len(rows),
                        "group_id": gid,
                    },
                    "limitation": None if rows else "没有已标记或实时命中的广告消息证据；不能据此判断群内不存在广告。",
                    "data": rows,
                    "count": len(rows),
                    "message": f"广告消息列表 (前{len(rows)}条，含实时检测)" if rows else "未查询到已标记或实时命中的广告消息",
                }

            else:
                return {"success": False, "message": f"未知查询类型: {query_type}"}
                
        except Exception as e:
            logger.error(f"SystemQueryTool 执行失败: {e}")
            return {"success": False, "message": f"系统查询错误: {e}"}


# ===== 工具 5b：Telegram SDK 官方接口查询 =====

class TelegramSdkQueryTool(BaseTool):
    """通过 Telegram Bot API 查询官方可见的群/频道元数据与机器人权限。"""
    name = "telegram_sdk_query"
    description = (
        "查询 Telegram Bot API 官方可见信息：getChat、getChatMember、"
        "getChatMemberCount、getChatAdministrators、forwardMessage 存在性探测。"
        "注意：Bot API 不支持按 chat_id 拉取任意历史消息。"
    )
    category = "system"

    @property
    def parameters(self) -> dict:
        return {
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": [
                        "chat_capabilities",
                        "bound_chat_capabilities",
                        "bot_visibility_status",
                        "message_existence_probe",
                    ],
                    "description": "SDK 查询类型"
                },
                "chat_id": {"type": "integer", "description": "Telegram chat_id，可选"},
                "group_id": {"type": "integer", "description": "兼容字段，等同 chat_id"},
                "tg_msg_id": {"type": "integer", "description": "需要探测的 Telegram 消息 ID"},
                "staging_chat_id": {"type": "integer", "description": "forwardMessage 存在性探测使用的校对群 ID"},
                "limit": {"type": "integer", "description": "批量查询数量限制，默认10"},
                "include_admins": {"type": "boolean", "description": "是否拉取管理员列表，默认 true"},
            },
            "required": ["operation"]
        }

    def _tenant_id_from_context(self, context) -> Optional[str]:
        return _context_scoped_tenant_id(context)

    def _get_bot(self, context):
        bot = getattr(context, "bot_instance", None) if context is not None else None
        if bot:
            return bot
        try:
            import web_admin
            return web_admin._bot_instance
        except Exception:
            return None

    def _obj_to_dict(self, obj: Any) -> Dict[str, Any]:
        if obj is None:
            return {}
        if isinstance(obj, dict):
            return obj
        if hasattr(obj, "to_dict"):
            try:
                return obj.to_dict()
            except Exception:
                pass
        out: Dict[str, Any] = {}
        for key in (
            "id", "type", "title", "username", "first_name", "last_name",
            "status", "custom_title", "can_be_edited", "can_manage_chat",
            "can_delete_messages", "can_manage_video_chats", "can_restrict_members",
            "can_promote_members", "can_change_info", "can_invite_users",
            "can_post_messages", "can_edit_messages", "can_pin_messages",
            "can_manage_topics", "is_anonymous",
        ):
            if hasattr(obj, key):
                value = getattr(obj, key)
                if not callable(value):
                    out[key] = value
        return out

    def _bot_member_permissions(self, member: Any) -> Dict[str, Any]:
        data = self._obj_to_dict(member)
        status = data.get("status") or getattr(member, "status", None)
        permissions = {"status": status}
        for key in (
            "can_manage_chat", "can_delete_messages", "can_manage_video_chats",
            "can_restrict_members", "can_promote_members", "can_change_info",
            "can_invite_users", "can_post_messages", "can_edit_messages",
            "can_pin_messages", "can_manage_topics",
        ):
            if key in data:
                permissions[key] = bool(data.get(key))
            elif hasattr(member, key):
                permissions[key] = bool(getattr(member, key))
        return permissions

    async def _target_chats(self, tenant_id: Optional[str], limit: int) -> List[Dict[str, Any]]:
        groups = await database.get_tracked_groups_with_stats(tenant_id=tenant_id)
        targets = await database.get_broadcast_targets(enabled_only=False, tenant_id=tenant_id)
        by_id: Dict[int, Dict[str, Any]] = {}
        for group in groups:
            try:
                chat_id = int(group.get("group_id"))
            except Exception:
                continue
            by_id[chat_id] = {
                "chat_id": chat_id,
                "title": group.get("group_title") or group.get("title") or "",
                "chat_type": "group",
                "tenant_id": group.get("tenant_id"),
                "source": "tracked_groups",
            }
        for target in targets:
            try:
                chat_id = int(target.get("chat_id"))
            except Exception:
                continue
            by_id.setdefault(chat_id, {
                "chat_id": chat_id,
                "title": target.get("chat_title") or "",
                "chat_type": target.get("chat_type") or "",
                "tenant_id": target.get("tenant_id"),
                "source": "broadcast_targets",
            })
        return list(by_id.values())[:max(1, min(int(limit or 10), 50))]

    async def _inspect_chat(self, bot: Any, chat_id: int, tenant_id: Optional[str], include_admins: bool) -> Dict[str, Any]:
        item: Dict[str, Any] = {
            "chat_id": int(chat_id),
            "source": "telegram_bot_api",
            "history_fetch_supported": False,
            "history_fetch_note": "Telegram Bot API 不支持按 chat_id 拉取任意历史消息；消息正文只能来自 bot 实际收到并落库的 updates。",
            "errors": [],
        }
        chat_obj = None
        try:
            chat_obj = await bot.get_chat(chat_id)
            chat = self._obj_to_dict(chat_obj)
            item.update({
                "chat_type": chat.get("type") or getattr(chat_obj, "type", None),
                "title": chat.get("title") or getattr(chat_obj, "title", None),
                "username": chat.get("username") or getattr(chat_obj, "username", None),
            })
        except Exception as exc:
            item["errors"].append(f"getChat: {str(exc)[:200]}")

        bot_user = None
        try:
            bot_user = await bot.get_me()
            item["bot_id"] = getattr(bot_user, "id", None) or self._obj_to_dict(bot_user).get("id")
            item["bot_username"] = getattr(bot_user, "username", None) or self._obj_to_dict(bot_user).get("username")
        except Exception as exc:
            item["errors"].append(f"getMe: {str(exc)[:200]}")

        if item.get("bot_id"):
            try:
                member = await bot.get_chat_member(chat_id=chat_id, user_id=int(item["bot_id"]))
                item["bot_status"] = getattr(member, "status", None) or self._obj_to_dict(member).get("status")
                item["bot_permissions"] = self._bot_member_permissions(member)
            except Exception as exc:
                item["errors"].append(f"getChatMember(bot): {str(exc)[:200]}")

        try:
            item["member_count"] = await bot.get_chat_member_count(chat_id)
        except Exception as exc:
            item["errors"].append(f"getChatMemberCount: {str(exc)[:200]}")

        admin_count = None
        if include_admins:
            try:
                admins = await bot.get_chat_administrators(chat_id)
                admin_count = len(admins or [])
                item["admin_count"] = admin_count
                item["admins"] = [
                    {
                        "user_id": self._obj_to_dict(getattr(a, "user", None)).get("id"),
                        "username": self._obj_to_dict(getattr(a, "user", None)).get("username"),
                        "status": getattr(a, "status", None) or self._obj_to_dict(a).get("status"),
                    }
                    for a in (admins or [])[:20]
                ]
            except Exception as exc:
                item["errors"].append(f"getChatAdministrators: {str(exc)[:200]}")

        try:
            await database.upsert_telegram_chat_capability_snapshot(
                chat_id=int(chat_id),
                tenant_id=tenant_id,
                chat_type=item.get("chat_type"),
                title=item.get("title"),
                username=item.get("username"),
                member_count=item.get("member_count"),
                admin_count=admin_count if admin_count is not None else item.get("admin_count"),
                bot_status=item.get("bot_status"),
                bot_permissions=item.get("bot_permissions") or {},
                history_fetch_supported=False,
                last_error="; ".join(item.get("errors") or []) or None,
            )
        except Exception as exc:
            item["snapshot_error"] = str(exc)[:200]

        item["success"] = not bool(item.get("errors"))
        item["data_complete"] = bool(item.get("bot_status")) and not bool(item.get("errors"))
        return item

    async def _message_existence_probe(self, bot: Any, params: Dict[str, Any], tenant_id: Optional[str]) -> dict:
        chat_id = int(params.get("chat_id") or params.get("group_id") or 0)
        tg_msg_id = int(params.get("tg_msg_id") or 0)
        staging_chat_id = params.get("staging_chat_id")
        if not staging_chat_id and chat_id:
            try:
                staging_chat_id = await database.get_group_revoke_sync_chat_id(chat_id)
            except Exception:
                staging_chat_id = None
        if not chat_id or not tg_msg_id:
            return {"success": False, "message": "message_existence_probe 需要 chat_id/group_id 和 tg_msg_id"}
        if not staging_chat_id:
            return {
                "success": False,
                "message": "缺少 staging_chat_id 或群组未配置校对信息群，无法用 forwardMessage 探测消息是否仍存在",
                "data": {"chat_id": chat_id, "tg_msg_id": tg_msg_id, "source": "telegram_bot_api"},
            }
        try:
            forwarded = await bot.forward_message(
                chat_id=int(staging_chat_id),
                from_chat_id=chat_id,
                message_id=tg_msg_id,
            )
            try:
                await bot.delete_message(chat_id=int(staging_chat_id), message_id=forwarded.message_id)
            except Exception:
                pass
            return {
                "success": True,
                "data_complete": True,
                "evidence_count": 1,
                "data": {
                    "chat_id": chat_id,
                    "tg_msg_id": tg_msg_id,
                    "exists": True,
                    "source": "telegram_bot_api.forwardMessage",
                },
                "message": "消息存在性探测成功：该消息可被 forwardMessage 读取",
            }
        except Exception as exc:
            return {
                "success": True,
                "data_complete": True,
                "evidence_count": 1,
                "data": {
                    "chat_id": chat_id,
                    "tg_msg_id": tg_msg_id,
                    "exists": False,
                    "source": "telegram_bot_api.forwardMessage",
                    "error": str(exc)[:300],
                },
                "message": "消息存在性探测完成：该消息无法被 forwardMessage 读取，可能已删除或 bot 无权限",
            }

    async def execute(self, params: Dict[str, Any], context=None) -> dict:
        operation = params.get("operation") or "chat_capabilities"
        tenant_id = self._tenant_id_from_context(context)
        limit = max(1, min(int(params.get("limit") or 10), 50))
        include_admins = bool(params.get("include_admins", True))
        bot = self._get_bot(context)
        if not bot:
            return {
                "success": False,
                "message": "bot 未初始化，无法调用 Telegram 官方 SDK",
                "data": {"source": "telegram_bot_api", "history_fetch_supported": False},
            }

        try:
            if operation == "message_existence_probe":
                return await self._message_existence_probe(bot, params, tenant_id)

            explicit_chat_id = params.get("chat_id") or params.get("group_id")
            if explicit_chat_id:
                rows = [{"chat_id": int(explicit_chat_id)}]
            else:
                rows = await self._target_chats(tenant_id, limit)

            inspected = []
            for row in rows:
                inspected.append(await self._inspect_chat(bot, int(row["chat_id"]), tenant_id, include_admins))

            visibility_notes = []
            if operation == "bot_visibility_status":
                visibility_notes.append("Bot API 没有拉取任意群/频道历史消息的方法。")
                visibility_notes.append("能分析的消息必须是 bot 已收到 update 并写入 SQL 的消息；群 privacy mode、管理员权限和 bot 入群时间都会影响可见性。")

            return {
                "success": True,
                "data_complete": bool(inspected),
                "evidence_count": len(inspected),
                "data": {
                    "operation": operation,
                    "items": inspected,
                    "history_fetch_supported": False,
                    "visibility_notes": visibility_notes,
                },
                "count": len(inspected),
                "message": f"Telegram SDK 查询完成：{len(inspected)} 个聊天；历史消息拉取不受 Bot API 支持",
            }
        except Exception as e:
            logger.error(f"TelegramSdkQueryTool 执行失败: {e}", exc_info=True)
            return {"success": False, "message": f"Telegram SDK 查询错误: {e}"}


# ===== 工具 6：Skill 管理 =====

class SkillManageTool(BaseTool):
    """AI 助手技能管理工具"""
    name = "skill_manage"
    description = (
        "管理 AI 助手技能。支持：列出技能、新增技能、更新技能、启用/停用技能、"
        "导入本地已存在的技能模板。"
    )
    category = "system"

    @property
    def parameters(self) -> dict:
        return {
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "add", "update", "enable", "disable", "import_builtin"],
                    "description": "操作类型"
                },
                "name": {"type": "string", "description": "技能名称"},
                "description": {"type": "string", "description": "技能描述"},
                "prompt_template": {"type": "string", "description": "技能提示词模板"},
            },
            "required": ["action"]
        }

    def _scan_builtin_skills(self) -> List[Dict[str, str]]:
        candidates = _get_project_skill_dirs()
        found: List[Dict[str, str]] = []
        seen = set()

        for base in candidates:
            if not os.path.isdir(base):
                continue
            for name in sorted(os.listdir(base)):
                path = os.path.join(base, name)
                if not os.path.isdir(path):
                    continue
                skill_md = os.path.join(path, "SKILL.md")
                if not os.path.isfile(skill_md):
                    continue
                key = name.strip().lower()
                if not key or key in seen:
                    continue
                seen.add(key)

                desc = "本地技能模板"
                try:
                    with open(skill_md, "r", encoding="utf-8", errors="ignore") as f:
                        head = f.read(2000)
                    for line in head.splitlines():
                        s = line.strip()
                        if s.lower().startswith("description:"):
                            desc = s.split(":", 1)[1].strip().strip('"')
                            break
                except Exception:
                    pass

                found.append({"name": name, "description": desc, "source": "builtin_fs"})
        return found

    async def execute(self, params: Dict[str, Any], context=None) -> dict:
        action = (params.get("action") or "list").strip().lower()
        name = (params.get("name") or "").strip()
        description = (params.get("description") or "").strip()
        prompt_template = params.get("prompt_template") or ""

        try:
            if action == "list":
                rows = await database.list_ai_custom_skills(enabled_only=False)
                rows = [r for r in rows if str(r.get("source") or "").strip().lower() != "builtin_fs"]
                return {"success": True, "data": rows, "count": len(rows), "message": f"共 {len(rows)} 个技能"}

            if action in ("add", "update"):
                if not name:
                    return {"success": False, "message": "缺少技能名称 name"}
                ok = await database.upsert_ai_custom_skill(
                    name=name,
                    description=description,
                    prompt_template=prompt_template,
                    source="custom",
                    is_enabled=True,
                )
                return {"success": bool(ok), "action_taken": bool(ok), "message": f"技能已{'新增' if action == 'add' else '更新'}: {name}"}

            if action in ("enable", "disable"):
                if not name:
                    return {"success": False, "message": "缺少技能名称 name"}
                ok = await database.set_ai_custom_skill_enabled(name=name, is_enabled=(action == "enable"))
                return {"success": bool(ok), "action_taken": bool(ok), "message": f"技能已{'启用' if action == 'enable' else '停用'}: {name}"}

            if action == "import_builtin":
                builtins = self._scan_builtin_skills()
                imported = 0
                for s in builtins:
                    ok = await database.upsert_ai_custom_skill(
                        name=s["name"],
                        description=s.get("description", ""),
                        prompt_template=f"引用本地技能: {s['name']}",
                        source=s.get("source", "builtin_fs"),
                        is_enabled=True,
                    )
                    if ok:
                        imported += 1
                return {
                    "success": True,
                    "action_taken": imported > 0,
                    "data": {"imported": imported, "detected": len(builtins)},
                    "message": f"已导入 {imported}/{len(builtins)} 个本地技能",
                }

            return {"success": False, "message": f"未知 action: {action}"}
        except Exception as e:
            logger.error(f"SkillManageTool 执行失败: {e}")
            return {"success": False, "message": f"技能管理错误: {e}"}


# ===== 工具注册表 =====

TOOL_REGISTRY: Dict[str, BaseTool] = {}

def register_tool(tool: BaseTool):
    """注册工具"""
    TOOL_REGISTRY[tool.name] = tool
    return tool

def get_tool(name: str) -> Optional[BaseTool]:
    """按名称获取工具"""
    return TOOL_REGISTRY.get(name)

def get_all_tools() -> List[BaseTool]:
    """获取所有已注册的工具"""
    return list(TOOL_REGISTRY.values())

def get_tools_for_llm() -> List[dict]:
    """获取所有工具的 LLM function calling schema"""
    return [t.schema_for_llm for t in TOOL_REGISTRY.values()]

# 注册内置工具
register_tool(FinanceTool())
register_tool(BroadcastTool())
register_tool(ContentGeneratorTool())
register_tool(AdDetectorTool())
register_tool(QueryRecentAdMessagesTool())
register_tool(DeleteRecentAdMessagesTool())
register_tool(DeleteAdMessageTool())
register_tool(QueryAdClustersTool())
register_tool(DeleteAdClusterTool())
register_tool(SystemQueryTool())
register_tool(TelegramSdkQueryTool())
register_tool(SkillManageTool())
