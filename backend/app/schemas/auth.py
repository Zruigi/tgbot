# -*- coding: utf-8 -*-
"""
认证相关的 Pydantic Schema
"""
from pydantic import BaseModel, Field


class LoginRequest(BaseModel):
    """登录请求"""
    token: str = Field(..., description="管理员 Token")


class LoginResponse(BaseModel):
    """登录响应"""
    status: str = Field(..., description="状态码")
    data: dict = Field(default_factory=dict, description="响应数据")


class OTPLoginRequest(BaseModel):
    """OTP 登录请求"""
    otp: str = Field(..., description="一次性密码")


class CSRFTokenResponse(BaseModel):
    """CSRF Token 响应"""
    status: str
    data: dict[str, str]
