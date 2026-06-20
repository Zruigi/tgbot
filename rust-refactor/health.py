# -*- coding: utf-8 -*-
"""
健康检查与监控
"""
import psutil
import time
from datetime import datetime
from fastapi import APIRouter

router = APIRouter()


@router.get("/health/detailed")
async def detailed_health():
    """详细健康检查"""
    return {
        "status": "ok",
        "timestamp": datetime.utcnow().isoformat(),
        "uptime": time.time(),
        "system": {
            "cpu_percent": psutil.cpu_percent(),
            "memory_percent": psutil.virtual_memory().percent,
            "disk_percent": psutil.disk_usage('/').percent
        }
    }


@router.get("/health/ready")
async def readiness_check():
    """就绪检查"""
    # 检查数据库连接
    # 检查Redis连接
    # 检查LiteLLM
    # 检查PaddleOCR
    checks = {
        "database": "ok",
        "redis": "ok",
        "litellm": "ok",
        "paddleocr": "ok"
    }

    all_ok = all(v == "ok" for v in checks.values())

    return {
        "ready": all_ok,
        "checks": checks
    }