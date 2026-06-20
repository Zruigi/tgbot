# -*- coding: utf-8 -*-
"""
Web 管理端认证与会话状态（OTP / CSRF / 加密入口 / 登录限流 / Token 签发时间）。

生产环境由 ``web_admin.start_web_server`` 使用；与 ``backend.app.api.auth`` 共用同一套内存状态，
避免两套 OTP 存储不一致。
"""
from __future__ import annotations

import hashlib
import logging
import os
import secrets as _secrets
import time
from typing import Dict

from aiohttp import web
from cachetools import TTLCache

import config

logger = logging.getLogger(__name__)

ALLOW_LOCALHOST_BYPASS = os.getenv("ALLOW_LOCALHOST_BYPASS", "false").lower() == "true"

# --- 状态（进程内单例）---
_otp_store: Dict[str, float] = {}
_login_rate_store: Dict[str, list] = {}
_csrf_token_store: Dict[str, float] = {}
_entry_route_store: Dict[str, float] = {}
# Token 签发记录：TTL 7天，上限 500 条
_token_issued_store: TTLCache = TTLCache(maxsize=500, ttl=7*86400)

ENTRY_ROUTE_TTL_SECONDS = int(os.getenv("ENTRY_ROUTE_TTL_SECONDS", "120"))


def generate_otp() -> str:
    """生成一个一次性登录票据 (OTP)，有效期见 SecurityConfig.OTP_EXPIRY_SECONDS。"""
    now = time.time()
    expired = [k for k, v in _otp_store.items() if v < now]
    for k in expired:
        _otp_store.pop(k, None)
    otp = _secrets.token_urlsafe(24)
    _otp_store[otp] = now + config.SecurityConfig.OTP_EXPIRY_SECONDS
    return otp


def generate_secure_entry_route() -> str:
    """生成短时有效的加密入口路由 token。"""
    now = time.time()
    expired = [k for k, v in _entry_route_store.items() if v < now]
    for k in expired:
        _entry_route_store.pop(k, None)

    token = _secrets.token_urlsafe(18)
    _entry_route_store[token] = now + ENTRY_ROUTE_TTL_SECONDS
    return token


def consume_secure_entry_route(token: str) -> bool:
    """消费一次性入口路由 token，防重放。"""
    if not token:
        return False

    now = time.time()
    expire = _entry_route_store.get(token)
    if not expire or expire < now:
        _entry_route_store.pop(token, None)
        return False

    _entry_route_store.pop(token, None)
    return True


def generate_csrf_token() -> str:
    """生成 CSRF Token，有效期 24 小时。"""
    now = time.time()
    expired = [k for k, v in _csrf_token_store.items() if v < now]
    for k in expired:
        _csrf_token_store.pop(k, None)
    csrf_token = _secrets.token_urlsafe(32)
    _csrf_token_store[csrf_token] = now + 86400
    return csrf_token


def check_login_rate_limit(ip: str) -> bool:
    """登录速率限制：返回 True 表示允许。"""
    now = time.time()
    window = config.SecurityConfig.RATE_LIMIT_WINDOW
    max_attempts = config.SecurityConfig.MAX_LOGIN_ATTEMPTS
    attempts = _login_rate_store.get(ip, [])
    attempts = [t for t in attempts if now - t < window]
    if len(attempts) >= max_attempts:
        return False
    attempts.append(now)
    _login_rate_store[ip] = attempts
    return True


def issue_token(token: str) -> None:
    """记录 Token 签发时间，用于过期检查。"""
    key = hashlib.sha256(token.encode()).hexdigest()[:16]
    _token_issued_store[key] = time.time()


def is_token_expired(token: str) -> bool:
    """检查 Token 是否已超过配置天数。"""
    key = hashlib.sha256(token.encode()).hexdigest()[:16]
    issued_at = _token_issued_store.get(key)
    if issued_at is None:
        return False
    expiry_seconds = config.SecurityConfig.TOKEN_EXPIRY_DAYS * 86400
    return (time.time() - issued_at) > expiry_seconds


def validate_and_consume_otp(otp: str) -> bool:
    """校验并消费 OTP；成功返回 True。"""
    now = time.time()
    expire = _otp_store.get(otp)
    if not expire or expire < now:
        _otp_store.pop(otp, None)
        return False
    _otp_store.pop(otp, None)
    return True


@web.middleware
async def auth_middleware(request, handler):
    """
    Bearer Token + CSRF + 安全响应头（与历史 web_admin 行为一致）。
    """
    path = request.path
    is_legacy_static = (
        path.startswith('/static/js/') and not path.startswith('/static/js/vendor/')
    ) or path.startswith('/static/css/viewer')
    if not is_legacy_static and (
        path in ['/', '/ws']
        or path.startswith('/e/')
        or path.startswith('/v1/')
        or path in ['/api/login', '/api/login/otp', '/api/csrf-token']
        or path.startswith('/api/file/')
        or path.startswith('/static/')
    ):
        return await handler(request)

    auth_header = request.headers.get('Authorization')
    cookie_token = request.cookies.get('admin_token')

    token = None
    if auth_header and auth_header.startswith('Bearer '):
        token = auth_header.split(' ')[1]
    elif cookie_token:
        token = cookie_token

    token_valid = False
    if token:
        token_valid = config.verify_admin_token(token)
        if token_valid and is_token_expired(token):
            logger.warning(f"过期 Token 被拒绝: 来自 {request.remote}")
            token_valid = False

    if not token_valid:
        if ALLOW_LOCALHOST_BYPASS and (request.remote == '127.0.0.1' or request.host.startswith('localhost:')):
            token_valid = True
            request['session'] = {
                'role': 'platform_admin',
                'tenant_id': '',
                'merchant_id': None,
                'authenticated': True,
                'scope_type': 'platform',
            }
            request['role'] = 'platform_admin'
            request['tenant_id'] = ''
            request['merchant_id'] = None
            request['_require_tenant_filter'] = False

    if not token_valid:
        logger.warning(f"拒绝未授权访问: {request.path} 来自 {request.remote}")
        return web.json_response({"status": "error", "message": "Unauthorized"}, status=401)

    is_local = request.remote == '127.0.0.1' or request.host.startswith('localhost:')
    if not is_local and request.method in ('POST', 'DELETE', 'PATCH', 'PUT'):
        skip_csrf_paths = [
            '/api/login', '/api/login/otp', '/api/messages/',
        ]
        need_csrf = not any(path.startswith(p) for p in skip_csrf_paths)
        if need_csrf:
            csrf_header = request.headers.get('X-CSRF-Token', '')
            now = time.time()
            expire = _csrf_token_store.get(csrf_header)
            if not csrf_header or not expire or expire < now:
                logger.warning(f"CSRF 校验失败: {path} 来自 {request.remote}")
                return web.json_response({"status": "error", "message": "CSRF token 无效"}, status=403)

    response = await handler(request)
    response.headers['X-Content-Type-Options'] = 'nosniff'
    response.headers['X-Frame-Options'] = 'DENY'
    response.headers['X-XSS-Protection'] = '1; mode=block'
    response.headers['Referrer-Policy'] = 'strict-origin-when-cross-origin'
    response.headers['Content-Security-Policy'] = (
        "default-src 'self'; "
        "script-src 'self' 'unsafe-inline' 'unsafe-eval' https://cdn.jsdelivr.net https://unpkg.com; "
        "style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net https://unpkg.com; "
        "img-src 'self' data: blob: https:; "
        "connect-src 'self' wss: ws:; "
        "font-src 'self' https://cdn.jsdelivr.net; "
        "report-uri /api/csp-report"
    )
    return response
