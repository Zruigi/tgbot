# -*- coding: utf-8 -*-
"""
SaaS API - session context、平台商户管理、商户自服务接口
"""
from typing import Optional, Dict, Any
import logging
import aiosqlite

from aiohttp import web

import config
from backend.repositories import merchant_repo, subscription_repo, payment_repo
from backend.repositories.ai_usage_repo import get_global_usage_summary
from backend.app.services import (
    get_audit_service,
    get_feature_gate_service,
    get_quota_service,
    get_merchant_onboarding_service,
    get_subscription_lifecycle_service,
    get_merchant_runtime_manager,
)

logger = logging.getLogger(__name__)


def _safe_int(value: Any, default: Optional[int] = None) -> Optional[int]:
    try:
        if value is None or value == '':
            return default
        return int(value)
    except Exception:
        return default


def _get_request_session(request) -> Dict[str, Any]:
    session = request.get('session') or {}
    role = str(session.get('role') or 'platform_admin').strip() or 'platform_admin'
    actor_role = str(session.get('actor_role') or role or 'platform_admin').strip() or role
    return {
        'role': role,
        'actor_role': actor_role,
        'tenant_id': str(session.get('tenant_id') or '').strip(),
        'merchant_id': _safe_int(session.get('merchant_id')),
        'iat': _safe_int(session.get('iat'), 0) or 0,
        'authenticated': session.get('authenticated', False),  # 新增: 认证状态
    }


def _issue_session_token(*, role: str, tenant_id: str = '', merchant_id: Optional[int] = None, actor_role: Optional[str] = None) -> str:
    import web_admin as wa

    return wa._issue_token(
        config.WEB_ADMIN_TOKEN,
        role=role,
        tenant_id=tenant_id,
        merchant_id=merchant_id,
        actor_role=actor_role,
    )


def _derive_onboarding_completed(merchant: Dict[str, Any], has_bot_token: bool = False) -> bool:
    explicit = merchant.get('onboarding_completed')
    if explicit is not None:
        try:
            if int(explicit) == 1:
                return True
        except Exception:
            if bool(explicit):
                return True

    status = str(merchant.get('status') or '').strip().lower()
    plan_type = str(merchant.get('plan_type') or '').strip().lower()
    subscription_status = str(merchant.get('subscription_status') or '').strip().lower()
    has_bot_binding = bool(merchant.get('bot_id') or merchant.get('bot_username') or has_bot_token)

    if not has_bot_binding:
        return False

    return (
        status in {'active_free', 'active_pro', 'active_paid', 'active'}
        or plan_type in {'free', 'pro', 'paid'}
        or subscription_status in {'active', 'free', 'paid', 'active_subscription'}
    )


async def _compact_platform_merchant_rows(merchants: list[Dict[str, Any]]) -> list[Dict[str, Any]]:
    if not merchants:
        return []

    merchant_ids = [int(m['id']) for m in merchants if m.get('id') is not None]
    owner_ids = [int(m['owner_tg_user_id']) for m in merchants if m.get('owner_tg_user_id') is not None]
    latest_subscriptions: Dict[int, Dict[str, Any]] = {}
    usernames: Dict[int, str] = {}

    async with aiosqlite.connect(config.DB_NAME) as db:
        db.row_factory = aiosqlite.Row
        if merchant_ids:
            placeholders = ','.join('?' for _ in merchant_ids)
            async with db.execute(
                f"""
                SELECT s.*
                FROM merchant_subscriptions s
                JOIN (
                    SELECT merchant_id, MAX(COALESCE(end_at, created_at)) AS max_at
                    FROM merchant_subscriptions
                    WHERE merchant_id IN ({placeholders})
                    GROUP BY merchant_id
                ) latest ON latest.merchant_id = s.merchant_id
                    AND latest.max_at = COALESCE(s.end_at, s.created_at)
                """,
                merchant_ids,
            ) as cursor:
                for row in await cursor.fetchall():
                    latest_subscriptions[int(row['merchant_id'])] = dict(row)

        if owner_ids:
            placeholders = ','.join('?' for _ in owner_ids)
            async with db.execute(
                f"""
                SELECT user_id, MAX(username) AS username
                FROM (
                    SELECT user_id, NULLIF(username, '') AS username FROM potential_users WHERE user_id IN ({placeholders})
                    UNION ALL
                    SELECT from_userid AS user_id, NULLIF(from_username, '') AS username FROM admin_inbox WHERE from_userid IN ({placeholders})
                    UNION ALL
                    SELECT user_id, COALESCE(NULLIF(username, ''), NULLIF(first_name, '')) AS username FROM group_members WHERE user_id IN ({placeholders})
                )
                WHERE username IS NOT NULL
                GROUP BY user_id
                """,
                owner_ids + owner_ids + owner_ids,
            ) as cursor:
                for row in await cursor.fetchall():
                    usernames[int(row['user_id'])] = str(row['username'] or '').strip()

    compact = []
    for merchant in merchants:
        merchant_id = int(merchant.get('id') or 0)
        owner_tg_user_id = int(merchant.get('owner_tg_user_id') or 0)
        subscription = latest_subscriptions.get(merchant_id) or {}
        paid_option = str(subscription.get('plan_code') or merchant.get('plan_type') or 'free').strip() or 'free'
        expires_at = subscription.get('end_at') or merchant.get('subscription_expires_at')
        is_paid = paid_option not in {'', 'free'} or str(subscription.get('status') or merchant.get('subscription_status') or '') in {
            'active',
            'active_subscription',
            'paid',
            'confirmed',
        }
        compact.append({
            'id': merchant_id,
            'user_id': owner_tg_user_id,
            'username': usernames.get(owner_tg_user_id) or str(merchant.get('merchant_name') or f'User {owner_tg_user_id}'),
            'registered_at': merchant.get('created_at'),
            'paid_option': paid_option,
            'expires_at': expires_at,
            'isPaid': bool(is_paid),
        })
    return compact


async def _get_canonical_session_context(request) -> Dict[str, Any]:
    session = _get_request_session(request)
    if session['role'] != 'merchant_owner' or session['merchant_id'] is None:
        return session

    merchant = await merchant_repo.get_merchant_by_id(int(session['merchant_id']))
    if not merchant:
        return session

    return {
        'role': 'merchant_owner',
        'actor_role': str(session.get('actor_role') or 'merchant_owner').strip() or 'merchant_owner',
        'tenant_id': str(merchant.get('tenant_id') or '').strip(),
        'merchant_id': int(merchant.get('id')),
        'iat': session.get('iat') or 0,
    }


def _require_platform_admin(request) -> Optional[web.Response]:
    """平台管理员权限校验 - 必须通过认证且角色为 platform_admin"""
    session = _get_request_session(request)

    # 先检查是否已认证
    if not session.get('authenticated', True):
        return web.json_response({'status': 'error', 'message': '未认证请求'}, status=401)

    role = session.get('role')
    if role != 'platform_admin':
        return web.json_response({'status': 'error', 'message': 'forbidden'}, status=403)
    return None


def _require_authenticated(request) -> Optional[web.Response]:
    """基础认证校验 - 只要通过认证即可"""
    session = _get_request_session(request)
    if not session.get('authenticated', True):
        return web.json_response({'status': 'error', 'message': '未认证请求'}, status=401)
    return None


def _require_merchant_or_platform_admin(request) -> Optional[web.Response]:
    """商户或平台管理员权限校验"""
    session = _get_request_session(request)

    # 先检查是否已认证
    if not session.get('authenticated', True):
        return web.json_response({'status': 'error', 'message': '未认证请求'}, status=401)

    role = session.get('role')
    if role not in ('platform_admin', 'merchant_owner'):
        return web.json_response({'status': 'error', 'message': 'forbidden'}, status=403)
    return None


async def api_get_session_context(request):
    try:
        session = await _get_canonical_session_context(request)
        return web.json_response({'status': 'ok', 'data': session})
    except Exception as e:
        logger.error(f'api_get_session_context error: {e}')
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_update_session_context(request):
    try:
        current = await _get_canonical_session_context(request)
        current_role = str(current.get('role') or 'platform_admin')
        current_actor_role = str(current.get('actor_role') or current_role or 'platform_admin')
        data = await request.json()

        # 安全校验：商户不能切换到 platform_admin
        if current_actor_role != 'platform_admin' and str(data.get('role', '')).strip() == 'platform_admin':
            logger.warning(f"[Auth] 非平台管理员试图切换到 platform_admin: current_role={current_role}, current_actor_role={current_actor_role}")
            from backend.repositories.audit_repo import log_tenant_isolation_event
            _tid = str(current.get('tenant_id') or '')
            await log_tenant_isolation_event(
                tenant_id=_tid or 'unknown',
                action='role_escalation_denied',
                resource_type='session_context',
                resource_id='role_switch',
                operator_role=current_actor_role,
                request_path=request.path,
            )
            return web.json_response({
                "status": "error",
                "message": "无权切换到平台管理员角色",
            }, status=403)

        role = str(data.get('role') or current_role or 'platform_admin').strip() or 'platform_admin'

        if current_role == 'platform_admin':
            if role == 'platform_admin':
                session_token = _issue_session_token(
                    role='platform_admin',
                    tenant_id='',
                    merchant_id=None,
                    actor_role=current_actor_role,
                )
                return web.json_response({
                    'status': 'ok',
                    'token': session_token,
                    'data': {
                        'role': 'platform_admin',
                        'actor_role': current_actor_role,
                        'tenant_id': '',
                        'merchant_id': None,
                    },
                })

            if role != 'merchant_owner':
                return web.json_response({'status': 'error', 'message': 'unsupported_role'}, status=400)

            tenant_id_input = str(data.get('tenant_id') or '').strip()
            merchant_id_input = _safe_int(data.get('merchant_id'))
            merchant = None
            if merchant_id_input is not None:
                merchant = await merchant_repo.get_merchant_by_id(merchant_id_input)
                if merchant and tenant_id_input and str(merchant.get('tenant_id') or '') != tenant_id_input:
                    return web.json_response({'status': 'error', 'message': 'merchant_session_mismatch'}, status=400)
            elif tenant_id_input:
                merchant = await merchant_repo.get_merchant_by_tenant_id(tenant_id_input)
            else:
                return web.json_response({'status': 'error', 'message': 'merchant_selector_required'}, status=400)

            if not merchant:
                return web.json_response({'status': 'error', 'message': 'merchant_not_found'}, status=404)

            session_token = _issue_session_token(
                role='merchant_owner',
                tenant_id=str(merchant.get('tenant_id') or ''),
                merchant_id=int(merchant.get('id')),
                actor_role=current_actor_role,
            )
            return web.json_response({
                'status': 'ok',
                'token': session_token,
                'data': {
                    'role': 'merchant_owner',
                    'actor_role': current_actor_role,
                    'tenant_id': str(merchant.get('tenant_id') or ''),
                    'merchant_id': int(merchant.get('id')),
                },
            })

        if current_role != 'merchant_owner':
            return web.json_response({'status': 'error', 'message': 'forbidden'}, status=403)

        if role == 'platform_admin' and current_actor_role == 'platform_admin':
            session_token = _issue_session_token(
                role='platform_admin',
                tenant_id='',
                merchant_id=None,
                actor_role='platform_admin',
            )
            return web.json_response({
                'status': 'ok',
                'token': session_token,
                'data': {
                    'role': 'platform_admin',
                    'actor_role': 'platform_admin',
                    'tenant_id': '',
                    'merchant_id': None,
                },
            })

        if role != 'merchant_owner':
            return web.json_response({'status': 'error', 'message': 'forbidden'}, status=403)

        current_tenant_id = str(current.get('tenant_id') or '').strip()
        current_merchant_id = _safe_int(current.get('merchant_id'))
        if not current_tenant_id or current_merchant_id is None:
            return web.json_response({'status': 'error', 'message': 'merchant_session_required'}, status=400)

        tenant_id_input = str(data.get('tenant_id') or current_tenant_id).strip()
        merchant_id_input = _safe_int(data.get('merchant_id'), current_merchant_id)
        if tenant_id_input != current_tenant_id or merchant_id_input != current_merchant_id:
            return web.json_response({'status': 'error', 'message': 'merchant_session_mismatch'}, status=403)

        merchant = await merchant_repo.get_merchant_by_id(current_merchant_id)
        if not merchant or str(merchant.get('tenant_id') or '').strip() != current_tenant_id:
            return web.json_response({'status': 'error', 'message': 'merchant_session_mismatch'}, status=400)

        session_token = _issue_session_token(
            role='merchant_owner',
            tenant_id=current_tenant_id,
            merchant_id=current_merchant_id,
            actor_role=current_actor_role,
        )
        return web.json_response({
            'status': 'ok',
            'token': session_token,
            'data': {
                'role': 'merchant_owner',
                'actor_role': current_actor_role,
                'tenant_id': current_tenant_id,
                'merchant_id': current_merchant_id,
            },
        })
    except Exception as e:
        logger.error(f'api_update_session_context error: {e}')
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def resolve_effective_merchant_session(request, body: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
    session = await _get_canonical_session_context(request)
    role = str(session.get('role') or 'platform_admin')
    tenant_id = str(session.get('tenant_id') or '').strip()
    merchant_id = _safe_int(session.get('merchant_id'))

    if role not in ('merchant_owner', 'platform_admin'):
        raise ValueError('forbidden')
    if not tenant_id or merchant_id is None:
        raise ValueError('merchant_session_required')

    merchant = await merchant_repo.get_merchant_by_id(merchant_id)
    if not merchant or str(merchant.get('tenant_id') or '').strip() != tenant_id:
        raise ValueError('merchant_session_mismatch')

    return {'tenant_id': tenant_id, 'merchant_id': int(merchant['id']), 'merchant': merchant}


async def api_platform_list_merchants(request):
    forbidden = _require_platform_admin(request)
    if forbidden:
        return forbidden
    try:
        status = (request.rel_url.query.get('status') or '').strip() or None
        limit = min(_safe_int(request.rel_url.query.get('limit'), 50) or 50, 200)
        offset = max(_safe_int(request.rel_url.query.get('offset'), 0) or 0, 0)
        merchants = await merchant_repo.list_merchants(limit=limit, offset=offset, status=status)
        compact = await _compact_platform_merchant_rows(merchants)
        return web.json_response({'status': 'ok', 'data': compact, 'total': len(compact)})
    except Exception as e:
        logger.error(f'api_platform_list_merchants error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_platform_create_merchant(request):
    forbidden = _require_platform_admin(request)
    if forbidden:
        return forbidden
    try:
        data = await request.json()
        owner_tg_user_id = _safe_int(data.get('owner_tg_user_id'))
        merchant_name = str(data.get('merchant_name') or '').strip()
        tenant_id = str(data.get('tenant_id') or '').strip() or None
        bot_token = str(data.get('bot_token') or '').strip() or None

        if not owner_tg_user_id:
            return web.json_response({'status': 'error', 'message': 'owner_tg_user_id 不能为空'}, status=400)
        if not merchant_name:
            return web.json_response({'status': 'error', 'message': 'merchant_name 不能为空'}, status=400)

        merchant = await get_merchant_onboarding_service().create_merchant(
            owner_tg_user_id=owner_tg_user_id,
            merchant_name=merchant_name,
            tenant_id=tenant_id,
            bot_token=bot_token,
        )
        return web.json_response({'status': 'ok', 'data': merchant})
    except ValueError as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=400)
    except Exception as e:
        logger.error(f'api_platform_create_merchant error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_platform_delete_merchant(request):
    """删除商户（管理员专用）"""
    forbidden = _require_platform_admin(request)
    if forbidden:
        return forbidden
    try:
        merchant_id = _safe_int(request.match_info.get('merchant_id'))
        if not merchant_id:
            return web.json_response({'status': 'error', 'message': 'merchant_id 不能为空'}, status=400)
        
        # 检查商户是否存在
        merchant = await merchant_repo.get_merchant_by_id(merchant_id)
        if not merchant:
            return web.json_response({'status': 'error', 'message': '商户不存在'}, status=404)
        
        tenant_id = str(merchant.get('tenant_id') or '').strip()
        
        # 删除商户相关数据
        import aiosqlite
        from config import DB_NAME
        from services.secret_manager import SecretManager
        
        # 删除SecretManager中的Bot Token
        if tenant_id:
            try:
                await SecretManager.delete_config('TELEGRAM_BOT_TOKEN', tenant_id=tenant_id)
            except Exception as e:
                logger.warning(f'Failed to delete bot token for tenant {tenant_id}: {e}')
        
        async with aiosqlite.connect(DB_NAME) as db:
            # 删除功能快照
            await db.execute("DELETE FROM merchant_features WHERE merchant_id = ?", (merchant_id,))
            # 删除AI配额
            await db.execute("DELETE FROM merchant_ai_quota WHERE merchant_id = ?", (merchant_id,))
            # 删除订阅记录
            await db.execute("DELETE FROM merchant_subscriptions WHERE merchant_id = ?", (merchant_id,))
            # 删除支付订单
            await db.execute("DELETE FROM payment_orders WHERE merchant_id = ?", (merchant_id,))
            # 删除审计日志
            await db.execute("DELETE FROM tenant_audit_logs WHERE merchant_id = ?", (merchant_id,))
            # 删除用户数据
            await db.execute("DELETE FROM users WHERE tenant_id = ?", (tenant_id,))
            # 删除商户
            await db.execute("DELETE FROM merchants WHERE id = ?", (merchant_id,))
            await db.commit()
        
        # 记录审计日志
        await get_audit_service().log(
            tenant_id=tenant_id or 'deleted',
            merchant_id=None,
            operator_role='platform_admin',
            action='merchant_deleted',
            resource_type='merchant',
            resource_id=str(merchant_id),
            payload={
                'merchant_name': merchant.get('merchant_name'),
                'tenant_id': tenant_id,
            },
        )
        
        return web.json_response({'status': 'ok', 'message': '商户已删除'})
    except Exception as e:
        logger.error(f'api_platform_delete_merchant error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_platform_confirm_merchant_onboarding(request):
    forbidden = _require_platform_admin(request)
    if forbidden:
        return forbidden
    try:
        data = await request.json()
        merchant_id = _safe_int(data.get('merchant_id'))
        note = str(data.get('note') or '').strip() or None
        if not merchant_id:
            return web.json_response({'status': 'error', 'message': 'merchant_id 不能为空'}, status=400)
        result = await get_merchant_onboarding_service().confirm_onboarding(merchant_id, note=note)
        return web.json_response({'status': 'ok', 'data': result})
    except ValueError as e:
        code = str(e)
        status = 404 if code == 'merchant_not_found' else 400
        return web.json_response({'status': 'error', 'message': code}, status=status)
    except Exception as e:
        logger.error(f'api_platform_confirm_merchant_onboarding error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_platform_list_runtime_statuses(request):
    forbidden = _require_platform_admin(request)
    if forbidden:
        return forbidden
    try:
        rows = get_merchant_runtime_manager().list_runtime_snapshots()
        return web.json_response({'status': 'ok', 'data': rows, 'total': len(rows)})
    except Exception as e:
        logger.error(f'api_platform_list_runtime_statuses error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_platform_list_plans(request):
    forbidden = _require_platform_admin(request)
    if forbidden:
        return forbidden
    try:
        active_only = (request.rel_url.query.get('active_only', 'true').strip().lower() != 'false')
        plans = await subscription_repo.list_plans(active_only=active_only)
        return web.json_response({'status': 'ok', 'data': plans, 'total': len(plans)})
    except Exception as e:
        logger.error(f'api_platform_list_plans error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_platform_upsert_plan(request):
    forbidden = _require_platform_admin(request)
    if forbidden:
        return forbidden
    try:
        data = await request.json()
        plan_code = str(data.get('plan_code') or '').strip()
        plan_name = str(data.get('plan_name') or '').strip()
        price_usdt = float(data.get('price_usdt') or 0)
        duration_days = int(data.get('duration_days') or 30)
        features_json = str(data.get('features_json') or '{}').strip()
        status = str(data.get('status') or 'active').strip()

        if not plan_code:
            return web.json_response({'status': 'error', 'message': 'plan_code 不能为空'}, status=400)
        if not plan_name:
            return web.json_response({'status': 'error', 'message': 'plan_name 不能为空'}, status=400)

        ok = await subscription_repo.upsert_plan(
            plan_code=plan_code,
            plan_name=plan_name,
            price_usdt=price_usdt,
            duration_days=duration_days,
            features_json=features_json,
            status=status,
        )
        if ok:
            plan = await subscription_repo.get_plan(plan_code)
            return web.json_response({'status': 'ok', 'data': plan})
        return web.json_response({'status': 'error', 'message': '保存失败'}, status=500)
    except Exception as e:
        logger.error(f'api_platform_upsert_plan error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_merchant_list_plans(request):
    try:
        await resolve_effective_merchant_session(request)
        active_only = (request.rel_url.query.get('active_only', 'true').strip().lower() != 'false')
        plans = await subscription_repo.list_plans(active_only=active_only)
        return web.json_response({'status': 'ok', 'data': plans, 'total': len(plans)})
    except ValueError as e:
        code = str(e)
        status = 404 if code == 'merchant_not_found' else 400
        return web.json_response({'status': 'error', 'message': code}, status=status)
    except Exception as e:
        logger.error(f'api_merchant_list_plans error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_platform_list_subscriptions(request):
    forbidden = _require_platform_admin(request)
    if forbidden:
        return forbidden
    try:
        status = (request.rel_url.query.get('status') or '').strip() or None
        merchant_id = _safe_int(request.rel_url.query.get('merchant_id'))
        limit = min(_safe_int(request.rel_url.query.get('limit'), 100) or 100, 200)
        rows = await subscription_repo.list_subscriptions(status=status, merchant_id=merchant_id, limit=limit)
        return web.json_response({'status': 'ok', 'data': rows, 'total': len(rows)})
    except Exception as e:
        logger.error(f'api_platform_list_subscriptions error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_platform_list_payment_orders(request):
    forbidden = _require_platform_admin(request)
    if forbidden:
        return forbidden
    try:
        status = (request.rel_url.query.get('status') or '').strip() or None
        merchant_id = _safe_int(request.rel_url.query.get('merchant_id'))
        limit = min(_safe_int(request.rel_url.query.get('limit'), 100) or 100, 200)
        rows = await payment_repo.list_payment_orders(status=status, merchant_id=merchant_id, limit=limit)
        enriched = []
        for row in rows:
            merchant = await merchant_repo.get_merchant_by_id(int(row['merchant_id']))
            enriched.append({
                **row,
                'tenant_id': merchant.get('tenant_id') if merchant else None,
                'merchant_name': merchant.get('merchant_name') if merchant else None,
            })
        return web.json_response({'status': 'ok', 'data': enriched, 'total': len(enriched)})
    except Exception as e:
        logger.error(f'api_platform_list_payment_orders error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_platform_confirm_payment(request):
    forbidden = _require_platform_admin(request)
    if forbidden:
        return forbidden
    try:
        data = await request.json()
        order_id = _safe_int(data.get('order_id'))
        if not order_id:
            return web.json_response({'status': 'error', 'message': 'order_id 不能为空'}, status=400)
        result = await get_subscription_lifecycle_service().confirm_payment(order_id)
        return web.json_response({'status': 'ok', 'data': result})
    except ValueError as e:
        code = str(e)
        status = 404 if code in ('payment_order_not_found', 'subscription_plan_not_found') else 400
        return web.json_response({'status': 'error', 'message': code}, status=status)
    except Exception as e:
        logger.error(f'api_platform_confirm_payment error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_platform_reject_payment(request):
    forbidden = _require_platform_admin(request)
    if forbidden:
        return forbidden
    try:
        data = await request.json()
        order_id = _safe_int(data.get('order_id'))
        if not order_id:
            return web.json_response({'status': 'error', 'message': 'order_id 不能为空'}, status=400)
        ok = await get_subscription_lifecycle_service().reject_payment(order_id)
        return web.json_response({'status': 'ok' if ok else 'error', 'message': '已拒绝' if ok else '更新失败'})
    except ValueError as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=400)
    except Exception as e:
        logger.error(f'api_platform_reject_payment error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_platform_run_subscription_downgrade(request):
    forbidden = _require_platform_admin(request)
    if forbidden:
        return forbidden
    try:
        rows = await get_subscription_lifecycle_service().downgrade_expired_subscriptions()
        return web.json_response({'status': 'ok', 'data': rows, 'total': len(rows)})
    except Exception as e:
        logger.error(f'api_platform_run_subscription_downgrade error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_merchant_get_profile(request):
    try:
        resolved = await resolve_effective_merchant_session(request)
        merchant = resolved['merchant']
        
        # 检查是否有bot_token（不返回实际token，只返回是否存在）
        from services.secret_manager import SecretManager
        has_bot_token = bool(await SecretManager.get_config('TELEGRAM_BOT_TOKEN', tenant_id=resolved['tenant_id'], default=''))
        
        # 添加bot_token字段供前端判断
        merchant_with_token_flag = {
            **merchant,
            'bot_token': 'exists' if has_bot_token else None,
            'onboarding_completed': _derive_onboarding_completed(merchant, has_bot_token=has_bot_token),
        }
        
        return web.json_response({'status': 'ok', 'data': merchant_with_token_flag})
    except ValueError as e:
        code = str(e)
        status = 404 if code == 'merchant_not_found' else 400
        return web.json_response({'status': 'error', 'message': code}, status=status)
    except Exception as e:
        logger.error(f'api_merchant_get_profile error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_merchant_update_status(request):
    """商户更新自己的状态（完成注册）"""
    try:
        resolved = await resolve_effective_merchant_session(request)
        merchant_id = int(resolved['merchant'].get('id') or 0)
        
        data = await request.json()
        status = str(data.get('status') or '').strip()
        onboarding_completed = data.get('onboarding_completed', False)
        
        if not status:
            return web.json_response({'status': 'error', 'message': 'status 不能为空'}, status=400)
        
        update_data = {'status': status}
        if status == 'active_free':
            update_data.setdefault('plan_type', 'free')
            update_data.setdefault('subscription_status', 'active')
        if onboarding_completed:
            update_data['onboarding_completed'] = 1

        await merchant_repo.update_merchant_by_tenant(merchant_id, resolved['tenant_id'], **update_data)
        get_merchant_runtime_manager().request_reconcile(resolved['tenant_id'])
        
        return web.json_response({'status': 'ok', 'message': '状态已更新'})
    except Exception as e:
        logger.error(f'api_merchant_update_status error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_merchant_save_bot_token(request):
    try:
        data = await request.json()
        resolved = await resolve_effective_merchant_session(request, data)
        bot_token = str(data.get('bot_token') or '').strip()
        if not bot_token:
            return web.json_response({'status': 'error', 'message': 'bot_token 不能为空'}, status=400)
        bot_info = await get_merchant_onboarding_service().save_bot_token(resolved['tenant_id'], bot_token)
        get_merchant_runtime_manager().request_reconcile(resolved['tenant_id'])
        return web.json_response({'status': 'ok', 'data': bot_info})
    except ValueError as e:
        code = str(e)
        status = 404 if code == 'merchant_not_found' else 400
        return web.json_response({'status': 'error', 'message': code}, status=status)
    except Exception as e:
        logger.error(f'api_merchant_save_bot_token error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_merchant_get_subscription(request):
    try:
        resolved = await resolve_effective_merchant_session(request)
        active_subscription = await subscription_repo.get_active_subscription(resolved['merchant_id'])
        return web.json_response({'status': 'ok', 'data': active_subscription, 'merchant': resolved['merchant']})
    except ValueError as e:
        code = str(e)
        status = 404 if code == 'merchant_not_found' else 400
        return web.json_response({'status': 'error', 'message': code}, status=status)
    except Exception as e:
        logger.error(f'api_merchant_get_subscription error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_merchant_create_payment_order(request):
    try:
        data = await request.json()
        resolved = await resolve_effective_merchant_session(request, data)
        plan_code = str(data.get('plan_code') or '').strip()
        if not plan_code:
            return web.json_response({'status': 'error', 'message': 'plan_code 不能为空'}, status=400)
        order = await get_subscription_lifecycle_service().create_payment_order(resolved['merchant_id'], plan_code)
        await get_audit_service().log(
            tenant_id=resolved['tenant_id'],
            merchant_id=resolved['merchant_id'],
            operator_role='merchant_owner',
            action='payment_order_created',
            resource_type='payment_order',
            resource_id=str(order.get('id') or ''),
            payload={
                'plan_code': plan_code,
                'base_amount_usdt': order.get('base_amount_usdt'),
                'amount_offset_usdt': order.get('amount_offset_usdt'),
                'amount_usdt': order.get('amount_usdt'),
            },
        )
        return web.json_response({'status': 'ok', 'data': order})
    except ValueError as e:
        code = str(e)
        status = 404 if code == 'merchant_not_found' else 400
        return web.json_response({'status': 'error', 'message': code}, status=status)
    except Exception as e:
        logger.error(f'api_merchant_create_payment_order error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_merchant_get_latest_payment_order(request):
    try:
        resolved = await resolve_effective_merchant_session(request)
        order = await get_subscription_lifecycle_service().get_latest_open_order(resolved['merchant_id'])
        return web.json_response({'status': 'ok', 'data': order})
    except ValueError as e:
        code = str(e)
        status = 404 if code == 'merchant_not_found' else 400
        return web.json_response({'status': 'error', 'message': code}, status=status)
    except Exception as e:
        logger.error(f'api_merchant_get_latest_payment_order error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_merchant_mark_payment_order_paid(request):
    try:
        data = await request.json()
        resolved = await resolve_effective_merchant_session(request, data)
        order_id = _safe_int(data.get('order_id'))
        if not order_id:
            return web.json_response({'status': 'error', 'message': 'order_id 不能为空'}, status=400)
        order = await payment_repo.get_payment_order(order_id)
        if not order:
            return web.json_response({'status': 'error', 'message': 'payment_order_not_found'}, status=404)
        if int(order['merchant_id']) != int(resolved['merchant_id']):
            return web.json_response({'status': 'error', 'message': 'forbidden'}, status=403)
        ok = await get_subscription_lifecycle_service().mark_order_paid(order_id)
        await get_audit_service().log(
            tenant_id=resolved['tenant_id'],
            merchant_id=resolved['merchant_id'],
            operator_role='merchant_owner',
            action='payment_marked_paid',
            resource_type='payment_order',
            resource_id=str(order_id),
            payload={'amount_usdt': order.get('amount_usdt'), 'plan_code': order.get('plan_code')},
        )
        return web.json_response({'status': 'ok' if ok else 'error', 'message': '已标记待审核' if ok else '更新失败'})
    except ValueError as e:
        code = str(e)
        status = 404 if code == 'merchant_not_found' else 400
        return web.json_response({'status': 'error', 'message': code}, status=status)
    except Exception as e:
        logger.error(f'api_merchant_mark_payment_order_paid error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_merchant_submit_payment_tx_hash(request):
    try:
        data = await request.json()
        resolved = await resolve_effective_merchant_session(request, data)
        order_id = _safe_int(data.get('order_id'))
        tx_hash = str(data.get('tx_hash') or '').strip()
        if not order_id:
            return web.json_response({'status': 'error', 'message': 'order_id 不能为空'}, status=400)
        if not tx_hash:
            return web.json_response({'status': 'error', 'message': 'tx_hash 不能为空'}, status=400)
        order = await payment_repo.get_payment_order(order_id)
        if not order:
            return web.json_response({'status': 'error', 'message': 'payment_order_not_found'}, status=404)
        if int(order['merchant_id']) != int(resolved['merchant_id']):
            return web.json_response({'status': 'error', 'message': 'forbidden'}, status=403)
        ok = await get_subscription_lifecycle_service().submit_tx_hash(order_id, tx_hash)
        await get_audit_service().log(
            tenant_id=resolved['tenant_id'],
            merchant_id=resolved['merchant_id'],
            operator_role='merchant_owner',
            action='payment_tx_submitted',
            resource_type='payment_order',
            resource_id=str(order_id),
            payload={'tx_hash': tx_hash, 'plan_code': order.get('plan_code')},
        )
        return web.json_response({'status': 'ok' if ok else 'error', 'message': '已提交' if ok else '提交失败'})
    except ValueError as e:
        code = str(e)
        status = 404 if code == 'merchant_not_found' else 400
        return web.json_response({'status': 'error', 'message': code}, status=status)
    except Exception as e:
        logger.error(f'api_merchant_submit_payment_tx_hash error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_merchant_get_features(request):
    try:
        resolved = await resolve_effective_merchant_session(request)
        feature_map = await get_feature_gate_service().get_feature_map(resolved['tenant_id'])
        return web.json_response({'status': 'ok', 'data': feature_map})
    except ValueError as e:
        code = str(e)
        status = 404 if code == 'merchant_not_found' else 400
        return web.json_response({'status': 'error', 'message': code}, status=status)
    except Exception as e:
        logger.error(f'api_merchant_get_features error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_merchant_get_ai_quota(request):
    try:
        resolved = await resolve_effective_merchant_session(request)
        quota = await get_quota_service().get_quota(resolved['tenant_id'])
        return web.json_response({'status': 'ok', 'data': quota})
    except ValueError as e:
        code = str(e)
        status = 404 if code == 'merchant_not_found' else 400
        return web.json_response({'status': 'error', 'message': code}, status=status)
    except Exception as e:
        logger.error(f'api_merchant_get_ai_quota error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_merchant_get_ai_usage(request):
    try:
        resolved = await resolve_effective_merchant_session(request)
        days = min(_safe_int(request.rel_url.query.get('days'), 7) or 7, 365)
        summary = await get_global_usage_summary(days=days, tenant_id=resolved['tenant_id'])
        return web.json_response({'status': 'ok', 'data': summary})
    except ValueError as e:
        code = str(e)
        status = 404 if code == 'merchant_not_found' else 400
        return web.json_response({'status': 'error', 'message': code}, status=status)
    except Exception as e:
        logger.error(f'api_merchant_get_ai_usage error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_merchant_list_audit_logs(request):
    try:
        resolved = await resolve_effective_merchant_session(request)
        action = (request.rel_url.query.get('action') or '').strip() or None
        resource_type = (request.rel_url.query.get('resource_type') or '').strip() or None
        limit = min(_safe_int(request.rel_url.query.get('limit'), 100) or 100, 200)
        logs = await get_audit_service().list_logs(
            resolved['tenant_id'],
            action=action,
            resource_type=resource_type,
            limit=limit,
        )
        return web.json_response({'status': 'ok', 'data': logs, 'total': len(logs)})
    except ValueError as e:
        code = str(e)
        status = 404 if code == 'merchant_not_found' else 400
        return web.json_response({'status': 'error', 'message': code}, status=status)
    except Exception as e:
        logger.error(f'api_merchant_list_audit_logs error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


# ── 平台支付配置管理 ──────────────────────────────────────────────

_VALID_CHAIN_TYPES = {'TRON', 'BSC'}


async def api_get_platform_payment_config(request):
    """获取平台支付配置（平台管理员）"""
    forbidden = _require_platform_admin(request)
    if forbidden:
        return forbidden
    try:
        chain_type = (request.rel_url.query.get('chain_type') or 'TRON').strip()
        if chain_type not in _VALID_CHAIN_TYPES:
            return web.json_response({'status': 'error', 'message': '不支持的链类型'}, status=400)
        config_row = await payment_repo.get_active_platform_payment_config(chain_type)
        if not config_row:
            return web.json_response({'status': 'ok', 'data': None})
        return web.json_response({
            'status': 'ok',
            'data': {
                'chain_type': config_row.get('chain_type'),
                'wallet_address': config_row.get('wallet_address'),
                'wallet_qr_image_path': config_row.get('wallet_qr_image_path'),
                'instruction_text': config_row.get('instruction_text'),
                'is_active': config_row.get('is_active'),
            },
        })
    except Exception as e:
        logger.error(f'api_get_platform_payment_config error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_upsert_platform_payment_config(request):
    """创建或更新平台支付配置（平台管理员）"""
    forbidden = _require_platform_admin(request)
    if forbidden:
        return forbidden
    try:
        data = await request.json()
        chain_type = str(data.get('chain_type') or 'TRON').strip()
        if chain_type not in _VALID_CHAIN_TYPES:
            return web.json_response({'status': 'error', 'message': '不支持的链类型'}, status=400)
        wallet_address = str(data.get('wallet_address') or '').strip()
        wallet_qr_image_path = str(data.get('wallet_qr_image_path') or '').strip() or None
        instruction_text = str(data.get('instruction_text') or '').strip()
        is_active = bool(data.get('is_active', True))

        if not wallet_address:
            return web.json_response({'status': 'error', 'message': 'wallet_address 不能为空'}, status=400)

        await payment_repo.upsert_platform_payment_config(
            chain_type=chain_type,
            wallet_address=wallet_address,
            wallet_qr_image_path=wallet_qr_image_path,
            instruction_text=instruction_text,
            is_active=is_active,
        )
        return web.json_response({'status': 'ok', 'message': '保存成功'})
    except Exception as e:
        logger.error(f'api_upsert_platform_payment_config error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_merchant_get_platform_payment_config(request):
    """商户获取平台支付配置（用于展示收款地址，需登录）"""
    try:
        await resolve_effective_merchant_session(request)
        chain_type = (request.rel_url.query.get('chain_type') or 'TRON').strip()
        if chain_type not in _VALID_CHAIN_TYPES:
            return web.json_response({'status': 'error', 'message': '不支持的链类型'}, status=400)
        config_row = await payment_repo.get_active_platform_payment_config(chain_type)
        if not config_row:
            return web.json_response({'status': 'ok', 'data': None})
        return web.json_response({
            'status': 'ok',
            'data': {
                'chain_type': config_row.get('chain_type'),
                'wallet_address': config_row.get('wallet_address'),
                'wallet_qr_image_path': config_row.get('wallet_qr_image_path'),
                'instruction_text': config_row.get('instruction_text'),
            },
        })
    except ValueError as e:
        code = str(e)
        status = 404 if code == 'merchant_not_found' else 400
        return web.json_response({'status': 'error', 'message': code}, status=status)
    except Exception as e:
        logger.error(f'api_merchant_get_platform_payment_config error: {e}', exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)
