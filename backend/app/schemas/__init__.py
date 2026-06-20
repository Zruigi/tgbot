# Schemas Package - Pydantic 模型定义
from .auth import LoginRequest, LoginResponse, OTPLoginRequest, CSRFTokenResponse
from .chat import SendMessageRequest, DeleteMessageRequest, MarkReadRequest, SearchMessagesRequest
from .group import GroupListenToggleRequest, GroupVerifyConfigRequest, VerifyStatsResponse

__all__ = [
    # Auth
    'LoginRequest',
    'LoginResponse',
    'OTPLoginRequest',
    'CSRFTokenResponse',
    
    # Chat
    'SendMessageRequest',
    'DeleteMessageRequest',
    'MarkReadRequest',
    'SearchMessagesRequest',
    
    # Group
    'GroupListenToggleRequest',
    'GroupVerifyConfigRequest',
    'VerifyStatsResponse',
]
