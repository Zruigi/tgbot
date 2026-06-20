# -*- coding: utf-8 -*-
"""
核心配置 - 多商户高并发架构
"""
import os
from typing import Optional
from pydantic_settings import BaseSettings
from pydantic import Field


class Settings(BaseSettings):
    """应用配置"""
    
    # === 应用基础 ===
    APP_NAME: str = TGBot Pro
    APP_VERSION: str = 2.0.0
    DEBUG: bool = Field(default=False, validation_alias='DEBUG')
    
    # === 数据库 ===
    DATABASE_URL: str = Field(
        default=postgresql+asyncpg://tgbot:changeme@localhost:5432/tgbot,
        validation_alias='DATABASE_URL'
    )
    DB_POOL_SIZE: int = 50
    DB_MAX_OVERFLOW: int = 50
    
    # === Redis ===
    REDIS_URL: str = redis://localhost:6379
    REDIS_MAX_CONNECTIONS: int = 50
    
    # === 安全 ===
    SECRET_KEY: str = Field(default=change-me-in-production)
    ALGORITHM: str = HS256
    ACCESS_TOKEN_EXPIRE_MINUTES: int = 60 * 24 * 7  # 7天
    
    # === 外部服务 ===
    LITELLM_BASE_URL: str = http://localhost:4000/v1
    LITELLM_API_KEY: str = sk-local
    PADDLEOCR_URL: str = http://localhost:8000
    
    # === Telegram Bot ===
    # 支持多商户Bot Token (JSON格式)
    BOT_TOKENS: str = Field(default={}, validation_alias='BOT_TOKENS')
    
    # === 多商户配置 ===
    MAX_TENANTS: int = 1000
    TENANT_ISOLATION: str = rls  # row-level-security
    
    # === AI Agent配置 ===
    AGENT_MAX_TOOL_CALLS: int = 5
    AGENT_TIMEOUT_SECONDS: int = 120
    
    class Config:
        env_file = .env
        extra = allow


settings = Settings()


# === 数据库连接池 ===
from sqlalchemy.ext.asyncio import create_async_engine, AsyncSession, async_sessionmaker

engine = create_async_engine(
    settings.DATABASE_URL,
    pool_size=settings.DB_POOL_SIZE,
    max_overflow=settings.DB_MAX_OVERFLOW,
    echo=settings.DEBUG,
    pool_pre_ping=True,
)

AsyncSessionLocal = async_sessionmaker(
    engine,
    class_=AsyncSession,
    expire_on_commit=False,
    autocommit=False,
    autoflush=False,
)


# === Redis连接 ===
import redis.asyncio as redis

redis_client: Optional[redis.Redis] = None


async def get_redis() -> redis.Redis:
    """获取Redis连接"""
    global redis_client
    if redis_client is None:
        redis_client = redis.from_url(
            settings.REDIS_URL,
            max_connections=settings.REDIS_MAX_CONNECTIONS,
            decode_responses=True
        )
    return redis_client


async def close_redis():
    """关闭Redis连接"""
    global redis_client
    if redis_client:
        await redis_client.close()
        redis_client = None
