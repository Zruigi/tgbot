# -*- coding: utf-8 -*-
"""
认证 API - 登录、OTP、CSRF、中间件
"""
import time
import secrets
import logging
import os
from aiohttp import web
from datetime import datetime
from typing import Optional, Dict, Any
from cachetools import TTLCache
import config
from backend.repositories.admin_repo import get_admin_setting, save_admin_setting

logger = logging.getLogger(__name__)

# Token 存储
_otp_store: dict = {}
_entry_route_store: dict = {}
_csrf_token_store: dict = {}
_login_rate_store: dict = {}
# Token 签发记录：TTL 7天（与 TOKEN_EXPIRY_DAYS 对齐），上限 500 条
_token_issued_store: TTLCache = TTLCache(maxsize=500, ttl=7*86400)

ENTRY_ROUTE_TTL_SECONDS = int(os.getenv("ENTRY_ROUTE_TTL_SECONDS", "120"))


def generate_otp() -> str:
    """生成一个 5 分钟有效的一次性登录票据 (OTP)"""
    now = time.time()
    expired = [k for k, v in _otp_store.items() if v < now]
    for k in expired:
        _otp_store.pop(k, None)
    otp = secrets.token_urlsafe(24)
    _otp_store[otp] = now + config.SecurityConfig.OTP_EXPIRY_SECONDS
    return otp


def generate_secure_entry_route() -> str:
    """生成短时有效的加密入口路由 token"""
    now = time.time()
    expired = [k for k, v in _entry_route_store.items() if v < now]
    for k in expired:
        _entry_route_store.pop(k, None)

    token = secrets.token_urlsafe(18)
    _entry_route_store[token] = now + ENTRY_ROUTE_TTL_SECONDS
    return token


def consume_secure_entry_route(token: str) -> bool:
    """消费一次性入口路由 token，防重放"""
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
    """生成一个 CSRF Token，有效期 24 小时"""
    now = time.time()
    # 清理过期 token
    expired = [k for k, v in _csrf_token_store.items() if v < now]
    for k in expired:
        _csrf_token_store.pop(k, None)
    csrf_token = secrets.token_urlsafe(32)
    _csrf_token_store[csrf_token] = now + 86400  # 24 小时
    return csrf_token


def _check_login_rate_limit(ip: str) -> bool:
    """检查登录速率限制"""
    now = time.time()
    window = config.SecurityConfig.RATE_LIMIT_WINDOW
    max_attempts = config.SecurityConfig.MAX_LOGIN_ATTEMPTS
    attempts = _login_rate_store.get(ip, [])
    # 只保留窗口内的尝试记录
    attempts = [t for t in attempts if now - t < window]
    if len(attempts) >= max_attempts:
        return False
    attempts.append(now)
    _login_rate_store[ip] = attempts
    return True


async def auth_middleware(app, handler):
    """认证中间件"""
    async def middleware(request):
        # 跳过登录和静态资源
        if request.path in ['/api/login', '/api/login/otp', '/static', '/']:
            return await handler(request)
        
        # 检查 Token
        token = request.headers.get('Authorization', '').replace('Bearer ', '')
        if not token:
            return web.json_response({'status': 'error', 'message': '未授权'}, status=401)
        
        # 验证 Token（简化版，实际应使用 JWT 或 session）
        # 这里暂时只检查是否为空
        
        return await handler(request)
    
    return middleware


async def api_login(request: web.Request) -> web.Response:
    """管理员登录接口"""
    try:
        data = await request.json()
        token = data.get('token', '')
        
        # 速率限制检查
        ip = request.remote or 'unknown'
        if not _check_login_rate_limit(ip):
            return web.json_response(
                {'status': 'error', 'message': '尝试次数过多，请稍后再试'},
                status=429
            )
        
        # 验证 Token
        if token != config.BOT_TOKEN:
            return web.json_response(
                {'status': 'error', 'message': 'Token 无效'},
                status=401
            )
        
        # 生成 OTP
        otp = generate_otp()
        
        # 生成 CSRF Token
        csrf = generate_csrf_token()
        
        return web.json_response({
            'status': 'ok',
            'data': {
                'otp': otp,
                'csrf_token': csrf,
            }
        })
    except Exception as e:
        logger.error(f"登录失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': '登录失败'},
            status=500
        )


async def api_login_otp(request: web.Request) -> web.Response:
    """OTP 一次性票据登录"""
    try:
        data = await request.json()
        otp = data.get('otp', '')
        now = time.time()

        # 优先从 web_admin 的 OTP store 验证（机器人通过 web_admin.generate_otp() 创建的 OTP）
        # 该 store 包含预创建的 HMAC signed session token（内含 role/merchant_id/tenant_id）
        access_token = None
        session_data = None

        try:
            import web_admin as wa
            wa_otp = wa._otp_store.get(otp)
            if wa_otp and isinstance(wa_otp, dict):
                if wa_otp.get('expire_at', 0) >= now:
                    # OTP 有效，取出预创建的 session token
                    access_token = wa_otp.get('token')
                    session_data = {
                        'role': wa_otp.get('role', 'platform_admin'),
                        'tenant_id': wa_otp.get('tenant_id', ''),
                        'merchant_id': wa_otp.get('merchant_id'),
                        'issued_for': wa_otp.get('issued_for', ''),
                    }
                    del wa._otp_store[otp]
        except Exception:
            pass  # web_admin 导入或访问失败，fallback 到旧逻辑

        # Fallback：旧版 auth._otp_store（仅存 expire_at，无 session token）
        if not access_token:
            expire = _otp_store.get(otp)
            if not expire or expire < now:
                _otp_store.pop(otp, None)
                return web.json_response(
                    {'status': 'error', 'message': 'OTP 已过期或无效'},
                    status=401
                )
            _otp_store.pop(otp, None)
            logger.warning("拒绝旧版无会话上下文 OTP：缺少 role/tenant_id/merchant_id")
            return web.json_response(
                {'status': 'error', 'message': '旧版登录票据缺少租户上下文，请重新获取登录链接'},
                status=401
            )

        # 生成 CSRF Token
        csrf = generate_csrf_token()

        result = {
            'status': 'ok',
            'data': {
                # 关键：字段名为 'token'，与前端 loginWithOtp 读取 data.token 保持一致
                'token': access_token,
                'csrf_token': csrf,
            }
        }
        # 如果有 session 数据，一并返回给前端用于即时设置 sessionContext
        if session_data:
            result['data']['session'] = session_data

        return web.json_response(result)
    except Exception as e:
        logger.error(f"OTP 登录失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': '登录失败'},
            status=500
        )


async def api_get_csrf_token(request: web.Request) -> web.Response:
    """获取 CSRF Token"""
    try:
        csrf = generate_csrf_token()
        return web.json_response({
            'status': 'ok',
            'data': {'csrf_token': csrf}
        })
    except Exception as e:
        logger.error(f"获取 CSRF Token 失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': '获取 CSRF Token 失败'},
            status=500
        )


async def api_csp_report(request: web.Request) -> web.Response:
    """CSP 违规报告收集"""
    try:
        report = await request.json()
        logger.warning(f"CSP 违规报告：{report}")
        return web.json_response({'status': 'ok'})
    except Exception as e:
        logger.error(f"CSP 报告处理失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': 'CSP 报告处理失败'},
            status=500
        )
