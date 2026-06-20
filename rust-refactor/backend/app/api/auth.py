# -*- coding: utf-8 -*-
"""
认证API - 多商户JWT鉴权
"""
from datetime import datetime, timedelta
from typing import Optional
from fastapi import APIRouter, Depends, HTTPException, status
from fastapi.security import HTTPBearer, HTTPAuthorizationCredentials
from pydantic import BaseModel, Field
from jose import JWTError, jwt
from backend.app.core.config import settings

router = APIRouter()
security = HTTPBearer()

# JWT 配置
ALGORITHM = "HS256"


class LoginRequest(BaseModel):
    """登录请求"""
    merchant_id: str
    username: str
    password: str


class LoginResponse(BaseModel):
    """登录响应"""
    access_token: str
    token_type: str = "bearer"
    expires_in: int
    merchant_id: str
    user_id: int


class TokenData(BaseModel):
    """Token数据"""
    user_id: int
    merchant_id: str
    tenant_id: str
    exp: datetime


def create_access_token(data: dict, expires_delta: Optional[timedelta] = None) -> str:
    """创建JWT Token"""
    to_encode = data.copy()
    expire = datetime.utcnow() + (expires_delta or timedelta(minutes=settings.ACCESS_TOKEN_EXPIRE_MINUTES))
    to_encode.update({"exp": expire})
    return jwt.encode(to_encode, settings.SECRET_KEY, algorithm=ALGORITHM)


async def verify_token(credentials: HTTPAuthorizationCredentials = Depends(security)) -> TokenData:
    """验证Token"""
    try:
        token = credentials.credentials
        payload = jwt.decode(token, settings.SECRET_KEY, algorithms=[ALGORITHM])
        return TokenData(**payload)
    except JWTError:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="无效的认证凭证",
            headers={"WWW-Authenticate": "Bearer"},
        )


@router.post("/login", response_model=LoginResponse)
async def login(request: LoginRequest):
    """
    商户管理员登录
    - 验证商户Bot Token
    - 返回JWT Token
    """
    # TODO: 实际验证商户和密码
    # 这里简化处理
    if not request.merchant_id or not request.password:
        raise HTTPException(status_code=400, detail="参数不完整")

    # 创建Token
    token_data = {
        "user_id": 123456,  # 实际从数据库获取
        "merchant_id": request.merchant_id,
        "tenant_id": "default",
    }
    access_token = create_access_token(token_data)

    return LoginResponse(
        access_token=access_token,
        expires_in=settings.ACCESS_TOKEN_EXPIRE_MINUTES * 60,
        merchant_id=request.merchant_id,
        user_id=123456
    )


@router.get("/me")
async def get_current_user(token_data: TokenData = Depends(verify_token)):
    """获取当前用户信息"""
    return {
        "user_id": token_data.user_id,
        "merchant_id": token_data.merchant_id,
        "tenant_id": token_data.tenant_id
    }


@router.post("/refresh")
async def refresh_token(token_data: TokenData = Depends(verify_token)):
    """刷新Token"""
    new_token = create_access_token({
        "user_id": token_data.user_id,
        "merchant_id": token_data.merchant_id,
        "tenant_id": token_data.tenant_id,
    })
    return {"access_token": new_token, "token_type": "bearer"}