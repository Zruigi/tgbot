# -*- coding: utf-8 -*-
"""
FastAPI 主入口 - 多商户高并发架构
"""
from contextlib import asynccontextmanager
from fastapi import FastAPI, Request, Depends, HTTPException, status
from fastapi.middleware.cors import CORSMiddleware
from fastapi.staticfiles import StaticFiles
from fastapi.responses import JSONResponse
import logging

from backend.app.core.config import settings, engine, close_redis
from backend.app.models.tenant import Base

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


@asynccontextmanager
async def lifespan(app: FastAPI):
    """应用生命周期管理"""
    # 启动时创建数据库表
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)
    logger.info("数据库表创建完成")

    yield

    # 关闭时清理资源
    await close_redis()
    await engine.dispose()
    logger.info("资源清���完成")


# 创建 FastAPI 应用
app = FastAPI(
    title=settings.APP_NAME,
    version=settings.APP_VERSION,
    lifespan=lifespan,
    docs_url="/docs",
    redoc_url="/redoc"
)

# CORS 中间件
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],  # 生产环境应限制域名
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)


# === 异常处理 ===
@app.exception_handler(HTTPException)
async def http_exception_handler(request: Request, exc: HTTPException):
    return JSONResponse(
        status_code=exc.status_code,
        content={"status": "error", "message": exc.detail}
    )


@app.exception_handler(Exception)
async def general_exception_handler(request: Request, exc: Exception):
    logger.error(f"未处理异常: {exc}", exc_info=True)
    return JSONResponse(
        status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
        content={"status": "error", "message": "服务器内部错误"}
    )


# === 租户上下文 ===
from contextvars import ContextVar
tenant_context: ContextVar[dict] = ContextVar("tenant_context", default={})


def get_current_tenant() -> dict:
    """获取当前租户上下文"""
    return tenant_context.get()


# === 导入路由 ===
from backend.app.api import auth, users, messages, groups, advertisements, ai


# === 注册路由 ===
app.include_router(auth.router, prefix="/api/auth", tags=["认证"])
app.include_router(users.router, prefix="/api/users", tags=["用户"])
app.include_router(messages.router, prefix="/api/messages", tags=["消息"])
app.include_router(groups.router, prefix="/api/groups", tags=["群组"])
app.include_router(advertisements.router, prefix="/api/ads", tags=["广告"])
app.include_router(ai.router, prefix="/api/ai", tags=["AI"])


# === 健康检查 ===
@app.get("/health")
async def health_check():
    return {"status": "ok", "service": settings.APP_NAME, "version": settings.APP_VERSION}


@app.get("/")
async def root():
    return {"message": f"Welcome to {settings.APP_NAME} API"}


# === 静态文件 (前端) ===
app.mount("/static", StaticFiles(directory="static/dist"), name="static")


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="0.0.0.0", port=8080)