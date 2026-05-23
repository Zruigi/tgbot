# -*- coding: utf-8 -*-
"""
AI Agent 内置工具集 — 每个工具封装一个系统功能
工具通过 Function Calling 机制被 LLM 调用
"""

import logging
import os
import json
from typing import Any, Dict, List, Optional
import httpx
import database
import config
from .ad_detection import detect_advertising_signals

logger = logging.getLogger(__name__)
_PROJECT_ROOT = os.path.realpath(os.path.join(os.path.dirname(__file__), "..", ".."))


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
        
        for s in summary or []:
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


# ===== 工具 5：系统查询 =====

class SystemQueryTool(BaseTool):
    """系统状态与配置查询工具"""
    name = "system_query"
    description = (
        "查询系统状态和数据统计。支持：在线用户数、监听地址数、群组列表、"
        "最近消息统计、系统运行时间等。"
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
                        "active_groups",       # 活跃群组
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
            },
            "required": ["query_type"]
        }

    async def execute(self, params: Dict[str, Any], context=None) -> dict:
        query_type = params.get("query_type", "stats_overview")
        limit = params.get("limit", 10)
        group_id = params.get("group_id")

        try:
            if query_type == "stats_overview":
                # 多维度聚合统计
                groups = await database.get_tracked_groups_with_stats()
                users = await database.get_inbox_list()

                import aiosqlite
                async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
                    cursor = await db.execute(
                        "SELECT COUNT(*) FROM address_monitors WHERE is_active=1"
                    )
                    monitor_count = (await cursor.fetchone())[0]

                return {
                    "success": True,
                    "data": {
                        "group_count": len(groups),
                        "user_count": len([u for u in users if u.get('from_userid')]),
                        "monitor_count": monitor_count,
                        "ai_enabled": True,
                    },
                    "message": "系统统计概览",
                }

            elif query_type == "active_groups":
                groups = await database.get_tracked_groups_with_stats()
                return {
                    "success": True,
                    "data": groups[:limit],
                    "count": len(groups),
                    "message": f"活跃群组列表 (前{min(limit, len(groups))}个)",
                }

            elif query_type == "monitors_list":
                import aiosqlite
                async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
                    db.row_factory = aiosqlite.Row
                    c = await db.execute("PRAGMA table_info(address_monitors)")
                    cols = {row[1] for row in await c.fetchall()}
                    inflow_expr = "inflow_20" if "inflow_20" in cols else "0 AS inflow_20"
                    outflow_expr = "outflow_20" if "outflow_20" in cols else "0 AS outflow_20"
                    profit_expr = "profit_20" if "profit_20" in cols else "0 AS profit_20"

                    cursor = await db.execute(
                        f"""
                        SELECT monitor_id, address, network, remark,
                               {inflow_expr}, {outflow_expr}, {profit_expr}
                        FROM address_monitors WHERE is_active=1
                        ORDER BY monitor_id DESC LIMIT ?
                    """,
                        (limit,),
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
                inbox_rows = await database.get_inbox_list()
                ai_uid = int(os.getenv("AI_ASSISTANT_USER_ID", "900000001"))

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
                    cursor = await db.execute(
                        """
                        SELECT msg_id, from_userid, to_userid, content, type AS msg_type, timestamp, status
                        FROM messages
                        WHERE is_deleted = 0
                          AND (to_userid = ? OR from_userid = ?)
                        ORDER BY timestamp DESC
                        LIMIT ?
                        """,
                        (config.ADMIN_ID, config.ADMIN_ID, max(1, int(limit or 10))),
                    )
                    rows = [dict(row) for row in await cursor.fetchall()]

                ai_uid = int(os.getenv("AI_ASSISTANT_USER_ID", "900000001"))
                messages = []
                for row in rows:
                    from_uid = int(row.get("from_userid", 0) or 0)
                    to_uid = int(row.get("to_userid", 0) or 0)
                    direction = "inbound" if to_uid == int(config.ADMIN_ID) else "outbound"
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
                    cursor = await db.execute("""
                        SELECT user_id, username, source_group, profile_tags, 
                               first_seen, last_active
                        FROM potential_users 
                        ORDER BY last_active DESC LIMIT ?
                    """, (limit,))
                    potentials = [dict(row) for row in await cursor.fetchall()]
                return {
                    "success": True,
                    "data": potentials,
                    "count": len(potentials),
                    "message": f"潜在用户列表 (前{len(potentials)}个)",
                }

            elif query_type == "group_ad_messages":
                import aiosqlite
                lim = max(1, int(limit or 10))
                gid = int(group_id) if str(group_id or "").strip() else None
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

                return {
                    "success": True,
                    "data": rows,
                    "count": len(rows),
                    "message": f"广告消息列表 (前{len(rows)}条)",
                }

            else:
                return {"success": False, "message": f"未知查询类型: {query_type}"}
                
        except Exception as e:
            logger.error(f"SystemQueryTool 执行失败: {e}")
            return {"success": False, "message": f"系统查询错误: {e}"}


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
register_tool(SystemQueryTool())
register_tool(SkillManageTool())
