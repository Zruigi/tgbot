# -*- coding: utf-8 -*-
"""
私聊消息相关的 Pydantic Schema
"""
from pydantic import BaseModel, Field
from typing import Optional


class SendMessageRequest(BaseModel):
    """发送消息请求"""
    text: str = Field(..., description="消息文本")
    reply_to_message_id: Optional[int] = Field(None, description="回复的消息 ID")


class DeleteMessageRequest(BaseModel):
    """删除消息请求"""
    message_id: int = Field(..., description="要删除的消息 ID")


class MarkReadRequest(BaseModel):
    """标记已读请求"""
    up_to_message_id: int = Field(..., description="标记到此消息为止")


class SearchMessagesRequest(BaseModel):
    """搜索消息请求"""
    query: str = Field(..., description="搜索关键词")
    limit: int = Field(default=20, ge=1, le=100, description="返回数量限制")
