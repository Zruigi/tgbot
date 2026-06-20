"""
统一 Scope 模型

定义 SessionScope 结构，用于在所有层次（API -> Service -> Repository）
中统一传递身份和作用域信息。

核心原则：
- 默认隔离：merchant scope 必须带 tenant_id
- 显式越权：只有 platform_admin 可用 platform scope
"""
from typing import Optional, Dict, Any, Literal, Callable
from dataclasses import dataclass
from aiohttp import web
import logging

logger = logging.getLogger(__name__)


@dataclass
class SessionScope:
    """统一会话作用域"""
    # 认证状态
    authenticated: bool

    # 谁登录的
    actor_role: Literal['platform_admin', 'merchant_owner', '']

    # 当前页面/请求以什么视角运行
    effective_role: Literal['platform_admin', 'merchant_owner', '']

    # 本次请求的数据范围
    scope_type: Literal['platform', 'merchant', '']

    # 商户 scope 必填
    tenant_id: str

    # 商户 scope 必填
    merchant_id: Optional[int]

    # 原始用户 ID（如果有）
    actor_user_id: Optional[int] = None

    @property
    def is_platform_admin(self) -> bool:
        return self.effective_role == 'platform_admin' and self.scope_type == 'platform'

    @property
    def is_merchant(self) -> bool:
        return self.effective_role == 'merchant_owner' and self.scope_type == 'merchant'

    @property
    def is_authenticated(self) -> bool:
        return self.authenticated

    def require_tenant_id(self) -> str:
        """商户 scope 必须有 tenant_id"""
        if self.scope_type != 'merchant' or not self.tenant_id:
            raise ValueError(f"tenant_id is required for merchant scope, but got: {self}")
        return self.tenant_id

    def require_merchant_id(self) -> int:
        """商户 scope 必须有 merchant_id"""
        if self.scope_type != 'merchant' or self.merchant_id is None:
            raise ValueError(f"merchant_id is required for merchant scope, but got: {self}")
        return self.merchant_id

    def to_dict(self) -> Dict[str, Any]:
        return {
            'authenticated': self.authenticated,
            'actor_role': self.actor_role,
            'effective_role': self.effective_role,
            'scope_type': self.scope_type,
            'tenant_id': self.tenant_id,
            'merchant_id': self.merchant_id,
            'actor_user_id': self.actor_user_id,
        }


def extract_scope_from_session(session: Dict[str, Any]) -> SessionScope:
    """
    从 request['session'] 或 token payload 中提取 SessionScope
    """
    authenticated = session.get('authenticated', False)
    role = session.get('role', '')
    tenant_id = session.get('tenant_id', '')
    merchant_id = session.get('merchant_id')

    # 确定 scope_type
    if role == 'merchant_owner' and tenant_id:
        scope_type: Literal['platform', 'merchant', ''] = 'merchant'
    elif role == 'platform_admin':
        scope_type = 'platform'
    else:
        scope_type = ''

    return SessionScope(
        authenticated=authenticated,
        actor_role=role,
        effective_role=role,
        scope_type=scope_type,
        tenant_id=tenant_id,
        merchant_id=merchant_id,
    )


def enforce_tenant_filter(
    sql: str,
    params: list,
    scope: SessionScope,
    require_tenant: bool = True,
    tenant_column: str = 'tenant_id',
) -> tuple:
    """
    在 SQL 查询中强制添加 tenant_id 过滤

    规则：
    - platform scope: 不加强制过滤（可查全局）
    - merchant scope: 必须有 tenant_id，否则报错
    - 未认证: 报错
    - tenant_column: 支持自定义列名（默认 'tenant_id'）
    """
    if not scope.authenticated:
        raise PermissionError("Unauthenticated request")

    if scope.scope_type == 'platform':
        return sql, params

    if scope.scope_type == 'merchant':
        tenant_id = scope.require_tenant_id()

        if tenant_column.lower() not in sql.lower():
            if 'WHERE' in sql.upper():
                sql = sql + f" AND {tenant_column} = ?"
            else:
                sql = sql + f" WHERE {tenant_column} = ?"
            params = params + [tenant_id]

        return sql, params

    if require_tenant:
        raise ValueError(f"tenant_id is required but scope is: {scope}")

    return sql, params


@web.middleware
async def tenant_context_middleware(request: web.Request, handler: Callable) -> web.Response:
    """
    租户上下文注入中间件

    在 SecurityMiddleware 之后执行，将 request['session'] 转化为
    标准 SessionScope 并注入到 request 中，供后续 handler 使用。
    """
    session = request.get('session', {})
    scope = extract_scope_from_session(session)

    if scope.is_merchant and not scope.tenant_id:
        logger.warning(f"[TenantIsolation] 商户角色但 tenant_id 为空，拒绝访问: {request.path}")
        return web.json_response(
            {'status': 'error', 'message': '租户身份验证失败'},
            status=403
        )

    request['scope'] = scope
    request['tenant_id'] = scope.tenant_id
    request['role'] = scope.effective_role
    request['merchant_id'] = scope.merchant_id
    request['_require_tenant_filter'] = scope.is_merchant

    return await handler(request)