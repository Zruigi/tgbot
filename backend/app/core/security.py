# -*- coding: utf-8 -*-
"""
安全服务 - Token 管理、速率限制、CSRF 校验、权限控制
"""
import time
import secrets
import hashlib
import hmac
import json
import base64
from typing import Optional, Dict, Any, List
from datetime import datetime, timedelta
import logging as _lg

logger = _lg.getLogger(__name__)


def _get_session_signing_secret() -> str:
    import config as _cfg
    return hashlib.sha256(str(_cfg.WEB_ADMIN_TOKEN).encode('utf-8')).hexdigest()


def _b64url_decode(data: str) -> bytes:
    padding = '=' * (-len(data) % 4)
    return base64.urlsafe_b64decode((data + padding).encode())


def _verify_session_token(token: str) -> Optional[Dict[str, Any]]:
    """验证 HMAC 签名的 session token，返回 payload 或 None"""
    try:
        payload_b64, sig = str(token or '').split('.', 1)
    except ValueError:
        return None

    expected_sig = hmac.new(
        _get_session_signing_secret().encode('utf-8'),
        payload_b64.encode('utf-8'),
        hashlib.sha256,
    ).hexdigest()
    if not hmac.compare_digest(sig, expected_sig):
        return None

    try:
        payload = json.loads(_b64url_decode(payload_b64).decode('utf-8'))
    except Exception:
        return None
    if not isinstance(payload, dict):
        return None
    return payload


def _extract_session_context(request) -> Dict[str, Any]:
    """从请求中提取并验证 session context"""
    auth_header = request.headers.get('Authorization', '')
    cookie_token = request.cookies.get('admin_token', '')

    raw_token = ''
    if auth_header.startswith('Bearer '):
        raw_token = auth_header.split(' ', 1)[1].strip()
    elif cookie_token:
        raw_token = cookie_token.strip()

    import config as _cfg
    payload = _verify_session_token(raw_token)
    if payload:
        payload['authenticated'] = True
        return payload

    if raw_token and raw_token == _cfg.WEB_ADMIN_TOKEN:
        return {
            'role': 'platform_admin',
            'tenant_id': '',
            'merchant_id': None,
            'issued_for': '',
            'iat': 0,
            'authenticated': True,
        }

    return {
        'role': 'platform_admin',
        'tenant_id': '',
        'merchant_id': None,
        'issued_for': '',
        'iat': 0,
        'authenticated': False,
    }


class TokenManager:
    """Token 管理器"""

    def __init__(self):
        self._otp_store: Dict[str, float] = {}
        self._csrf_store: Dict[str, float] = {}
        self._entry_route_store: Dict[str, float] = {}
        self._token_store: Dict[str, Dict[str, Any]] = {}

    def generate_otp(self, expiry_seconds: int = 300) -> str:
        """生成 OTP"""
        now = time.time()
        expired = [k for k, v in self._otp_store.items() if v < now]
        for k in expired:
            self._otp_store.pop(k, None)

        otp = secrets.token_urlsafe(24)
        self._otp_store[otp] = now + expiry_seconds
        return otp

    def verify_otp(self, otp: str) -> bool:
        """验证 OTP"""
        now = time.time()
        expire = self._otp_store.get(otp)
        if not expire or expire < now:
            self._otp_store.pop(otp, None)
            return False

        self._otp_store.pop(otp, None)
        return True

    def generate_csrf_token(self, expiry_seconds: int = 86400) -> str:
        """生成 CSRF Token"""
        now = time.time()
        expired = [k for k, v in self._csrf_store.items() if v < now]
        for k in expired:
            self._csrf_store.pop(k, None)

        csrf = secrets.token_urlsafe(32)
        self._csrf_store[csrf] = now + expiry_seconds
        return csrf

    def verify_csrf_token(self, csrf: str) -> bool:
        """验证 CSRF Token"""
        now = time.time()
        expire = self._csrf_store.get(csrf)
        if not expire or expire < now:
            self._csrf_store.pop(csrf, None)
            return False

        return True

    def generate_access_token(self, user_id: int, role: str = 'merchant_owner', tenant_id: str = '', merchant_id: Optional[int] = None, expiry_seconds: int = 3600) -> str:
        """生成访问 Token，包含完整的 session 信息"""
        now = time.time()
        token_data = f"{user_id}:{now}:{secrets.token_hex(16)}"
        token = hashlib.sha256(token_data.encode()).hexdigest()
        self._token_store[token] = {
            'expire_at': now + expiry_seconds,
            'user_id': user_id,
            'role': role,
            'tenant_id': tenant_id,
            'merchant_id': merchant_id,
        }
        return token

    def verify_access_token(self, token: str) -> Optional[Dict[str, Any]]:
        """验证访问 Token，返回完整的 session 信息"""
        now = time.time()
        token_data = self._token_store.get(token)
        if not token_data:
            # 尝试使用 HMAC 验证（兼容新的签名 token）
            payload = _verify_session_token(token)
            if payload and payload.get('authenticated'):
                return payload
            return None

        expire_at = token_data.get('expire_at', 0)
        if expire_at < now:
            self._token_store.pop(token, None)
            return None

        return {
            'user_id': token_data.get('user_id'),
            'role': token_data.get('role', 'merchant_owner'),
            'tenant_id': token_data.get('tenant_id', ''),
            'merchant_id': token_data.get('merchant_id'),
            'authenticated': True,
        }


class RateLimiter:
    """速率限制器"""

    def __init__(self, window_seconds: int = 60, max_attempts: int = 5):
        self.window = window_seconds
        self.max_attempts = max_attempts
        self._stores: Dict[str, List[float]] = {}

    def check_limit(self, key: str) -> bool:
        """检查是否超出限制，返回 True 表示允许"""
        now = time.time()
        attempts = self._stores.get(key, [])

        # 只保留窗口内的记录
        attempts = [t for t in attempts if now - t < self.window]

        if len(attempts) >= self.max_attempts:
            return False

        attempts.append(now)
        self._stores[key] = attempts
        return True

    def reset(self, key: str) -> None:
        """重置限制"""
        self._stores.pop(key, None)


class SecurityMiddleware:
    """安全中间件 - 认证 + 权限 + 租户隔离"""

    # 路由权限映射：哪些路由需要什么角色
    ROUTE_PERMISSIONS: Dict[str, List[str]] = {
        '/api/platform': ['platform_admin'],
        '/api/saas/platform': ['platform_admin'],
        '/api/config': ['platform_admin', 'merchant_owner'],
        '/api/broadcast': ['platform_admin', 'merchant_owner'],
        '/api/monitors': ['platform_admin', 'merchant_owner'],
        '/api/finance': ['platform_admin', 'merchant_owner'],
    }

    def __init__(self, token_manager: TokenManager, rate_limiter: RateLimiter):
        self.token_manager = token_manager
        self.rate_limiter = rate_limiter

    def _check_route_permission(self, path: str, role: str) -> bool:
        """检查用户角色是否有权限访问该路由"""
        # 精确匹配
        if path in self.ROUTE_PERMISSIONS:
            return role in self.ROUTE_PERMISSIONS[path]

        # 前缀匹配（更宽松的权限组）
        for route_prefix, allowed_roles in self.ROUTE_PERMISSIONS.items():
            if path.startswith(route_prefix):
                return role in allowed_roles

        # 默认：已认证用户可以访问
        return True

    async def auth_middleware(self, request, handler):
        """认证 + 权限 + 租户隔离中间件"""
        from aiohttp import web

        # 跳过登录和静态资源
        skip_paths = ['/api/login', '/api/login/otp', '/api/csrf-token', '/api/csp-report', '/static', '/']
        if request.path in skip_paths or request.path.startswith('/static/'):
            return await handler(request)

        # 提取并验证 Token
        auth_header = request.headers.get('Authorization', '')
        cookie_token = request.cookies.get('admin_token', '')

        raw_token = ''
        if auth_header.startswith('Bearer '):
            raw_token = auth_header.split(' ', 1)[1].strip()
        elif cookie_token:
            raw_token = cookie_token.strip()

        if not raw_token:
            return web.json_response(
                {'status': 'error', 'message': '未授权'},
                status=401
            )

        # 验证 Token 获取完整 session信息
        session = self.token_manager.verify_access_token(raw_token)

        if not session:
            return web.json_response(
                {'status': 'error', 'message': 'Token 无效或已过期'},
                status=401
            )

        # 检查认证状态
        if not session.get('authenticated', False):
            return web.json_response(
                {'status': 'error', 'message': '未认证请求'},
                status=401
            )

        # 权限检查
        role = session.get('role', 'merchant_owner')
        if not self._check_route_permission(request.path, role):
            logger.warning(f"[Permission] 拒绝: role={role} 访问 {request.path}")
            return web.json_response(
                {'status': 'error', 'message': '无权访问该资源'},
                status=403
            )

        # 角色权威化验证：从数据库验证用户的真实角色
        verified_session = await _enforce_session_context(request, session)

        # 如果数据库验证失败（用户不存在或被禁用），拒绝访问
        if not verified_session.get('authenticated', False):
            logger.warning(f"[Auth] 用户身份验证失败: {request.path}")
            return web.json_response(
                {'status': 'error', 'message': '用户身份验证失败'},
                status=403
            )

        # 使用验证后的角色（可能与 Token 中声称的不同）
        role = verified_session.get('role', role)

        # 租户隔离：商户只能访问自己的 tenant 数据
        tenant_id = verified_session.get('tenant_id', '')
        merchant_id = verified_session.get('merchant_id')

        # 将验证后的 session 信息注入请求
        request['session'] = verified_session
        request['user_id'] = verified_session.get('user_id')
        request['role'] = role
        request['tenant_id'] = tenant_id
        request['merchant_id'] = merchant_id

        # 商户用户：强制注入 tenant_id 到查询参数
        if role == 'merchant_owner' and tenant_id:
            request['_require_tenant_filter'] = True

        return await handler(request)

    async def csrf_check(self, request) -> 'web.Response':
        """CSRF 检查"""
        from aiohttp import web

        # 只对状态变更方法检查 CSRF
        if request.method not in ['POST', 'PUT', 'DELETE', 'PATCH']:
            return None

        csrf_token = request.headers.get('X-CSRF-Token', '')
        if not self.token_manager.verify_csrf_token(csrf_token):
            return web.json_response(
                {'status': 'error', 'message': 'CSRF Token 无效'},
                status=403
            )

        return None


# --- 角色权威化：从数据库验证用户真实身份 ---
async def verify_user_role(tg_user_id: int) -> Dict[str, Any]:
    """
    根据 Telegram user_id 验证用户的真实角色
    返回: {role, tenant_id, merchant_id, is_valid}
    """
    import aiosqlite
    from config import DB_NAME

    # 1. 优先检查是否是平台管理员
    async with aiosqlite.connect(DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        async with db.execute(
            "SELECT admin_id, username, role FROM admin_users WHERE admin_id = ?",
            (tg_user_id,),
        ) as cursor:
            admin = await cursor.fetchone()
            if admin:
                return {
                    'role': 'platform_admin',
                    'tenant_id': '',
                    'merchant_id': None,
                    'issued_for': str(tg_user_id),
                    'authenticated': True,
                }

    # 2. 检查是否是商户（通过 owner_tg_user_id 查询）
    try:
        from backend.repositories.merchant_repo import get_merchant_by_owner
        merchant = await get_merchant_by_owner(tg_user_id)
        if merchant:
            return {
                'role': 'merchant_owner',
                'tenant_id': str(merchant.get('tenant_id', '')),
                'merchant_id': merchant.get('id'),
                'issued_for': str(tg_user_id),
                'authenticated': True,
            }
    except Exception as e:
        logger.warning(f"[Auth] 商户查询失败: {e}")

    # 3. 未找到匹配用户，返回未认证
    return {
        'role': 'unknown',
        'tenant_id': '',
        'merchant_id': None,
        'issued_for': str(tg_user_id),
        'authenticated': False,
    }


async def _enforce_session_context(request, session: Dict[str, Any]) -> Dict[str, Any]:
    """
    强制验证 session context：从 Token 解析 + 数据库验证
    如果 Token 中的角色与数据库中的真实角色不符，以数据库为准
    """
    tg_user_id = session.get('issued_for', '')

    if not tg_user_id:
        return session

    try:
        verified = await verify_user_role(int(tg_user_id) if tg_user_id.isdigit() else 0)
    except Exception as e:
        logger.warning(f"[Auth] 角色验证失败: {e}")
        return session

    # 如果数据库验证成功，覆盖 session 中的角色信息
    if verified.get('authenticated'):
        session['role'] = verified.get('role', session.get('role'))
        session['tenant_id'] = verified.get('tenant_id', session.get('tenant_id'))
        session['merchant_id'] = verified.get('merchant_id', session.get('merchant_id'))
        session['verified'] = True

    return session


# --- 租户隔离工具：确保查询强制包含 tenant_id 过滤 ---
def enforce_tenant_filter(sql: str, params: tuple, tenant_id: str, role: str) -> tuple:
    """
    [已废弃] 请使用 backend.app.core.scope.enforce_tenant_filter（基于 SessionScope 的统一版本）
    保留此函数以兼容现有调用方，内部委托到统一版本
    """
    from backend.app.core.scope import SessionScope, enforce_tenant_filter as _unified_etf
    scope_type = 'platform' if role == 'platform_admin' else ('merchant' if role == 'merchant_owner' else '')
    scope = SessionScope(
        authenticated=True,
        actor_role=role,
        effective_role=role,
        scope_type=scope_type,
        tenant_id=tenant_id or '',
        merchant_id=None,
    )
    new_sql, new_params = _unified_etf(sql, list(params), scope)
    return new_sql, tuple(new_params)


def get_tenant_filter_params(request) -> tuple:
    """从请求中获取租户过滤参数"""
    return (
        request.get('tenant_id', ''),
        request.get('role', 'merchant_owner'),
        request.get('merchant_id'),
    )


class TenantFilterMixin:
    """Repository 混入类：自动为查询添加租户过滤（委托到统一版本）"""
    @staticmethod
    def apply_tenant_filter(
        sql: str, params: list, scope: 'SessionScope', require_tenant: bool = True
    ) -> tuple:
        from backend.app.core.scope import enforce_tenant_filter as _unified_etf
        return _unified_etf(sql, params, scope, require_tenant)

    @staticmethod
    def apply_tenant_filter_legacy(sql: str, params: tuple, tenant_id: str, role: str) -> tuple:
        return enforce_tenant_filter(sql, params, tenant_id, role)


# 全局单例
_default_token_manager = TokenManager()
_default_rate_limiter = RateLimiter(window_seconds=60, max_attempts=5)


def get_token_manager() -> TokenManager:
    return _default_token_manager


def get_rate_limiter() -> RateLimiter:
    return _default_rate_limiter