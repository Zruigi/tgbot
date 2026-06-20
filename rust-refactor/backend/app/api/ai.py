# -*- coding: utf-8 -*-
"""
AI API - Agent接口
"""
from fastapi import APIRouter, Depends
from pydantic import BaseModel
from typing import List, Optional
from backend.app.agent.core import AIAgent, AgentContext, AgentRole, PermissionLevel, get_agent
from backend.app.api.auth import verify_token, TokenData

router = APIRouter()


class ChatRequest(BaseModel):
    """AI对话请求"""
    message: str
    model: str = "deepseek-chat"
    conversation_history: Optional[List[dict]] = []


class ChatResponse(BaseModel):
    """AI对话响应"""
    reply: str
    tool_calls: List[str] = []
    action_taken: bool = False


# LLM调用函数 (通过LiteLLM)
async def llm_call_fn(messages, model):
    import aiohttp
    url = f"{settings.LITELLM_BASE_URL}/chat/completions"
    headers = {"Authorization": f"Bearer {settings.LITELLM_API_KEY}", "Content-Type": "application/json"}
    payload = {"model": model, "messages": messages}

    async with aiohttp.ClientSession() as session:
        async with session.post(url, json=payload, headers=headers) as resp:
            data = await resp.json()
            if "choices" in data and data["choices"]:
                return data["choices"][0]["message"].get("content", ""), data.get("usage", {})
            return str(data), {}


# 工具执行函数
async def tool_executor_fn(tool_name: str, params: dict, ctx: AgentContext):
    """工具执行器"""
    results = {
        "ad_detect": {"is_advertising": False, "ad_score": 0, "ad_type": {}},
        "ad_register": {"success": True, "ad_id": 123},
        "ad_delete": {"success": True},
        "ad_beautify": {"beautified": params.get("original_content", "")},
        "ad_push_channel": {"success": True, "message_id": 456},
        "group_ban": {"success": True},
        "translate": {"translated": params.get("text", "")},
        "system_query": {"stats": {"users": 100, "groups": 10}}
    }
    return results.get(tool_name, {"error": "unknown tool"})


@router.post("/chat", response_model=ChatResponse)
async def chat(
    request: ChatRequest,
    token_data: TokenData = Depends(verify_token)
):
    """
    AI对话接口 - 多商户隔离
    """
    # 创建Agent实例
    agent = get_agent(llm_call_fn=llm_call_fn, tool_executor_fn=tool_executor_fn)

    # 构建上下文
    ctx = AgentContext(
        user_id=token_data.user_id,
        merchant_id=token_data.merchant_id,
        tenant_id=token_data.tenant_id,
        role=AgentRole.ADMIN if token_data.user_id else AgentRole.USER,
        permission=PermissionLevel.DANGEROUS_WRITE,
        conversation_history=request.conversation_history
    )

    # 处理消息
    response = await agent.process_message(request.message, ctx, request.model)

    return ChatResponse(
        reply=response.text,
        tool_calls=response.tool_calls_made,
        action_taken=response.is_action_taken
    )


@router.post("/detect-ad")
async def detect_advertising(text: str, context: str = "group_message"):
    """
    广告检测接口 - 不需要认证(供Bot调用)
    """
    # 创建简化的Agent进行检测
    agent = get_agent(llm_call_fn=llm_call_fn)

    ctx = AgentContext(
        user_id=0,
        merchant_id="system",
        tenant_id="system"
    )

    response = await agent.process_message(f"检测这段内容是否为广告: {text}", ctx)

    return {"is_advertising": "ad" in response.text.lower(), "score": 50, "response": response.text}


@router.get("/tools")
async def list_tools():
    """列出可用工具"""
    return {
        "tools": [
            {"name": "ad_detect", "description": "检测广告"},
            {"name": "ad_register", "description": "登记广告"},
            {"name": "ad_delete", "description": "删除广告"},
            {"name": "ad_beautify", "description": "美化广告"},
            {"name": "ad_push_channel", "description": "推送到频道"},
            {"name": "group_ban", "description": "踢人/禁言"},
            {"name": "translate", "description": "翻译"},
            {"name": "system_query", "description": "���统查询"}
        ]
    }