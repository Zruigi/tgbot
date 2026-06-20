# -*- coding: utf-8 -*-
"""
群组管理相关的 Pydantic Schema
"""
from pydantic import BaseModel, Field
from typing import Optional


class GroupListenToggleRequest(BaseModel):
    """切换群组监听请求"""
    chat_id: int = Field(..., description="群组 ID")
    enabled: bool = Field(..., description="是否启用")


class GroupVerifyConfigRequest(BaseModel):
    """群组验证配置请求"""
    chat_id: int = Field(..., description="群组 ID")
    enable_verify: bool = Field(default=True, description="是否启用验证")
    welcome_message: Optional[str] = Field(None, description="欢迎消息")
    verify_button_text: Optional[str] = Field(None, description="验证按钮文本")


class VerifyStatsResponse(BaseModel):
    """验证统计响应"""
    total_users: int
    verified_users: int
    pending_users: int
