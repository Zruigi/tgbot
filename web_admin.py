# -*- coding: utf-8 -*-
"""
内置 Web 管理端，提供管理员来访用户界面
"""
import aiohttp
from aiohttp import web
import asyncio
import os
import json
import logging
import io
import hashlib
import base64
from datetime import datetime, timedelta
from zoneinfo import ZoneInfo
import database
import config
import weakref
import re
import aiosqlite
import time
import secrets
import string
import hmac
from urllib.parse import urlencode, quote, parse_qsl
from html import escape
from typing import Optional, Dict, Any
from backend.repositories import merchant_repo
from services.secret_manager import SecretManager

# OTP 一次性登录凭证存储: {otp: {token, expire_at, role, tenant_id, merchant_id, issued_for}}
_otp_store: Dict[str, dict] = {}

# CSRF Token 存储 {csrf_token: expire_timestamp}
_csrf_token_store: Dict[str, float] = {}

# AI Agent 会话缓存。key = tenant/user/channel，value = {"ctx": AgentContext, "pending": AgentResponse}
_ai_agent_sessions: Dict[str, Dict[str, Any]] = {}
AI_ASSISTANT_USER_ID = config.AI_ASSISTANT_USER_ID

ENTRY_GRANT_TTL_SECONDS = 30 * 86400
ENTRY_PROOF_COOKIE = 'entry_proof'
LAST_SCOPE_COOKIE = 'miniapp_scope'
MINIAPP_REOPEN_STATUS = 'reopen_in_telegram_required'


def _env_flag_enabled(name: str, default: str = '') -> bool:
    value = str(os.getenv(name, default) or '').strip().lower()
    return value in {'1', 'true', 'yes', 'on'}


def _allow_local_auth_bypass(request: web.Request) -> bool:
    """Only allow local unauthenticated API bypass when explicitly enabled for development."""
    if not _env_flag_enabled('ALLOW_LOCAL_API_BYPASS'):
        return False
    real_ip = request.headers.get('X-Real-IP', '')
    forwarded_for = request.headers.get('X-Forwarded-For', '')
    return not real_ip and not forwarded_for and request.remote in {'127.0.0.1', '::1'}


def _frontend_dist_dir() -> str:
    """Return the built frontend directory. Vite production output is static/dist."""
    base_dir = os.path.dirname(os.path.abspath(__file__))
    candidates = [
        os.path.join(base_dir, 'static', 'dist'),
        os.path.join(base_dir, 'frontend', 'dist'),
    ]
    for candidate in candidates:
        if os.path.exists(os.path.join(candidate, 'index.html')):
            return candidate
    return candidates[0]


def _saas_backend_url() -> str:
    return str(os.getenv('SAAS_BACKEND_URL') or '').strip().rstrip('/')


def _saas_proxy_enabled() -> bool:
    return bool(_saas_backend_url())


def _is_saas_proxy_path(path: str) -> bool:
    return (
        path == '/api/session/context'
        or path.startswith('/api/platform/')
        or path == '/api/platform'
        or path.startswith('/api/merchant/')
        or path == '/api/merchant'
    )


def _build_session_token_payload(
    *,
    role: str = 'platform_admin',
    tenant_id: str = '',
    merchant_id: Optional[int] = None,
    issued_for: Optional[str] = None,
) -> Dict[str, Any]:
    return {
        'role': str(role or 'platform_admin'),
        'actor_role': str(role or 'platform_admin'),
        'tenant_id': str(tenant_id or ''),
        'merchant_id': int(merchant_id) if merchant_id is not None else None,
        'issued_for': str(issued_for or ''),
        'iat': int(time.time()),
    }


def _get_session_signing_secret() -> str:
    return hashlib.sha256(str(config.WEB_ADMIN_TOKEN).encode('utf-8')).hexdigest()


def _b64url_encode(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).decode().rstrip('=')


def _b64url_decode(data: str) -> bytes:
    padding = '=' * (-len(data) % 4)
    return base64.urlsafe_b64decode((data + padding).encode())


def _issue_token(
    *,
    role: str = 'platform_admin',
    tenant_id: str = '',
    merchant_id: Optional[int] = None,
    issued_for: Optional[str] = None,
) -> str:
    import hmac

    payload = _build_session_token_payload(
        role=role,
        tenant_id=tenant_id,
        merchant_id=merchant_id,
        issued_for=issued_for,
    )
    payload_json = json.dumps(payload, ensure_ascii=False, separators=(',', ':')).encode('utf-8')
    payload_b64 = _b64url_encode(payload_json)
    sig = hmac.new(
        _get_session_signing_secret().encode('utf-8'),
        payload_b64.encode('utf-8'),
        hashlib.sha256,
    ).hexdigest()
    return f'{payload_b64}.{sig}'


def _verify_session_token(token: str) -> Optional[Dict[str, Any]]:
    import hmac

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


def _extract_session_context_from_request(request) -> Dict[str, Any]:
    auth_header = request.headers.get('Authorization') or ''
    cookie_token = request.cookies.get('admin_token') or ''
    raw_token = ''
    if auth_header.startswith('Bearer '):
        raw_token = auth_header.split(' ', 1)[1].strip()
    elif cookie_token:
        raw_token = cookie_token.strip()

    payload = _verify_session_token(raw_token)
    if payload:
        payload['authenticated'] = True
        return payload

    # Token 验证失败时的安全回退: 不再默认返回 platform_admin
    # 使用 authenticated=False 明确标记未认证状态
    if raw_token and raw_token == config.WEB_ADMIN_TOKEN:
        # 兼容旧版直接使用 BOT_TOKEN 的情况
        result = _build_session_token_payload(role='platform_admin')
        result['authenticated'] = True
        return result

    # 未提供有效 Token，返回未认证状态（需要登录）
    return {
        'role': 'platform_admin',
        'tenant_id': '',
        'merchant_id': None,
        'issued_for': '',
        'iat': 0,
        'authenticated': False,  # 关键: 明确标记为未认证
    }


def generate_csrf_token() -> str:
    now = time.time()
    expired = [k for k, v in _csrf_token_store.items() if v < now]
    for k in expired:
        _csrf_token_store.pop(k, None)
    csrf_token = secrets.token_urlsafe(32)
    _csrf_token_store[csrf_token] = now + 86400
    return csrf_token


async def generate_otp(
    *,
    role: str = 'platform_admin',
    tenant_id: str = '',
    merchant_id: Optional[int] = None,
    issued_for: Optional[str] = None,
    ttl_seconds: int = 300,
) -> str:
    now = time.time()
    expired = [k for k, v in _otp_store.items() if v['expire_at'] < now]
    for k in expired:
        del _otp_store[k]

    alphabet = string.ascii_letters + string.digits
    otp = ''.join(secrets.choice(alphabet) for _ in range(12))
    session_token = _issue_token(
        role=role,
        tenant_id=tenant_id,
        merchant_id=merchant_id,
        issued_for=issued_for,
    )
    _otp_store[otp] = {
        'token': session_token,
        'expire_at': now + ttl_seconds,
        'role': role,
        'tenant_id': tenant_id,
        'merchant_id': merchant_id,
        'issued_for': issued_for,
    }

    # 持久化到数据库，确保重启后 OTP 仍然有效
    otp_hash = hashlib.sha256(otp.encode()).hexdigest()
    expires_at = datetime.fromtimestamp(now + ttl_seconds).isoformat()
    try:
        await database.save_web_login_otp(
            otp_hash,
            role=role,
            tenant_id=tenant_id,
            merchant_id=merchant_id,
            issued_for=issued_for,
            expires_at=expires_at,
            session_token=session_token,
        )
    except Exception as e:
        logger.warning(f"保存 OTP 到数据库失败: {e}")

    return otp


def _normalize_entry_intent(value: Optional[str]) -> str:
    intent = str(value or 'menu').strip().lower()
    if intent not in {'menu', 'admin_console', 'merchant_console', 'support'}:
        return 'menu'
    return intent


def _normalize_role_hint(value: Optional[str]) -> str:
    role = str(value or 'platform_admin').strip().lower()
    if role not in {'platform_admin', 'merchant_owner'}:
        return 'platform_admin'
    return role


def _hash_grant(grant: str) -> str:
    return hashlib.sha256(str(grant or '').encode('utf-8')).hexdigest()


def _cookie_secure(request: Optional[web.Request] = None) -> bool:
    if request is not None:
        proto = str(request.headers.get('X-Forwarded-Proto') or request.scheme or '').strip().lower()
        return proto == 'https'
    return str(config.FRONTEND_URL or '').startswith('https://')


def _extract_user_agent(request: Optional[web.Request], client_context: Optional[Dict[str, Any]] = None) -> str:
    if client_context and client_context.get('user_agent'):
        return str(client_context.get('user_agent') or '')
    if request is not None:
        return str(request.headers.get('User-Agent') or '')
    return ''


def _ua_fingerprint(user_agent: str) -> str:
    return hashlib.sha256(str(user_agent or '').encode('utf-8')).hexdigest()[:32]


async def generate_entry_grant(
    *,
    issued_for: str,
    role_hint: str = 'platform_admin',
    tenant_id: str = '',
    merchant_id: Optional[int] = None,
    entry_intent: str = 'menu',
    ua_fingerprint: str = '',
    ttl_seconds: int = ENTRY_GRANT_TTL_SECONDS,
) -> Dict[str, Any]:
    grant = secrets.token_urlsafe(32)
    grant_hash = _hash_grant(grant)
    expires_at = datetime.fromtimestamp(time.time() + ttl_seconds).isoformat()
    proof_salt = secrets.token_urlsafe(24)
    await database.save_web_entry_grant(
        grant_hash,
        issued_for=str(issued_for or ''),
        role_hint=_normalize_role_hint(role_hint),
        tenant_id=str(tenant_id or ''),
        merchant_id=int(merchant_id) if merchant_id is not None else None,
        entry_intent=_normalize_entry_intent(entry_intent),
        ua_fingerprint=str(ua_fingerprint or ''),
        proof_salt=proof_salt,
        expires_at=expires_at,
    )
    return {
        'grant': grant,
        'grant_hash': grant_hash,
        'expires_at': expires_at,
    }


async def _resolve_entry_grant(grant: str) -> Optional[Dict[str, Any]]:
    grant_hash = _hash_grant(grant)
    row = await database.get_web_entry_grant(grant_hash)
    if not row:
        return None
    try:
        expires_at = datetime.fromisoformat(str(row.get('expires_at') or ''))
    except Exception:
        return None
    if expires_at <= datetime.now():
        return None
    row['grant_hash'] = grant_hash
    return row


def _issue_entry_proof_cookie_value(grant_row: Dict[str, Any], ua_fingerprint: str) -> str:
    payload = {
        'grant_hash': str(grant_row.get('grant_hash') or ''),
        'issued_for': str(grant_row.get('issued_for') or ''),
        'ua_fingerprint': str(ua_fingerprint or ''),
        'iat': int(time.time()),
    }
    payload_b64 = _b64url_encode(json.dumps(payload, separators=(',', ':')).encode('utf-8'))
    proof_secret = f"{_get_session_signing_secret()}:{grant_row.get('proof_salt') or ''}"
    sig = hmac.new(
        proof_secret.encode('utf-8'),
        payload_b64.encode('utf-8'),
        hashlib.sha256,
    ).hexdigest()
    return f'{payload_b64}.{sig}'


def _verify_entry_proof_cookie_value(value: str, grant_row: Dict[str, Any], ua_fingerprint: str) -> bool:
    try:
        payload_b64, sig = str(value or '').split('.', 1)
    except ValueError:
        return False

    proof_secret = f"{_get_session_signing_secret()}:{grant_row.get('proof_salt') or ''}"
    expected_sig = hmac.new(
        proof_secret.encode('utf-8'),
        payload_b64.encode('utf-8'),
        hashlib.sha256,
    ).hexdigest()
    if not hmac.compare_digest(sig, expected_sig):
        return False

    try:
        payload = json.loads(_b64url_decode(payload_b64).decode('utf-8'))
    except Exception:
        return False

    return (
        isinstance(payload, dict)
        and str(payload.get('grant_hash') or '') == str(grant_row.get('grant_hash') or '')
        and str(payload.get('issued_for') or '') == str(grant_row.get('issued_for') or '')
        and str(payload.get('ua_fingerprint') or '') == str(ua_fingerprint or '')
    )


def build_miniapp_url(
    *,
    entry_intent: str = 'menu',
    grant: Optional[str] = None,
    otp: Optional[str] = None,
    tenant_hint: Optional[str] = None,
    merchant_id_hint: Optional[int] = None,
) -> str:
    frontend_url = (config.FRONTEND_URL or '').strip().rstrip('/')
    if not frontend_url:
        raise ValueError('frontend_url_unavailable')
    params = {'entry': _normalize_entry_intent(entry_intent)}
    if grant:
        params['grant'] = str(grant)
    if otp:
        params['otp'] = str(otp)
    if tenant_hint:
        params['tenant'] = str(tenant_hint)
    if merchant_id_hint is not None:
        params['merchant_id'] = str(int(merchant_id_hint))
    return f"{frontend_url}/miniapp?{urlencode(params, quote_via=quote)}"

logger = logging.getLogger(__name__)

# 全局存储 bot 实例以发送消息
_bot_instance = None

# WebSocket 客户端连接池
ws_clients = weakref.WeakSet()

# 2026 规格：发送队列计数器与性能统计
active_send_tasks = 0
last_performance_metrics = {}

# P2-4 任务调度器状态
_push_scheduler_task = None
_push_scheduler_started = False

# 标签缓存与过期时间
_tags_cache: Dict[str, Dict[str, Any]] = {}
# 成员头像 file_id 缓存（避免频繁打 Telegram API），TTL 10分钟，上限 2000 条
from cachetools import TTLCache as _TTLCache
_member_avatar_cache: _TTLCache = _TTLCache(maxsize=2000, ttl=600)

# 图片上传大小限制：10MB
_MAX_UPLOAD_SIZE = 10 * 1024 * 1024

# 后台任务引用集合：防止 fire-and-forget 任务被 GC 回收
_web_bg_tasks: set = set()

def set_bot_instance(bot):
    global _bot_instance
    _bot_instance = bot

async def websocket_handler(request):
    """处理 WebSocket 连接"""
    ws = web.WebSocketResponse()
    await ws.prepare(request)
    
    ws_clients.add(ws)
    logger.info("Web 管理端: WebSocket 客户端已连接")
    
    try:
        async for msg in ws:
            if msg.type == aiohttp.WSMsgType.TEXT:
                if msg.data == 'ping':
                    await ws.send_str('pong')
            elif msg.type == aiohttp.WSMsgType.ERROR:
                logger.error(f"WebSocket connection closed with exception {ws.exception()}")
    finally:
        ws_clients.discard(ws)
        logger.info("Web 管理端: WebSocket 客户端已断开")
        
    return ws

async def notify_web_clients(event_type: str, data: Optional[Dict] = None):
    """向所有连接的 WebSocket 客户端广播消息"""
    if not ws_clients:
        logger.debug(f"WS 推送跳过 (无客户端连接): {event_type}")
        return
        
    safe_data = data if data is not None else {}
    message = json.dumps({"type": event_type, "data": safe_data})
    logger.info(f"📡 [WS 推送] 类型={event_type} 数据={safe_data}")
    dead_clients = set()
    
    for ws in ws_clients:
        try:
            await ws.send_str(message)
        except Exception as e:
            logger.error(f"WebSocket 发送消息失败: {e}")
            dead_clients.add(ws)
            
    for ws in dead_clients:
        ws_clients.discard(ws)


async def _fetch_member_avatar_file_id(user_id: int) -> Optional[str]:
    """Get member avatar file_id with TTLCache."""
    if not _bot_instance or not user_id:
        return None

    cached = _member_avatar_cache.get(int(user_id))
    if cached is not None:
        return cached

    file_id = None
    try:
        photos = await _bot_instance.get_user_profile_photos(user_id=user_id, limit=1)
        if photos and photos.total_count > 0 and photos.photos and photos.photos[0]:
            file_id = photos.photos[0][-1].file_id
    except Exception:
        pass

    if not file_id:
        try:
            chat = await _bot_instance.get_chat(user_id)
            if chat and chat.photo:
                file_id = chat.photo.big_file_id
        except Exception:
            file_id = None

    _member_avatar_cache[int(user_id)] = file_id
    return file_id


async def _sync_broadcast_targets_from_bot() -> dict:
    """从 Telegram 拉取 Bot 可见群聊/频道并同步到广播目标表"""
    if not _bot_instance:
        return {"scanned": 0, "updated": 0}

    updated = 0
    scanned = 0
    groups = await database.get_tracked_groups_with_stats()
    for g in groups:
        scanned += 1
        try:
            chat = await _bot_instance.get_chat(int(g['group_id']))
            await database.upsert_broadcast_target(
                chat_id=int(g['group_id']),
                chat_title=getattr(chat, 'title', None) or g.get('group_title') or str(g['group_id']),
                chat_type=getattr(chat, 'type', None) or 'supergroup',
                chat_username=getattr(chat, 'username', None),
                can_broadcast=True,
            )
            updated += 1
        except Exception:
            # 群已不可见或无权限时跳过
            continue

    return {"scanned": scanned, "updated": updated}


def _dedupe_int_values(values) -> list[int]:
    """Return a stable, deduplicated list of integer ids from mixed frontend input."""
    result: list[int] = []
    seen = set()
    for value in values or []:
        raw = str(value).strip()
        if not raw:
            continue
        try:
            parsed = int(raw)
        except (TypeError, ValueError):
            continue
        if parsed not in seen:
            seen.add(parsed)
            result.append(parsed)
    return result


def _build_broadcast_reply_markup(buttons):
    """Build Telegram inline keyboard rows from frontend broadcast button specs."""
    def normalize_button_url(raw_url: str) -> str:
        raw_url = str(raw_url or '').strip()
        if re.match(r'^https?://', raw_url, flags=re.IGNORECASE):
            return raw_url
        if re.match(r'^@[A-Za-z0-9_]{5,32}$', raw_url):
            return f"https://t.me/{raw_url[1:]}"
        if re.match(r'^[A-Za-z0-9_]{5,32}$', raw_url):
            return f"https://t.me/{raw_url}"
        raise ValueError('按钮链接必须以 http://、https:// 或 Telegram 用户名开头')

    cleaned = []
    for button in buttons or []:
        if not isinstance(button, dict):
            continue
        text = str(button.get('text') or '').strip()
        raw_url = str(button.get('url') or '').strip()
        if not text and not raw_url:
            continue
        if not text or not raw_url:
            raise ValueError('按钮文字和链接不能为空')
        url = normalize_button_url(raw_url)
        cleaned.append({'text': text[:64], 'url': url})
        if len(cleaned) >= 6:
            break

    if not cleaned:
        return None

    from telegram import InlineKeyboardButton, InlineKeyboardMarkup
    rows = []
    for i in range(0, len(cleaned), 2):
        rows.append([
            InlineKeyboardButton(item['text'], url=item['url'])
            for item in cleaned[i:i + 2]
        ])
    return InlineKeyboardMarkup(rows)


async def _send_broadcast_to_chats(
    chat_ids,
    content,
    image_file=None,
    do_translate=False,
    retries=1,
    reply_markup=None,
    prechecked_text=None,
):
    """面向群/频道执行广播，返回成功失败汇总"""
    if not _bot_instance:
        return {"total": 0, "success": 0, "failed": 0, "success_ids": [], "failed_items": []}

    if prechecked_text is None:
        from services.security_center import security_check_flow
        is_passed, verified_text, error_msg, _, _ = await security_check_flow(
            content or '', image_bytes=image_file, do_translate=do_translate
        )
        if not is_passed:
            raise ValueError(error_msg or '安全校验失败')
    else:
        verified_text = prechecked_text

    success_ids = []
    failed_items = []
    normalized_chat_ids = _dedupe_int_values(chat_ids)

    for chat_id in normalized_chat_ids:
        attempt = 0
        sent_ok = False
        last_err = None
        while attempt <= retries and not sent_ok:
            attempt += 1
            try:
                if image_file:
                    # 复用同一个 BytesIO，每次 seek 回开头，避免 N 份内存拷贝
                    if attempt == 1:
                        bio = io.BytesIO(image_file)
                        bio.name = 'broadcast.jpg'
                    bio.seek(0)
                    await _bot_instance.send_photo(
                        chat_id=chat_id,
                        photo=bio,
                        caption=verified_text or None,
                        parse_mode="HTML" if verified_text else None,
                        reply_markup=reply_markup,
                    )
                else:
                    await _bot_instance.send_message(
                        chat_id=chat_id,
                        text=verified_text,
                        parse_mode="HTML",
                        reply_markup=reply_markup,
                    )
                sent_ok = True
                success_ids.append(chat_id)
            except Exception as ex:
                last_err = str(ex)
                if attempt <= retries:
                    await asyncio.sleep(0.5)

        if not sent_ok:
            failed_items.append({"chat_id": chat_id, "reason": last_err or 'unknown'})

    return {
        "total": len(normalized_chat_ids),
        "success": len(success_ids),
        "failed": len(failed_items),
        "success_ids": success_ids,
        "failed_items": failed_items,
    }


async def _run_push_scheduler_loop():
    """P2-4：定时任务执行循环"""
    while True:
        try:
            now = datetime.now()
            due_tasks = await database.get_due_push_tasks(now.isoformat())
            for task in due_tasks:
                task_id = int(task['task_id'])
                targets = task.get('target_chat_ids') or []
                interval_hours = max(int(task.get('interval_hours') or 1), 1)
                try:
                    summary = await _send_broadcast_to_chats(
                        chat_ids=targets,
                        content=(task.get('content') or ''),
                        image_file=None,
                        do_translate=bool(task.get('do_translate')),
                        retries=1,
                    )
                    next_run = now + timedelta(hours=interval_hours)
                    await database.update_push_task_run_result(
                        task_id=task_id,
                        last_run_at=now.isoformat(),
                        next_run_at=next_run.isoformat(),
                        summary=summary,
                        retry_count=0,
                    )
                    await notify_web_clients('push_task_update', {"task_id": task_id, "summary": summary, "next_run_at": next_run.isoformat()})
                except Exception as ex:
                    fail_summary = {"total": len(targets), "success": 0, "failed": len(targets), "failed_items": [{"reason": str(ex)}]}
                    next_run = now + timedelta(hours=interval_hours)
                    await database.update_push_task_run_result(
                        task_id=task_id,
                        last_run_at=now.isoformat(),
                        next_run_at=next_run.isoformat(),
                        summary=fail_summary,
                        retry_count=int(task.get('retry_count') or 0) + 1,
                    )
            await asyncio.sleep(5)
        except Exception as loop_ex:
            logger.error('push scheduler loop error: %s', loop_ex)
            await asyncio.sleep(5)


# --- Auth Middleware ---
@web.middleware
async def auth_middleware(request, handler):
    """
    2026 规格：Bearer Token 校验中间件。
    同时将 session context 注入到 request['session'] 中供后端 API 使用。
    """
    public_paths = {
        '/',
        '/miniapp',
        '/favicon.ico',
        '/healthz',
        '/ws',
        '/api/login',
        '/api/login/otp',
        '/api/login/tma',
        '/api/health',
        '/api/csrf-token',
        '/api/session/context',
        '/api/miniapp/bootstrap',
        '/api/miniapp/context',
        '/api/miniapp/logout_revoke',
    }
    session_management_paths = {'/api/session/context'}
    is_spa_document_path = (
        request.method in {'GET', 'HEAD'}
        and not request.path.startswith((
            '/api',
            '/assets/',
            '/static/',
            '/ws',
            '/v1/',
            '/miniapp',
        ))
    )
    if request.path in public_paths or request.path.startswith('/api/file/') or request.path.startswith('/static/') or request.path.startswith('/assets/'):
        return await handler(request)
    if is_spa_document_path:
        return await handler(request)
    if _saas_proxy_enabled() and _is_saas_proxy_path(request.path):
        return await handler(request)

    auth_header = request.headers.get('Authorization', '')
    cookie_token = request.cookies.get('admin_token', '')

    token = None
    if auth_header.startswith('Bearer '):
        token = auth_header.split(' ', 1)[1]
    elif cookie_token:
        token = cookie_token

    session_ctx = _extract_session_context_from_request(request)

    # 将 session context 注入 request 供后端 API 使用
    request['session'] = session_ctx

    # 商户不应访问平台共享后台接口；平台管理员也不应在此入口切换为商户视角。
    if request.path in session_management_paths and session_ctx.get('authenticated', False):
        if request.method != 'GET':
            return web.json_response({"status": "error", "message": "session_context_switch_disabled"}, status=403)

    merchant_blocked_prefixes = (
        '/api/users',
        '/api/messages',
        '/api/broadcast',
        '/api/push_tasks',
        '/api/monitors',
        '/api/potential_users',
        '/api/group_listen',
        '/api/groups',
        '/api/group_members',
        '/api/group_messages',
        '/api/config',
        '/api/openai',
        '/v1/',
    )
    if session_ctx.get('authenticated', False) and session_ctx.get('role') == 'merchant_owner':
        if any(request.path.startswith(prefix) for prefix in merchant_blocked_prefixes):
            return web.json_response({"status": "error", "message": "merchant_console_forbidden"}, status=403)

    if not session_ctx.get('authenticated', False):
        if token != config.WEB_ADMIN_TOKEN:
            if _allow_local_auth_bypass(request):
                return await handler(request)
            real_ip = request.headers.get('X-Real-IP', '')
            logger.warning(f"拒绝未授权访问: {request.path} 来自 {request.remote} real={real_ip}")
            return web.json_response({"status": "error", "message": "Unauthorized"}, status=401)

    if _verify_session_token(token or '') is not None or token == config.WEB_ADMIN_TOKEN:
        return await handler(request)

    if _allow_local_auth_bypass(request):
        return await handler(request)

    real_ip = request.headers.get('X-Real-IP', '')
    logger.warning(f"拒绝未授权访问: {request.path} 来自 {request.remote} real={real_ip}")
    return web.json_response({"status": "error", "message": "Unauthorized"}, status=401)


def _request_scope_context(request) -> Dict[str, Any]:
    session = request.get('session') or {}
    role = str(session.get('role') or '').strip()
    return {
        'authenticated': bool(session.get('authenticated', False)),
        'role': role,
        'tenant_id': str(session.get('tenant_id') or '').strip(),
        'merchant_id': int(session.get('merchant_id')) if session.get('merchant_id') is not None else None,
        'is_merchant': role == 'merchant_owner' and bool(str(session.get('tenant_id') or '').strip()),
    }


async def _resolve_ai_chat_counterpart_id(scope: Dict[str, Any]) -> int:
    """Resolve the local admin/bot side used to persist the virtual Agent chat."""
    if scope.get('is_merchant') and scope.get('merchant_id') is not None:
        try:
            async with aiosqlite.connect(database.DB_NAME) as db:
                async with db.execute(
                    "SELECT bot_id FROM merchants WHERE id = ? LIMIT 1",
                    (int(scope['merchant_id']),),
                ) as cursor:
                    row = await cursor.fetchone()
                    if row and row[0]:
                        return int(row[0])
        except Exception as exc:
            logger.debug("resolve ai counterpart bot_id failed: %s", exc)
    return int(config.ADMIN_ID)


async def _resolve_business_window() -> Dict[str, Any]:
    timezone_name = str(await database.get_admin_setting('miniapp_timezone', 'Asia/Shanghai') or 'Asia/Shanghai').strip() or 'Asia/Shanghai'
    start = str(await database.get_admin_setting('miniapp_business_start', '09:00') or '09:00').strip() or '09:00'
    end = str(await database.get_admin_setting('miniapp_business_end', '22:00') or '22:00').strip() or '22:00'

    try:
        tz = ZoneInfo(timezone_name)
    except Exception:
        timezone_name = 'Asia/Shanghai'
        tz = ZoneInfo(timezone_name)

    now_local = datetime.now(tz)
    current_minutes = now_local.hour * 60 + now_local.minute

    def _parse_minutes(raw: str, fallback: int) -> int:
        try:
            hour, minute = str(raw).split(':', 1)
            return int(hour) * 60 + int(minute)
        except Exception:
            return fallback

    start_minutes = _parse_minutes(start, 9 * 60)
    end_minutes = _parse_minutes(end, 22 * 60)
    if end_minutes < start_minutes:
        in_window = current_minutes >= start_minutes or current_minutes < end_minutes
    else:
        in_window = start_minutes <= current_minutes < end_minutes

    return {
        'timezone': timezone_name,
        'start': start,
        'end': end,
        'active': bool(in_window),
        'now': now_local.isoformat(),
    }


async def _authenticate_tma_init_data(
    init_data: str,
    *,
    grant_row: Optional[Dict[str, Any]] = None,
    tenant_hint: str = '',
    merchant_id_hint: Optional[int] = None,
) -> Dict[str, Any]:
    init_data = str(init_data or '').strip()
    if not init_data:
        return {'ok': False, 'message': 'init_data 不能为空'}

    bot_token_for_verify = await _resolve_tma_verification_token(
        grant_row=grant_row,
        tenant_hint=tenant_hint,
        merchant_id_hint=merchant_id_hint,
    )
    if not bot_token_for_verify:
        return {'ok': False, 'message': '服务端未配置 Bot Token', 'status_code': 500}
    pairs = dict(parse_qsl(init_data, keep_blank_values=True))

    received_hash = pairs.pop('hash', '')
    if not received_hash:
        return {'ok': False, 'message': 'init_data 缺少 hash'}

    sorted_pairs = sorted(pairs.items())
    check_string = '\n'.join(f'{k}={v}' for k, v in sorted_pairs)
    secret_key = hmac.new(
        b"WebAppData",
        bot_token_for_verify.encode(),
        hashlib.sha256,
    ).digest()
    computed_hash = hmac.new(
        secret_key,
        check_string.encode(),
        hashlib.sha256,
    ).hexdigest()
    if computed_hash != received_hash:
        logger.warning("TMA initData HMAC 验证失败: hash mismatch")
        return {'ok': False, 'message': 'init_data 验证失败', 'status_code': 401}

    user_json = pairs.get('user', '')
    if not user_json:
        return {'ok': False, 'message': 'init_data 缺少 user'}

    user_info = json.loads(user_json)
    tg_user_id = int(user_info.get('id', 0))
    if not tg_user_id:
        return {'ok': False, 'message': '无效的 user id'}

    expected_merchant = None
    effective_merchant_id = _safe_int((grant_row or {}).get('merchant_id')) or merchant_id_hint
    if effective_merchant_id is not None:
        expected_merchant = await merchant_repo.get_merchant_by_id(int(effective_merchant_id))

    merchant = await merchant_repo.get_merchant_by_owner(tg_user_id)
    if expected_merchant and int(expected_merchant.get('owner_tg_user_id') or 0) == tg_user_id:
        merchant = expected_merchant
    is_platform_admin = tg_user_id in config.ADMIN_TG_IDS
    return {
        'ok': True,
        'tg_user_id': tg_user_id,
        'merchant': merchant,
        'is_platform_admin': is_platform_admin,
        'user': user_info,
        'start_param': str(pairs.get('start_param') or ''),
        'verified_tenant_id': str((grant_row or {}).get('tenant_id') or tenant_hint or ''),
        'verified_merchant_id': effective_merchant_id,
    }


async def _consume_otp_session(otp: str) -> Dict[str, Any]:
    otp = str(otp or '').strip()
    if not otp:
        return {'ok': False, 'reason': 'missing'}

    now = time.time()
    otp_data = _otp_store.get(otp)
    if otp_data and otp_data['expire_at'] > now:
        _otp_store.pop(otp, None)
        payload = _verify_session_token(otp_data['token']) or {}
        return {
            'ok': True,
            'token': otp_data['token'],
            'session': {
                'role': payload.get('role', otp_data.get('role', 'platform_admin')),
                'tenant_id': payload.get('tenant_id', otp_data.get('tenant_id', '')),
                'merchant_id': payload.get('merchant_id', otp_data.get('merchant_id')),
                'issued_for': payload.get('issued_for', otp_data.get('issued_for', '')),
                'actor_role': payload.get('role', otp_data.get('role', 'platform_admin')),
            },
            'source': 'otp',
        }

    _otp_store.pop(otp, None)
    otp_hash = hashlib.sha256(otp.encode()).hexdigest()
    db_otp = await database.get_web_login_otp(otp_hash)
    if db_otp:
        try:
            expires_at = datetime.fromisoformat(db_otp['expires_at'])
        except Exception:
            expires_at = datetime.min
        if expires_at > datetime.now():
            session_token = db_otp.get('session_token')
            if session_token:
                payload = _verify_session_token(session_token)
                if not payload:
                    return {'ok': False, 'reason': 'session_expired'}
                await database.consume_web_login_otp(otp_hash)
                return {
                    'ok': True,
                    'token': session_token,
                    'session': {
                        'role': payload.get('role', 'platform_admin'),
                        'tenant_id': payload.get('tenant_id', ''),
                        'merchant_id': payload.get('merchant_id'),
                        'issued_for': payload.get('issued_for', db_otp.get('issued_for', '')),
                        'actor_role': payload.get('role', 'platform_admin'),
                    },
                    'source': 'otp',
                }
        return {'ok': False, 'reason': 'expired'}

    return {'ok': False, 'reason': 'invalid'}


def _safe_int(value: Any) -> Optional[int]:
    try:
        if value is None or value == '':
            return None
        return int(value)
    except Exception:
        return None


def _extract_request_auth_hints(
    data: Optional[Dict[str, Any]] = None,
    client_context: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    payload = data or {}
    ctx = client_context or {}
    tenant_hint = str(payload.get('tenant_hint') or '').strip()
    merchant_id_hint = _safe_int(payload.get('merchant_id_hint'))
    grant = str(payload.get('grant') or '').strip()

    raw_search = str(ctx.get('search') or '').strip()
    if raw_search.startswith('?'):
        raw_search = raw_search[1:]
    if raw_search:
        for key, value in parse_qsl(raw_search, keep_blank_values=True):
            if key == 'tenant' and not tenant_hint:
                tenant_hint = str(value or '').strip()
            elif key == 'merchant_id' and merchant_id_hint is None:
                merchant_id_hint = _safe_int(value)
            elif key == 'grant' and not grant:
                grant = str(value or '').strip()

    return {
        'tenant_hint': tenant_hint,
        'merchant_id_hint': merchant_id_hint,
        'grant': grant,
    }


async def _resolve_tma_verification_token(
    *,
    grant_row: Optional[Dict[str, Any]] = None,
    tenant_hint: str = '',
    merchant_id_hint: Optional[int] = None,
) -> str:
    effective_tenant_id = str((grant_row or {}).get('tenant_id') or tenant_hint or '').strip()
    effective_merchant_id = _safe_int((grant_row or {}).get('merchant_id')) or merchant_id_hint

    if not effective_tenant_id and effective_merchant_id is not None:
        merchant = await merchant_repo.get_merchant_by_id(int(effective_merchant_id))
        if merchant:
            effective_tenant_id = str(merchant.get('tenant_id') or '').strip()

    if effective_tenant_id:
        merchant_bot_token = str(
            await SecretManager.get_config('TELEGRAM_BOT_TOKEN', tenant_id=effective_tenant_id, default='')
            or ''
        ).strip()
        if merchant_bot_token:
            return merchant_bot_token

    return str(config.BOT_TOKEN or '').strip()


def _resolve_identity_scope(
    *,
    entry_intent: str,
    is_platform_admin: bool,
    merchant: Optional[Dict[str, Any]],
    last_scope: str = '',
) -> Dict[str, Any]:
    entry_intent = _normalize_entry_intent(entry_intent)
    last_scope = _normalize_role_hint(last_scope or 'platform_admin')
    has_merchant = merchant is not None

    if entry_intent == 'merchant_console':
        if has_merchant:
            return {'role': 'merchant_owner', 'reason': 'merchant_console_entry'}
        if is_platform_admin:
            return {'role': 'platform_admin', 'reason': 'merchant_entry_fallback_platform'}

    if entry_intent == 'admin_console':
        if is_platform_admin:
            return {'role': 'platform_admin', 'reason': 'admin_console_entry'}
        if has_merchant:
            return {'role': 'merchant_owner', 'reason': 'admin_entry_fallback_merchant'}

    if is_platform_admin and has_merchant:
        return {'role': last_scope or 'platform_admin', 'reason': 'dual_identity_last_scope'}
    if is_platform_admin:
        return {'role': 'platform_admin', 'reason': 'platform_identity'}
    if has_merchant:
        return {'role': 'merchant_owner', 'reason': 'merchant_identity'}
    return {'role': 'unregistered', 'reason': 'unregistered'}


def _build_landing_payload(role: str, *, entry_intent: str, business_window: Dict[str, Any]) -> Dict[str, Any]:
    active = bool(business_window.get('active'))
    mode = 'normal' if active else 'after_hours'

    if role == 'unregistered':
        return {'route': '/onboarding', 'mode': 'register', 'reason': 'unregistered'}
    if role == 'merchant_owner':
        return {'route': '/saas', 'mode': mode, 'reason': f'{entry_intent}_merchant'}
    if entry_intent == 'admin_console':
        return {'route': '/saas', 'mode': mode, 'reason': 'platform_admin_console'}
    if entry_intent == 'merchant_console':
        return {'route': '/saas', 'mode': mode, 'reason': 'platform_merchant_console'}
    if entry_intent == 'support':
        return {'route': '/chat', 'mode': mode, 'reason': 'support_entry'}
    return {'route': '/saas', 'mode': mode, 'reason': f'{entry_intent}_platform_beta'}


def _identity_label(role: str) -> str:
    if role == 'platform_admin':
        return '平台管理员'
    if role == 'merchant_owner':
        return '商户管理员'
    return '未注册用户'


def _session_from_grant_row(grant_row: Dict[str, Any]) -> Dict[str, Any]:
    role = _normalize_role_hint(grant_row.get('role_hint'))
    return {
        'role': role,
        'tenant_id': str(grant_row.get('tenant_id') or ''),
        'merchant_id': int(grant_row.get('merchant_id')) if grant_row.get('merchant_id') is not None else None,
        'issued_for': str(grant_row.get('issued_for') or ''),
        'actor_role': role,
    }


async def api_login(request):
    """Token 登录校验接口"""
    try:
        data = await request.json()
        token = (data.get('token') or '').strip()
        if token == config.WEB_ADMIN_TOKEN:
            session_token = _issue_token(role='platform_admin')
            return web.json_response({
                "status": "ok",
                "token": session_token,
                "session": {
                    "role": "platform_admin",
                    "tenant_id": "",
                    "merchant_id": None,
                },
            })
        return web.json_response({"status": "error", "message": "Token 错误"}, status=401)
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=400)


async def api_login_otp(request):
    """OTP 登录接口"""
    try:
        data = await request.json()
        otp = (data.get('otp') or '').strip()
        if not otp:
            return web.json_response({"status": "error", "message": "otp 不能为空"}, status=400)
        otp_result = await _consume_otp_session(otp)
        if otp_result.get('ok'):
            return web.json_response({
                "status": "ok",
                "token": otp_result.get('token'),
                "session": {
                    "role": otp_result['session'].get('role', 'platform_admin'),
                    "tenant_id": otp_result['session'].get('tenant_id', ''),
                    "merchant_id": otp_result['session'].get('merchant_id'),
                    "actor_role": otp_result['session'].get('actor_role', otp_result['session'].get('role', 'platform_admin')),
                },
            })

        return web.json_response({"status": "error", "message": "OTP 无效或已过期"}, status=401)
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=400)


async def api_get_csrf_token(request):
    try:
        return web.json_response({"status": "ok", "token": generate_csrf_token()})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_login_tma(request):
    """Telegram Mini App initData 认证接口
    
    验证 Telegram 传递的 initData HMAC 签名，
    签名有效则信任其中的 user 信息，自动签发 session token。
    """
    try:
        data = await request.json()
        init_data = (data.get('init_data') or '').strip()
        client_context = data.get('client_context') if isinstance(data.get('client_context'), dict) else {}
        hints = _extract_request_auth_hints(data, client_context)
        grant_row = await _resolve_entry_grant(hints['grant']) if hints.get('grant') else None
        auth_result = await _authenticate_tma_init_data(
            init_data,
            grant_row=grant_row,
            tenant_hint=hints.get('tenant_hint') or '',
            merchant_id_hint=hints.get('merchant_id_hint'),
        )
        if not auth_result.get('ok'):
            return web.json_response(
                {"status": "error", "message": auth_result.get('message') or 'init_data 验证失败'},
                status=int(auth_result.get('status_code') or 400),
            )

        merchant = auth_result.get('merchant')
        is_platform_admin = bool(auth_result.get('is_platform_admin'))
        user_info = auth_result.get('user') or {}
        tg_user_id = int(auth_result.get('tg_user_id') or 0)
        scope = _resolve_identity_scope(
            entry_intent='menu',
            is_platform_admin=is_platform_admin,
            merchant=merchant,
            last_scope='platform_admin',
        )
        role = scope.get('role') or 'unregistered'

        if role == 'merchant_owner' and merchant:
            tenant_id = str(merchant.get('tenant_id') or '')
            merchant_id = int(merchant.get('id') or 0)
        elif role == 'platform_admin':
            tenant_id = ''
            merchant_id = None
        else:
            return web.json_response({
                "status": "ok",
                "token": None,
                "session": {
                    "role": "unregistered",
                    "tenant_id": "",
                    "merchant_id": None,
                    "actor_role": "unregistered",
                },
                "user": user_info,
            })

        session_token = _issue_token(
            role=role,
            tenant_id=tenant_id,
            merchant_id=merchant_id,
            issued_for=str(tg_user_id),
        )

        return web.json_response({
            "status": "ok",
            "token": session_token,
            "session": {
                "role": role,
                "tenant_id": tenant_id,
                "merchant_id": merchant_id,
                "actor_role": role,
            },
            "user": user_info,
        })

    except Exception as e:
        logger.error(f'api_login_tma error: {e}', exc_info=True)
        return web.json_response({"status": "error", "message": str(e)}, status=500)


def _set_bootstrap_cookies(
    response: web.StreamResponse,
    *,
    request: web.Request,
    role: str,
    proof_value: Optional[str] = None,
) -> None:
    secure = _cookie_secure(request)
    response.set_cookie(
        LAST_SCOPE_COOKIE,
        value=str(role or 'platform_admin'),
        max_age=ENTRY_GRANT_TTL_SECONDS,
        path='/',
        samesite='Strict',
        secure=secure,
    )
    if proof_value:
        response.set_cookie(
            ENTRY_PROOF_COOKIE,
            value=proof_value,
            max_age=ENTRY_GRANT_TTL_SECONDS,
            path='/',
            httponly=True,
            samesite='Strict',
            secure=secure,
        )


async def api_miniapp_bootstrap(request):
    try:
        data = await request.json()
        init_data = str(data.get('init_data') or '').strip()
        otp = str(data.get('otp') or '').strip()
        grant = str(data.get('grant') or '').strip()
        entry_intent = _normalize_entry_intent(data.get('entry_intent'))
        client_context = data.get('client_context') if isinstance(data.get('client_context'), dict) else {}
        auth_hints = _extract_request_auth_hints(data, client_context)
        user_agent = _extract_user_agent(request, client_context)
        ua_fp = _ua_fingerprint(user_agent)
        last_scope = str(request.cookies.get(LAST_SCOPE_COOKIE) or 'platform_admin').strip()

        resolved_grant = grant or str(auth_hints.get('grant') or '').strip()
        grant_row = await _resolve_entry_grant(resolved_grant) if resolved_grant else None
        business_window = await _resolve_business_window()

        token = None
        session = None
        renewal = {'rotated': False, 'source': 'none', 'grant_expires_at': grant_row.get('expires_at') if grant_row else None}
        proof_value = None

        if init_data:
            auth_result = await _authenticate_tma_init_data(
                init_data,
                grant_row=grant_row,
                tenant_hint=auth_hints.get('tenant_hint') or '',
                merchant_id_hint=auth_hints.get('merchant_id_hint'),
            )
            if auth_result.get('ok'):
                merchant = auth_result.get('merchant')
                scope = _resolve_identity_scope(
                    entry_intent=entry_intent,
                    is_platform_admin=bool(auth_result.get('is_platform_admin')),
                    merchant=merchant,
                    last_scope=last_scope,
                )
                role = scope.get('role') or 'unregistered'
                if role == 'unregistered':
                    landing = _build_landing_payload(role, entry_intent=entry_intent, business_window=business_window)
                    return web.json_response({
                        'status': 'ok',
                        'token': None,
                        'session': {
                            'role': 'unregistered',
                            'tenant_id': '',
                            'merchant_id': None,
                            'actor_role': 'unregistered',
                        },
                        'landing': landing,
                        'renewal': renewal,
                        'ui_context': {
                            'business_window': business_window,
                            'entry_intent': entry_intent,
                            'identity_label': _identity_label('unregistered'),
                        },
                        'user': auth_result.get('user'),
                    })

                merchant_id = None
                tenant_id = ''
                if role == 'merchant_owner' and merchant:
                    merchant_id = int(merchant.get('id') or 0)
                    tenant_id = str(merchant.get('tenant_id') or '')
                token = _issue_token(
                    role=role,
                    tenant_id=tenant_id,
                    merchant_id=merchant_id,
                    issued_for=str(auth_result.get('tg_user_id') or ''),
                )
                session = {
                    'role': role,
                    'tenant_id': tenant_id,
                    'merchant_id': merchant_id,
                    'actor_role': role,
                }
                renewal = {'rotated': bool(grant_row), 'source': 'init_data', 'grant_expires_at': grant_row.get('expires_at') if grant_row else None}
                if grant_row and str(grant_row.get('issued_for') or '') == str(auth_result.get('tg_user_id') or ''):
                    await database.bind_web_entry_grant_proof(grant_row['grant_hash'], ua_fingerprint=ua_fp)
                    grant_row = await _resolve_entry_grant(grant)
                    if grant_row:
                        proof_value = _issue_entry_proof_cookie_value(grant_row, ua_fp)

        if not token and otp:
            otp_result = await _consume_otp_session(otp)
            if otp_result.get('ok'):
                token = otp_result.get('token')
                session = {
                    'role': otp_result['session'].get('role', 'platform_admin'),
                    'tenant_id': otp_result['session'].get('tenant_id', ''),
                    'merchant_id': otp_result['session'].get('merchant_id'),
                    'actor_role': otp_result['session'].get('actor_role', otp_result['session'].get('role', 'platform_admin')),
                }
                renewal = {'rotated': False, 'source': 'otp', 'grant_expires_at': grant_row.get('expires_at') if grant_row else None}
                if grant_row and str(grant_row.get('issued_for') or '') == str(otp_result['session'].get('issued_for') or ''):
                    await database.bind_web_entry_grant_proof(grant_row['grant_hash'], ua_fingerprint=ua_fp)
                    grant_row = await _resolve_entry_grant(grant)
                    if grant_row:
                        proof_value = _issue_entry_proof_cookie_value(grant_row, ua_fp)

        if not token and grant_row:
            proof_cookie = str(request.cookies.get(ENTRY_PROOF_COOKIE) or '').strip()
            if proof_cookie and _verify_entry_proof_cookie_value(proof_cookie, grant_row, ua_fp):
                await database.touch_web_entry_grant_rotation(grant_row['grant_hash'])
                session = _session_from_grant_row(grant_row)
                token = _issue_token(
                    role=session['role'],
                    tenant_id=session['tenant_id'],
                    merchant_id=session['merchant_id'],
                    issued_for=str(grant_row.get('issued_for') or ''),
                )
                renewal = {'rotated': True, 'source': 'grant_proof', 'grant_expires_at': grant_row.get('expires_at')}
                proof_value = _issue_entry_proof_cookie_value(grant_row, ua_fp)
            else:
                return web.json_response({
                    'status': MINIAPP_REOPEN_STATUS,
                    'message': '请返回 Telegram 重新打开 Mini App',
                    'reason': 'grant_requires_proof',
                    'landing': {'route': '/chat', 'mode': 'reopen', 'reason': 'grant_requires_proof'},
                    'renewal': {'rotated': False, 'source': 'grant_rejected', 'grant_expires_at': grant_row.get('expires_at')},
                    'ui_context': {
                        'business_window': business_window,
                        'entry_intent': entry_intent,
                        'identity_label': '需要返回 Telegram',
                    },
                }, status=401)

        if not token or not session:
            return web.json_response({
                'status': MINIAPP_REOPEN_STATUS,
                'message': '登录状态已失效，请返回 Telegram 重新打开',
                'reason': 'bootstrap_auth_failed',
                'landing': {'route': '/chat', 'mode': 'reopen', 'reason': 'bootstrap_auth_failed'},
                'renewal': renewal,
                'ui_context': {
                    'business_window': business_window,
                    'entry_intent': entry_intent,
                    'identity_label': '需要返回 Telegram',
                },
            }, status=401)

        landing = _build_landing_payload(session['role'], entry_intent=entry_intent, business_window=business_window)
        response = web.json_response({
            'status': 'ok',
            'token': token,
            'session': session,
            'landing': landing,
            'renewal': renewal,
            'ui_context': {
                'business_window': business_window,
                'entry_intent': entry_intent,
                'identity_label': _identity_label(session['role']),
            },
        })
        _set_bootstrap_cookies(response, request=request, role=session['role'], proof_value=proof_value)
        return response
    except Exception as e:
        logger.error('api_miniapp_bootstrap error: %s', e, exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_miniapp_context(request):
    try:
        session = _extract_session_context_from_request(request)
        business_window = await _resolve_business_window()
        return web.json_response({
            'status': 'ok',
            'data': {
                'session': session,
                'business_window': business_window,
                'identity_label': _identity_label(str(session.get('role') or 'platform_admin')),
            },
        })
    except Exception as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_miniapp_logout_revoke(request):
    try:
        data = await request.json()
        grant = str(data.get('grant') or '').strip()
        if grant:
            await database.revoke_web_entry_grant(_hash_grant(grant))
        response = web.json_response({'status': 'ok'})
        response.del_cookie(ENTRY_PROOF_COOKIE, path='/')
        response.del_cookie(LAST_SCOPE_COOKIE, path='/')
        return response
    except Exception as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_get_session_context(request):
    try:
        session = _extract_session_context_from_request(request)
        return web.json_response({"status": "ok", "data": session})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_health(request):
    """Small production readiness endpoint for tunnel and Mini App smoke checks."""
    dist_dir = _frontend_dist_dir()
    index_path = os.path.join(dist_dir, 'index.html')
    assets_dir = os.path.join(dist_dir, 'assets')
    index_exists = os.path.exists(index_path)
    assets_exists = os.path.isdir(assets_dir)
    status = 'ok' if index_exists and assets_exists else 'degraded'
    payload = {
        'status': status,
        'service': 'tgbot',
        'time': datetime.now(ZoneInfo('Asia/Shanghai')).isoformat(),
        'frontend': {
            'dist': os.path.relpath(dist_dir, os.path.dirname(os.path.abspath(__file__))),
            'index': index_exists,
            'assets': assets_exists,
        },
        'miniapp': {
            'path': '/miniapp',
            'requires_telegram_init_data': True,
        },
    }
    return web.json_response(payload, status=200 if status == 'ok' else 503)


async def api_update_session_context(request):
    try:
        current = _extract_session_context_from_request(request)

        # 安全校验: 未认证用户不能切换角色
        if not current.get('authenticated', True):
            return web.json_response({
                "status": "error",
                "message": "未认证用户禁止切换角色",
            }, status=401)

        # 会话上下文只允许读取，不允许在生产入口动态切换角色/租户。
        # 目标架构要求：
        # 1. 平台管理员固定平台域
        # 2. 商户通过自己的 OTP/session 进入自己的 tenant 域
        # 3. 禁止管理员代入商户视角，禁止商户切回平台视角
        return web.json_response({
            "status": "error",
            "message": "session_context_switch_disabled",
        }, status=403)
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def index_handler(request):
    """返回前端 SPA 页面，OTP 登录由前端自行调用 /api/login/otp。"""
    dist_index = os.path.join(_frontend_dist_dir(), 'index.html')

    try:
        with open(dist_index, 'r', encoding='utf-8') as f:
            return web.Response(
                text=f.read(),
                content_type='text/html',
                headers={'Cache-Control': 'no-store, no-cache, must-revalidate', 'Pragma': 'no-cache'}
            )
    except Exception as e:
        logger.error(f"加载前端 index.html 失败: {e}")
        return web.Response(text=f"Error loading frontend UI: {e}", status=500)


async def favicon_handler(request):
    return web.Response(status=204)


async def saas_proxy_handler(request):
    """Forward SaaS API traffic to the Rust/Supabase backend when SAAS_BACKEND_URL is configured."""
    target_base = _saas_backend_url()
    if not target_base:
        return web.json_response({"status": "error", "message": "saas_backend_not_configured"}, status=503)

    target_url = f"{target_base}{request.path}"
    if request.rel_url.query_string:
        target_url = f"{target_url}?{request.rel_url.query_string}"

    hop_by_hop = {
        'connection',
        'content-length',
        'host',
        'keep-alive',
        'proxy-authenticate',
        'proxy-authorization',
        'te',
        'trailer',
        'transfer-encoding',
        'upgrade',
    }
    trusted_header_names = {
        'x-saas-internal-secret',
        'x-user-id',
        'x-user-role',
        'x-tenant-id',
        'x-merchant-id',
    }
    proxy_headers = {}
    for name, value in request.headers.items():
        lower = name.lower()
        if lower in hop_by_hop or lower in trusted_header_names or lower == 'cookie':
            continue
        proxy_headers[name] = value

    internal_secret = str(os.getenv('INTERNAL_SAAS_SECRET') or '').strip()
    session = _extract_session_context_from_request(request)
    if internal_secret and session.get('authenticated', False):
        proxy_headers.update({
            'x-saas-internal-secret': internal_secret,
            'x-user-role': str(session.get('role') or ''),
            'x-user-id': str(session.get('issued_for') or ''),
            'x-tenant-id': str(session.get('tenant_id') or ''),
            'x-merchant-id': str(session.get('merchant_id') or ''),
        })

    body = await request.read()
    timeout = aiohttp.ClientTimeout(total=float(os.getenv('SAAS_PROXY_TIMEOUT', '20')))
    try:
        async with aiohttp.ClientSession(timeout=timeout) as client:
            async with client.request(
                request.method,
                target_url,
                headers=proxy_headers,
                data=body if body else None,
                allow_redirects=False,
            ) as upstream:
                response_body = await upstream.read()
                response_headers = {}
                for name, value in upstream.headers.items():
                    lower = name.lower()
                    if lower in hop_by_hop or lower in {'content-encoding', 'content-length'}:
                        continue
                    response_headers[name] = value
                return web.Response(body=response_body, status=upstream.status, headers=response_headers)
    except Exception as e:
        logger.warning(f"SaaS Rust backend proxy failed: {e}")
        return web.json_response(
            {"status": "error", "message": "saas_backend_unavailable", "detail": str(e)},
            status=502,
        )


async def api_get_users(request):
    """获取来访用户列表"""
    try:
        scope = _request_scope_context(request)
        users = await database.get_inbox_list(
            tenant_id=scope['tenant_id'] if scope['is_merchant'] else None,
            merchant_id=scope['merchant_id'] if scope['is_merchant'] else None,
        )
        return web.json_response({"status": "ok", "data": users})
    except Exception as e:
        logger.error(f"api_get_users 错误: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def api_get_messages(request):
    """获取与某用户的聊天历史"""
    try:
        scope = _request_scope_context(request)
        user_id = int(request.match_info['user_id'])
        page = int(request.query.get('page', 1))
        limit = 50
        offset = (page - 1) * limit

        if user_id < 0 and user_id != AI_ASSISTANT_USER_ID:
            return web.json_response({"status": "ok", "data": []})
        
        messages = await database.get_chat_history(
            user_id,
            limit,
            offset,
            tenant_id=scope['tenant_id'] if scope['is_merchant'] else None,
            merchant_id=scope['merchant_id'] if scope['is_merchant'] else None,
        )
        return web.json_response({"status": "ok", "data": messages})
    except Exception as e:
        logger.error(f"api_get_messages 错误: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def _execute_web_send_task(
    user_id: int,
    msg_db_id: Optional[int],
    raw_input_text: str,
    final_send_content: str,
    image_file: Optional[bytes],
    is_actually_translated: bool,
    quoted_text: Optional[str],
    quoted_type: str,
    quoted_file_id: Optional[str],
    reply_to_msg_id: Optional[int],
    ocr_tag: str,
    security_check_duration: float,
    reply_markup=None,
) -> None:
    """Web 端单条消息异步投递（单发与群发共用）。"""
    import io
    global _bot_instance, active_send_tasks, last_performance_metrics
    if not _bot_instance or msg_db_id is None:
        return
    active_send_tasks += 1
    await notify_web_clients("queue_update", {"active_tasks": active_send_tasks})
    task_start = time.time()
    try:
        from handlers.customer_service import safe_send_message, safe_send_photo, format_admin_tag
        sent_msg = None
        admin_info = await database.get_admin_info(config.ADMIN_ID)
        admin_display_name = admin_info["username"] if admin_info else "Administrator"
        admin_worker_no = admin_info["worker_no"] if admin_info else "CS001"
        admin_tag = format_admin_tag(config.ADMIN_ID, admin_display_name, admin_worker_no)
        tg_send_start = time.time()
        if image_file:
            photo_stream = io.BytesIO(image_file)
            photo_stream.name = "upload.jpg"
            caption_text = admin_tag
            if final_send_content:
                caption_text += f"\n{final_send_content}"
            sent_msg = await safe_send_photo(
                context=None,
                chat_id=user_id,
                photo=photo_stream,
                caption=caption_text,
                parse_mode="HTML",
                reply_to_message_id=reply_to_msg_id,
                reply_markup=reply_markup,
            )
        else:
            sent_msg = await safe_send_message(
                context=None,
                chat_id=user_id,
                text=f"{admin_tag}\n{final_send_content}",
                parse_mode="HTML",
                reply_to_message_id=reply_to_msg_id,
                reply_markup=reply_markup,
            )
        tg_send_duration = (time.time() - tg_send_start) * 1000
        if sent_msg:
            await database.update_message_status(msg_db_id, "sent", sent_msg.message_id)
            sync_start = time.time()
            try:
                from handlers.customer_service import format_user_identity
                target_identity = format_user_identity(user_id)
                sync_text = f"📤 <b>已同步至用户:</b>\n{target_identity}\n"
                if is_actually_translated:
                    content_body = f"\n(译文): {escape(final_send_content)}\n(原文): {escape(raw_input_text)}"
                else:
                    content_body = f"\n{escape(final_send_content)}"
                if ocr_tag:
                    content_body += f"\n{ocr_tag}"
                sync_text += content_body
                admin_reply_to_id = await database.get_admin_msg_id_by_user_msg(reply_to_msg_id) if reply_to_msg_id else None
                if image_file:
                    photo_stream = io.BytesIO(image_file)
                    photo_stream.name = "upload.jpg"
                    await _bot_instance.send_photo(
                        chat_id=config.ADMIN_ID,
                        photo=photo_stream,
                        caption=sync_text,
                        parse_mode="HTML",
                        reply_to_message_id=admin_reply_to_id,
                    )
                else:
                    await safe_send_message(
                        context=None,
                        chat_id=config.ADMIN_ID,
                        text=sync_text,
                        parse_mode="HTML",
                        reply_to_message_id=admin_reply_to_id,
                    )
            except Exception as sync_err:
                logger.warning(f"同步消息失败: {sync_err}")
            sync_duration = (time.time() - sync_start) * 1000
            last_performance_metrics = {
                "user_id": user_id,
                "tg_send_ms": round(tg_send_duration, 2),
                "sync_ms": round(sync_duration, 2),
                "security_check_ms": round(security_check_duration, 2),
                "timestamp": time.time(),
            }
    except Exception as e:
        logger.error(f"发送失败: {e}")
        await database.update_message_status(msg_db_id, "failed")
    finally:
        active_send_tasks -= 1
        total_duration = (time.time() - task_start) * 1000
        await notify_web_clients(
            "status_update",
            {
                "user_id": user_id,
                "duration": round(total_duration, 2),
                "active_tasks": active_send_tasks,
                "metrics": last_performance_metrics,
            },
        )


async def api_send_message(request):
    """通过管理员面板向用户发送消息 (支持图文合并)"""
    try:
        scope = _request_scope_context(request)
        user_id = int(request.match_info['user_id'])
        if user_id < 0:
            return web.json_response(
                {"status": "error", "message": "虚拟助手会话请使用 /api/ai/chat"},
                status=400,
            )
        
        raw_input_text = ""
        reply_to_msg_id = None
        image_file = None
        do_translate = False
        
        if request.content_type == 'application/json':
            data = await request.json()
            raw_input_text = data.get('content', '')
            reply_to_msg_id = data.get('reply_to_msg_id')
            do_translate = data.get('do_translate', False)
        else:
            reader = await request.multipart()
            while True:
                part = await reader.next()
                if part is None: break
                if part.name == 'content': raw_input_text = await part.text()
                elif part.name == 'reply_to_msg_id':
                    val = await part.text()
                    reply_to_msg_id = int(val) if val else None
                elif part.name == 'do_translate':
                    val = await part.text()
                    do_translate = (val.lower() == 'true')
                elif part.name == 'image': image_file = await part.read()
                if image_file and len(image_file) > _MAX_UPLOAD_SIZE:
                    return web.json_response({"status": "error", "message": f"图片大小超过限制 ({_MAX_UPLOAD_SIZE // 1024 // 1024}MB)"}, status=400)
                    
        if not raw_input_text and not image_file:
            return web.json_response({"status": "error", "message": "内容或图片不能为空"}, status=400)

        actual_do_translate = do_translate
        if actual_do_translate and raw_input_text:
            from services.translator import is_chinese
            admin_is_cn = is_chinese(raw_input_text)
            customer_is_cn = False
            if reply_to_msg_id:
                orig_msg = await database.get_message_by_tg_id(reply_to_msg_id)
                if orig_msg:
                    customer_is_cn = is_chinese(orig_msg.get('content', ''))
            if admin_is_cn and customer_is_cn:
                actual_do_translate = False

        target_lang = 'en'
        if actual_do_translate and raw_input_text:
            try:
                from langdetect import detect
                source_text = ''
                if reply_to_msg_id:
                    orig_msg_for_lang = await database.get_message_by_tg_id(reply_to_msg_id)
                    if orig_msg_for_lang:
                        source_text = (orig_msg_for_lang.get('original_text') or orig_msg_for_lang.get('content') or '').strip()
                if not source_text:
                    history = await database.get_chat_history(user_id, limit=20, offset=0)
                    for m in reversed(history):
                        if int(m.get('from_userid', 0)) == int(user_id):
                            source_text = (m.get('original_text') or m.get('content') or '').strip()
                            if source_text:
                                break
                if source_text:
                    target_lang = detect(source_text)
            except Exception:
                target_lang = 'en'

        from services.security_center import security_check_flow
        start_time = time.time()
        is_passed, verified_text, error_msg, _, ocr_tag = await security_check_flow(
            raw_input_text, image_bytes=image_file, do_translate=actual_do_translate, target_lang=target_lang
        )
        security_check_duration = (time.time() - start_time) * 1000
        
        if not is_passed:
            return web.json_response({"status": "error", "message": error_msg}, status=403)
            
        final_send_content = verified_text
        is_actually_translated = (verified_text.strip() != raw_input_text.strip()) and actual_do_translate
            
        quoted_text = None
        quoted_type = 'text'
        quoted_file_id = None
        if reply_to_msg_id:
            orig_msg = await database.get_message_by_tg_id(reply_to_msg_id)
            if orig_msg:
                quoted_text = orig_msg.get('content')
                quoted_type = orig_msg.get('type', 'text')
                quoted_file_id = orig_msg.get('file_id')
                if quoted_type != 'text':
                    quoted_text = f"[{'图片' if quoted_type == 'photo' else '文件'}]"
                if quoted_text and len(quoted_text) > 50:
                    quoted_text = quoted_text[:47] + "..."
            
        msg_db_id = await database.save_message(
            from_userid=config.ADMIN_ID,
            to_userid=user_id,
            content=final_send_content,
            msg_type='photo' if image_file else 'text',
            is_read=True,
            username="Admin",
            quoted_text=quoted_text,
            quoted_msg_id=reply_to_msg_id,
            quoted_file_id=quoted_file_id,
            quoted_type=quoted_type,
            status='sending',
            translated_text=final_send_content if is_actually_translated else None,
            original_text=raw_input_text if is_actually_translated else None,
            admin_userid=str(config.ADMIN_ID),
            tenant_id=scope['tenant_id'] if scope['is_merchant'] else None,
        )
        
        t = asyncio.create_task(
            _execute_web_send_task(
                user_id, msg_db_id, raw_input_text, final_send_content, image_file,
                is_actually_translated, quoted_text, quoted_type, quoted_file_id,
                reply_to_msg_id, ocr_tag, security_check_duration,
            )
        )
        _web_bg_tasks.add(t); t.add_done_callback(_web_bg_tasks.discard)
        return web.json_response({"status": "ok"})
    except Exception as e:
        logger.error(f"api_send_message 错误: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_broadcast_message(request):
    """向多名用户一键发送相同内容"""
    try:
        scope = _request_scope_context(request)
        raw_input_text = ""; image_file = None; do_translate = False; user_ids = []
        if request.content_type == "application/json":
            data = await request.json()
            raw_input_text = data.get("content", ""); do_translate = data.get("do_translate", False); user_ids = data.get("user_ids", [])
        else:
            reader = await request.multipart()
            while True:
                part = await reader.next()
                if part is None: break
                if part.name == "content": raw_input_text = await part.text()
                elif part.name == "do_translate": do_translate = (await part.text()).lower() == "true"
                elif part.name == "image": image_file = await part.read()
                if image_file and len(image_file) > _MAX_UPLOAD_SIZE:
                    return web.json_response({"status": "error", "message": f"图片大小超过限制 ({_MAX_UPLOAD_SIZE // 1024 // 1024}MB)"}, status=400)
                elif part.name == "user_ids": user_ids = json.loads(await part.text())

        if not user_ids or (not raw_input_text and not image_file):
            return web.json_response({"status": "error", "message": "参数不完整"}, status=400)

        from services.security_center import security_check_flow
        is_passed, verified_text, error_msg, _, ocr_tag = await security_check_flow(
            raw_input_text, image_bytes=image_file, do_translate=do_translate
        )
        if not is_passed: return web.json_response({"status": "error", "message": error_msg}, status=403)

        count = 0
        for uid in set(user_ids):
            msg_db_id = await database.save_message(
                from_userid=config.ADMIN_ID, to_userid=int(uid), content=verified_text,
                msg_type="photo" if image_file else "text", is_read=True, username="Admin",
                status="sending", admin_userid=str(config.ADMIN_ID),
                tenant_id=scope['tenant_id'] if scope['is_merchant'] else None,
            )
            count += 1
            t = asyncio.create_task(_execute_web_send_task(int(uid), msg_db_id, raw_input_text, verified_text, image_file, False, None, "text", None, None, ocr_tag, 0))
            _web_bg_tasks.add(t); t.add_done_callback(_web_bg_tasks.discard)
        return web.json_response({"status": "ok", "count": count})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_broadcast_unified(request):
    """统一群发入口：同时支持用户、群组、频道、图片和内联按钮。"""
    try:
        scope = _request_scope_context(request)
        content = ''
        do_translate = False
        user_ids = []
        chat_ids = []
        buttons = []
        retry_count = 1
        image_file = None

        if request.content_type == 'application/json':
            data = await request.json()
            content = str(data.get('content') or '').strip()
            do_translate = bool(data.get('do_translate', False))
            user_ids = data.get('user_ids') or []
            chat_ids = data.get('chat_ids') or []
            buttons = data.get('buttons') or []
            retry_count = int(data.get('retry_count') or 1)
        else:
            reader = await request.multipart()
            while True:
                part = await reader.next()
                if part is None:
                    break
                if part.name == 'content':
                    content = (await part.text()).strip()
                elif part.name == 'do_translate':
                    do_translate = (await part.text()).lower() == 'true'
                elif part.name == 'user_ids':
                    user_ids = json.loads(await part.text() or '[]')
                elif part.name == 'chat_ids':
                    chat_ids = json.loads(await part.text() or '[]')
                elif part.name == 'buttons':
                    buttons = json.loads(await part.text() or '[]')
                elif part.name == 'retry_count':
                    retry_count = int((await part.text()) or 1)
                elif part.name == 'image':
                    image_file = await part.read()
                    if image_file and len(image_file) > _MAX_UPLOAD_SIZE:
                        return web.json_response(
                            {"status": "error", "message": f"图片大小超过限制 ({_MAX_UPLOAD_SIZE // 1024 // 1024}MB)"},
                            status=400,
                        )

        user_ids = _dedupe_int_values(user_ids)
        chat_ids = _dedupe_int_values(chat_ids)
        if not user_ids and not chat_ids:
            return web.json_response({"status": "error", "message": "请选择发送目标"}, status=400)
        if not content and not image_file:
            return web.json_response({"status": "error", "message": "请输入群发内容或选择图片"}, status=400)

        reply_markup = _build_broadcast_reply_markup(buttons)

        from services.security_center import security_check_flow
        is_passed, verified_text, error_msg, _, ocr_tag = await security_check_flow(
            content,
            image_bytes=image_file,
            do_translate=do_translate,
        )
        if not is_passed:
            return web.json_response({"status": "error", "message": error_msg or "安全校验失败"}, status=403)

        user_success_ids = []
        failed_items = []
        translated = bool(do_translate and verified_text != content)

        for uid in user_ids:
            try:
                msg_db_id = await database.save_message(
                    from_userid=config.ADMIN_ID,
                    to_userid=uid,
                    content=verified_text or '[图片]',
                    msg_type="photo" if image_file else "text",
                    is_read=True,
                    username="Admin",
                    status="sending",
                    admin_userid=str(config.ADMIN_ID),
                    tenant_id=scope['tenant_id'] if scope['is_merchant'] else None,
                )
                task = asyncio.create_task(
                    _execute_web_send_task(
                        uid,
                        msg_db_id,
                        content,
                        verified_text,
                        image_file,
                        translated,
                        None,
                        "text",
                        None,
                        None,
                        ocr_tag,
                        0,
                        reply_markup=reply_markup,
                    )
                )
                _web_bg_tasks.add(task)
                task.add_done_callback(_web_bg_tasks.discard)
                user_success_ids.append(uid)
            except Exception as ex:
                failed_items.append({"user_id": uid, "reason": str(ex)})

        chat_summary = {
            "total": 0,
            "success": 0,
            "failed": 0,
            "success_ids": [],
            "failed_items": [],
        }
        if chat_ids:
            chat_summary = await _send_broadcast_to_chats(
                chat_ids=chat_ids,
                content=content,
                image_file=image_file,
                do_translate=do_translate,
                retries=max(retry_count, 0),
                reply_markup=reply_markup,
                prechecked_text=verified_text,
            )
            failed_items.extend(chat_summary.get('failed_items', []))

        summary = {
            "total": len(user_ids) + int(chat_summary.get('total') or 0),
            "success": len(user_success_ids) + int(chat_summary.get('success') or 0),
            "failed": len(failed_items),
            "success_user_ids": user_success_ids,
            "success_chat_ids": chat_summary.get('success_ids', []),
            "failed_items": failed_items,
        }
        await notify_web_clients('broadcast_chat_result', summary)
        return web.json_response({"status": "ok", "summary": summary, "failed_items": failed_items})
    except ValueError as e:
        return web.json_response({"status": "error", "message": str(e)}, status=400)
    except Exception as e:
        logger.error(f"api_broadcast_unified 错误: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def _resolve_ai_pipeline_mode(tenant_id: str) -> str:
    tenant_key = f"ai_pipeline_mode:{tenant_id}" if tenant_id else ''
    mode = None
    if tenant_key:
        mode = await database.get_admin_setting(tenant_key, None)
    if not mode:
        mode = await database.get_admin_setting('ai_pipeline_mode', 'agent')
    mode = str(mode or 'agent').strip().lower()
    return mode if mode in {'agent', 'legacy'} else 'agent'


async def api_get_ai_pipeline_mode(request):
    try:
        scope = _request_scope_context(request)
        tenant_id = str(request.rel_url.query.get('tenant_id') or scope['tenant_id'] or '').strip()
        if not tenant_id and scope['role'] != 'platform_admin':
            return web.json_response({'status': 'error', 'message': 'tenant_id required'}, status=400)
        mode = await _resolve_ai_pipeline_mode(tenant_id or 'platform')
        return web.json_response({
            'status': 'ok',
            'data': {
                'tenantId': tenant_id or 'platform',
                'mode': mode,
                'modes': ['agent', 'legacy'],
            },
        })
    except Exception as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_set_ai_pipeline_mode(request):
    try:
        scope = _request_scope_context(request)
        if scope['role'] not in {'platform_admin'}:
            return web.json_response({'status': 'error', 'message': 'platform_admin_required'}, status=403)

        data = await request.json()
        tenant_id = str(data.get('tenantId') or data.get('tenant_id') or 'platform').strip() or 'platform'
        mode = str(data.get('mode') or '').strip()
        if mode not in {'agent', 'legacy'}:
            return web.json_response({'status': 'error', 'message': 'invalid pipeline mode'}, status=400)

        key = 'ai_pipeline_mode' if tenant_id == 'platform' else f'ai_pipeline_mode:{tenant_id}'
        await database.save_admin_setting(key, mode)
        return web.json_response({'status': 'ok', 'data': {'tenantId': tenant_id, 'mode': mode}})
    except Exception as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def _load_enabled_ai_custom_skills() -> list:
    try:
        rows = await database.list_ai_custom_skills(enabled_only=True)
        if isinstance(rows, list):
            return rows[:20]
    except Exception as exc:
        logger.debug("加载 AI 自定义技能失败: %s", exc)
    return []


def _agent_session_key(*, tenant_id: str, user_id: int, scope: Dict[str, Any]) -> str:
    merchant_id = scope.get('merchant_id')
    role = scope.get('role') or 'platform_admin'
    return f"{tenant_id or 'platform'}:{role}:{merchant_id or ''}:{user_id}"


def _is_agent_confirmation_text(text: str) -> bool:
    normalized = re.sub(r"\s+", "", str(text or "").strip().lower())
    return normalized in {
        "确认",
        "确认执行",
        "确认删除",
        "继续",
        "执行",
        "yes",
        "ok",
        "confirm",
    }


async def _run_ai_agent_chat(
    *,
    user_id: int,
    input_text: str,
    model: str,
    tenant_id: str,
    scope: Dict[str, Any],
    chat_id: Optional[int] = None,
):
    from backend.app.services.ai_service import get_ai_service
    from services.ai_agent.core import get_agent
    from services.ai_agent.schema import AgentContext

    async def llm_call(messages: list, model: str = ""):
        return await get_ai_service().chat_messages(
            messages=messages,
            model=model or "gemini-2.5-flash-lite",
        )

    session_key = _agent_session_key(tenant_id=tenant_id, user_id=user_id, scope=scope)
    session = _ai_agent_sessions.get(session_key)
    custom_skills = await _load_enabled_ai_custom_skills()

    if not session:
        restored_history = []
        try:
            history_tenant_id = tenant_id if tenant_id != 'platform' else None
            db_history = await database.get_chat_history(
                user_id,
                limit=12,
                tenant_id=history_tenant_id,
                merchant_id=scope.get('merchant_id'),
            )
            for row in db_history:
                content = str(row.get('content') or '').strip()
                if not content:
                    continue
                from_uid = int(row.get('from_userid') or 0)
                role = 'assistant' if from_uid == int(user_id) else 'user'
                restored_history.append({"role": role, "content": content[:3000]})
        except Exception as hist_err:
            logger.debug("恢复 AI Agent 历史失败: %s", hist_err)

        ctx = AgentContext(
            user_id=user_id,
            channel='web_ai_chat',
            sandbox_id=f"sbx_{tenant_id or 'platform'}_{user_id}",
            metadata={
                'tenant_id': tenant_id,
                'merchant_id': scope.get('merchant_id'),
                'role': scope.get('role'),
                'custom_skills': custom_skills,
                'chat_id': chat_id,
                'group_id': chat_id,
            },
            bot_instance=_bot_instance,
            conversation_history=restored_history,
        )
        session = {'ctx': ctx, 'pending': None, 'updated_at': time.time()}
        _ai_agent_sessions[session_key] = session
    else:
        ctx = session['ctx']
        ctx.metadata['custom_skills'] = custom_skills
        ctx.metadata['tenant_id'] = tenant_id
        ctx.metadata['merchant_id'] = scope.get('merchant_id')
        ctx.metadata['role'] = scope.get('role')
        ctx.metadata['chat_id'] = chat_id
        ctx.metadata['group_id'] = chat_id
        ctx.bot_instance = _bot_instance
        session['updated_at'] = time.time()

    # Soft cleanup to avoid unbounded growth.
    if len(_ai_agent_sessions) > 300:
        cutoff = time.time() - 6 * 3600
        for key in list(_ai_agent_sessions.keys()):
            if float(_ai_agent_sessions[key].get('updated_at') or 0) < cutoff:
                _ai_agent_sessions.pop(key, None)

    agent = get_agent(llm_call_fn=llm_call)
    pending = session.get('pending')
    if pending is not None and _is_agent_confirmation_text(input_text):
        result = await agent.confirm_and_execute(pending, ctx)
        session['pending'] = None
        return result

    result = await agent.process_message(input_text, ctx=ctx, model=model)
    session['pending'] = result if result.requires_confirmation else None
    return result


async def api_ai_chat_bridge(request):
    """AI chat endpoint used by the virtual Agent assistant."""
    try:
        data = await request.json()
        raw_user_id = request.match_info.get('user_id') or data.get('userId') or data.get('user_id') or AI_ASSISTANT_USER_ID
        user_id = int(raw_user_id)
        input_text = str(data.get('content') or data.get('input') or data.get('message') or '').strip()
        model = str(data.get('model') or 'gemini-2.5-flash-lite')
        feature_key = str(data.get('feature_key') or 'general')
        idempotency_key = str(data.get('idempotencyKey') or data.get('idempotency_key') or '').strip()
        scope = _request_scope_context(request)
        body_tenant_id = str(data.get('tenant_id') or (data.get('context') or {}).get('tenant_id') or '').strip()
        body_context = data.get('context') if isinstance(data.get('context'), dict) else {}
        raw_chat_id = (
            data.get('chatId') or data.get('chat_id') or data.get('group_id')
            or body_context.get('chat_id') or body_context.get('group_id')
        )
        try:
            chat_id = int(raw_chat_id) if str(raw_chat_id or '').strip() else None
        except Exception:
            chat_id = None
        if body_tenant_id and scope.get('role') == 'platform_admin':
            scope['tenant_id'] = body_tenant_id
            scope['is_merchant'] = body_tenant_id not in {'', 'platform'}
        tenant_id = scope['tenant_id'] or 'platform'

        if not input_text:
            return web.json_response({'status': 'error', 'message': '输入不能为空'}, status=400)

        mode = await _resolve_ai_pipeline_mode(tenant_id)

        try:
            from backend.app.services.quota_service import get_quota_service
            quota_check = await get_quota_service().check_available(tenant_id, amount=1)
            if not quota_check.get('allowed'):
                return web.json_response(
                    {'status': 'error', 'message': 'AI 配额不足', 'reason': quota_check.get('reason')},
                    status=403,
                )
        except Exception:
            quota_check = {'allowed': True}

        agent_result = None
        if mode == 'agent':
            try:
                agent_result = await _run_ai_agent_chat(
                    user_id=user_id,
                    input_text=input_text,
                    model=model,
                    tenant_id=tenant_id,
                    scope=scope,
                    chat_id=chat_id,
                )
                reply = agent_result.text
            except Exception as agent_err:
                logger.error(f"AI Agent 调用失败: {agent_err}", exc_info=True)
                return web.json_response(
                    {'status': 'error', 'message': f'AI Agent 调用失败: {str(agent_err)[:200]}'},
                    status=502,
                )
        else:
            try:
                from backend.app.services.ai_service import get_ai_service
                reply = await get_ai_service().chat(user_id=user_id, message=input_text, model=model)
            except Exception as ai_err:
                logger.error(f"AI 服务调用失败: {ai_err}")
                return web.json_response(
                    {'status': 'error', 'message': f'LLM 服务调用失败: {str(ai_err)[:200]}'},
                    status=502,
                )

        try:
            counterpart_id = await _resolve_ai_chat_counterpart_id(scope)
            effective_tenant_id = tenant_id if tenant_id != 'platform' else None
            await database.save_message(
                from_userid=counterpart_id,
                to_userid=user_id,
                content=input_text,
                msg_type='text',
                is_read=True,
                username='Admin',
                status='sent',
                tenant_id=effective_tenant_id,
            )
            await database.save_message(
                from_userid=user_id,
                to_userid=counterpart_id,
                content=reply,
                msg_type='text',
                is_read=True,
                username='Agent助手',
                status='received',
                tenant_id=effective_tenant_id,
            )
        except Exception as persist_err:
            logger.warning("AI Agent 会话落库失败: %s", persist_err)

        try:
            from backend.repositories.ai_usage_repo import save_ai_usage_log
            from backend.app.services.quota_service import get_quota_service
            total_tokens = (len(input_text) + len(reply)) // 4
            await save_ai_usage_log(
                channel='web_chat',
                user_id=user_id,
                sandbox_id=f'sbx_{user_id}',
                tenant_id=tenant_id,
                merchant_id=scope['merchant_id'],
                feature_key=feature_key,
                result_status='success',
                endpoint='chat',
                model=model,
                prompt_tokens=len(input_text) // 4,
                completion_tokens=len(reply) // 4,
                tokens_used=total_tokens,
                cost=0.0001,
            )
            await get_quota_service().record_usage(tenant_id, amount=1)
        except Exception:
            pass

        payload = {
            'status': 'ok',
            'reply': reply,
            'data': {'reply': reply},
            'pipelineMode': mode,
            'reply_text': reply,
            'reply_msg_type': 'text',
        }
        if agent_result is not None:
            evidence = {}
            data_complete_values = []
            analysis_allowed_values = []
            for tool_name, tool_result in (agent_result.tool_results or {}).items():
                if isinstance(tool_result, dict):
                    evidence[tool_name] = {
                        'source': tool_result.get('source'),
                        'data_complete': tool_result.get('data_complete'),
                        'analysis_allowed': tool_result.get('analysis_allowed'),
                        'evidence_count': tool_result.get('evidence_count'),
                        'limitation': tool_result.get('limitation'),
                    }
                    if tool_result.get('data_complete') is not None:
                        data_complete_values.append(bool(tool_result.get('data_complete')))
                    if tool_result.get('analysis_allowed') is not None:
                        analysis_allowed_values.append(bool(tool_result.get('analysis_allowed')))
            payload.update({
                'tool_calls_made': agent_result.tool_calls_made,
                'tool_results': agent_result.tool_results,
                'evidence': evidence,
                'data_complete': all(data_complete_values) if data_complete_values else None,
                'analysis_allowed': all(analysis_allowed_values) if analysis_allowed_values else None,
                'is_action_taken': agent_result.is_action_taken,
                'requires_confirmation': agent_result.requires_confirmation,
                'action_summary': agent_result.action_summary,
            })
        return web.json_response(payload)
    except ValueError:
        return web.json_response({'status': 'error', 'message': '用户 ID 无效'}, status=400)
    except Exception as e:
        logger.error(f"AI 聊天失败：{e}")
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_agent_telegram_sdk_query(request):
    """n8n/Agent 只读工具端口：查询 Telegram Bot API 官方可见信息。"""
    try:
        data = await request.json()
        scope = _request_scope_context(request)
        body_tenant_id = str(data.get('tenant_id') or (data.get('context') or {}).get('tenant_id') or '').strip()
        if body_tenant_id and scope.get('role') == 'platform_admin':
            scope['tenant_id'] = body_tenant_id
            scope['is_merchant'] = body_tenant_id not in {'', 'platform'}

        from services.ai_agent.schema import AgentContext
        from services.ai_agent.tools import TelegramSdkQueryTool

        tenant_id = scope.get('tenant_id') or 'platform'
        ctx = AgentContext(
            user_id=int(data.get('userId') or data.get('user_id') or AI_ASSISTANT_USER_ID),
            channel='n8n_tool',
            sandbox_id=f"n8n_{tenant_id}",
            metadata={
                'tenant_id': tenant_id,
                'merchant_id': scope.get('merchant_id'),
                'role': scope.get('role'),
            },
            bot_instance=_bot_instance,
        )
        result = await TelegramSdkQueryTool().execute(data, context=ctx)
        return web.json_response({'status': 'ok' if result.get('success', True) else 'error', **result})
    except Exception as e:
        logger.error("Telegram SDK query API failed: %s", e, exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_agent_tool_manifest(request):
    """Single source of truth for MiniApp and n8n tool discovery."""
    from services.ai_agent.safety import DANGEROUS_ACTIONS
    from services.ai_agent.tools import get_tools_for_llm

    tools = get_tools_for_llm()
    for schema in tools:
        name = str((schema.get('function') or {}).get('name') or '')
        schema['dangerous'] = name in DANGEROUS_ACTIONS
    return web.json_response({'status': 'ok', 'count': len(tools), 'tools': tools})


async def api_agent_ad_fingerprints(request):
    """n8n/Agent shared advertisement text knowledge-base API."""
    try:
        scope = _request_scope_context(request)
        data = await request.json() if request.method != 'GET' else {}
        requested_tenant = str(
            data.get('tenant_id') or request.rel_url.query.get('tenant_id') or ''
        ).strip()
        tenant_id = (
            scope.get('tenant_id') if scope.get('is_merchant')
            else requested_tenant or 'default'
        )

        if request.method == 'GET':
            rows = await database.list_ad_fingerprints(
                tenant_id=tenant_id,
                limit=int(request.rel_url.query.get('limit') or 50),
            )
            return web.json_response({
                'status': 'ok',
                'data': rows,
                'count': len(rows),
                'stats': await database.get_ad_fingerprint_stats(tenant_id),
            })

        content = str(data.get('content') or data.get('text') or '').strip()
        if not content:
            return web.json_response({'status': 'error', 'message': 'content required'}, status=400)
        text_hash = await database.confirm_ad_fingerprint(
            tenant_id=tenant_id,
            sender_id=data.get('sender_id'),
            content=content,
            source_msg_id=data.get('source_msg_id'),
            confirmed_by=str(data.get('confirmed_by') or 'n8n'),
        )
        return web.json_response({'status': 'ok', 'text_hash': text_hash, 'tenant_id': tenant_id})
    except Exception as exc:
        logger.error("Agent ad fingerprint API failed: %s", exc, exc_info=True)
        return web.json_response({'status': 'error', 'message': str(exc)}, status=500)


async def api_agent_delete_ad_fingerprint(request):
    try:
        scope = _request_scope_context(request)
        tenant_id = (
            scope.get('tenant_id') if scope.get('is_merchant')
            else str(request.rel_url.query.get('tenant_id') or 'default').strip()
        )
        deleted = await database.delete_ad_fingerprint(
            tenant_id=tenant_id,
            text_hash=request.match_info.get('text_hash') or '',
        )
        return web.json_response({'status': 'ok', 'deleted': deleted})
    except Exception as exc:
        return web.json_response({'status': 'error', 'message': str(exc)}, status=500)


async def api_ad_pipeline_event(request):
    """n8n/Python hook: ingest one already-received Telegram message into the ad pipeline."""
    try:
        data = await request.json()
        scope = _request_scope_context(request)
        if data.get('tenant_id') and scope.get('role') == 'platform_admin':
            tenant_id = str(data.get('tenant_id') or '').strip()
        else:
            tenant_id = scope.get('tenant_id') if scope.get('is_merchant') else str(data.get('tenant_id') or 'default')
        if tenant_id:
            data['tenant_id'] = tenant_id

        from services.ad_pipeline import get_ad_pipeline
        result = await get_ad_pipeline().upsert_event(
            data,
            compute_embedding=bool(data.get('compute_embedding', False)),
        )
        return web.json_response({'status': 'ok' if result.get('success', True) else 'error', **result})
    except Exception as e:
        logger.error("Ad pipeline event API failed: %s", e, exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_ad_pipeline_backfill(request):
    """Backfill recent SQLite group_messages into the optional pgvector sidecar."""
    try:
        data = await request.json()
        scope = _request_scope_context(request)
        tenant_id = scope.get('tenant_id') if scope.get('is_merchant') else str(data.get('tenant_id') or '').strip() or None
        from services.ad_pipeline import get_ad_pipeline
        result = await get_ad_pipeline().backfill_from_sqlite(
            tenant_id=tenant_id,
            days=max(1, int(data.get('days') or 3)),
            limit=max(1, min(int(data.get('limit') or 1000), 10000)),
        )
        return web.json_response({'status': 'ok' if result.get('success', True) else 'error', **result})
    except Exception as e:
        logger.error("Ad pipeline backfill API failed: %s", e, exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_query_ad_clusters(request):
    """Agent/n8n read tool: query aggregated suspected ad clusters."""
    try:
        data = await request.json() if request.method != 'GET' else dict(request.query)
        scope = _request_scope_context(request)
        body_tenant_id = str(data.get('tenant_id') or (data.get('context') or {}).get('tenant_id') or '').strip() if isinstance(data, dict) else ''
        tenant_id = scope.get('tenant_id') if scope.get('is_merchant') else body_tenant_id or None
        group_id_raw = data.get('group_id') or data.get('chat_id') if isinstance(data, dict) else None
        from services.ad_pipeline import get_ad_pipeline
        result = await get_ad_pipeline().query_clusters(
            tenant_id=tenant_id,
            group_id=int(group_id_raw) if str(group_id_raw or '').strip() else None,
            status=str(data.get('status') or 'pending'),
            limit=max(1, int(data.get('limit') or 20)),
            window_minutes=max(1, int(data.get('window_minutes') or 60)),
        )
        return web.json_response({'status': 'ok' if result.get('success', True) else 'error', **result})
    except Exception as e:
        logger.error("Query ad clusters API failed: %s", e, exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_delete_ad_cluster(request):
    """Agent/n8n action tool: delete messages belonging to a cluster via Python bot instance."""
    try:
        data = await request.json()
        cluster_id = str(data.get('cluster_id') or '').strip()
        if not cluster_id:
            return web.json_response({'status': 'error', 'message': 'cluster_id 不能为空'}, status=400)
        if not _bot_instance:
            return web.json_response({'status': 'error', 'message': 'bot 未初始化'}, status=500)

        from services.ad_pipeline import delete_cluster_messages, get_ad_pipeline
        refs = data.get('message_refs') or []
        if not refs:
            scope = _request_scope_context(request)
            tenant_id = scope.get('tenant_id') if scope.get('is_merchant') else str(data.get('tenant_id') or '').strip() or None
            clusters_result = await get_ad_pipeline().query_clusters(
                tenant_id=tenant_id,
                status='all',
                limit=100,
            )
            clusters = (clusters_result.get('data') or {}).get('clusters') or []
            matched = next((c for c in clusters if str(c.get('cluster_id')) == cluster_id), None)
            refs = (matched or {}).get('message_refs') or []

        result = await delete_cluster_messages(
            cluster={'cluster_id': cluster_id, 'message_refs': refs},
            bot=_bot_instance,
            dry_run=bool(data.get('dry_run', True)),
            ban_senders=bool(data.get('ban_senders', False)),
        )
        return web.json_response({'status': 'ok' if result.get('success', True) else 'error', **result})
    except Exception as e:
        logger.error("Delete ad cluster API failed: %s", e, exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_get_broadcast_targets(request):
    """P2-2：获取 bot 可操作群聊/频道清单"""
    try:
        chat_types_raw = (request.query.get('chat_types') or '').strip()
        chat_types = [x.strip() for x in chat_types_raw.split(',') if x.strip()] if chat_types_raw else None
        targets = await database.get_broadcast_targets(chat_types=chat_types, enabled_only=True)
        return web.json_response({"status": "ok", "data": targets, "total": len(targets)})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_sync_broadcast_targets(request):
    """P2-2：刷新同步 bot 已加入群聊/频道"""
    try:
        stats = await _sync_broadcast_targets_from_bot()
        targets = await database.get_broadcast_targets(chat_types=None, enabled_only=True)
        await notify_web_clients('broadcast_targets_sync', {"stats": stats, "total": len(targets)})
        return web.json_response({"status": "ok", "summary": stats, "data": targets})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_broadcast_to_chats(request):
    """P2-3：面向群/频道的一键群发"""
    try:
        content = ''
        do_translate = False
        chat_ids = []
        retry_count = 1
        image_file = None

        if request.content_type == 'application/json':
            data = await request.json()
            content = (data.get('content') or '').strip()
            do_translate = bool(data.get('do_translate', False))
            chat_ids = data.get('chat_ids') or []
            retry_count = int(data.get('retry_count') or 1)
        else:
            reader = await request.multipart()
            while True:
                part = await reader.next()
                if part is None:
                    break
                if part.name == 'content':
                    content = (await part.text()).strip()
                elif part.name == 'do_translate':
                    do_translate = (await part.text()).lower() == 'true'
                elif part.name == 'chat_ids':
                    chat_ids = json.loads(await part.text())
                elif part.name == 'retry_count':
                    retry_count = int((await part.text()) or 1)
                elif part.name == 'image':
                    image_file = await part.read()
                    if image_file and len(image_file) > _MAX_UPLOAD_SIZE:
                        return web.json_response({"status": "error", "message": f"图片大小超过限制 ({_MAX_UPLOAD_SIZE // 1024 // 1024}MB)"}, status=400)

        if not chat_ids or (not content and not image_file):
            return web.json_response({"status": "error", "message": "参数不完整"}, status=400)

        summary = await _send_broadcast_to_chats(chat_ids=chat_ids, content=content, image_file=image_file, do_translate=do_translate, retries=max(retry_count, 0))
        await notify_web_clients('broadcast_chat_result', summary)
        return web.json_response({"status": "ok", "summary": summary, "failed_items": summary.get('failed_items', [])})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_create_push_task(request):
    """P2-4：创建定时/周期推送任务"""
    try:
        data = await request.json()
        task_name = (data.get('task_name') or '').strip() or f"任务-{int(time.time())}"
        content = (data.get('content') or '').strip()
        do_translate = bool(data.get('do_translate', False))
        target_chat_ids = [int(x) for x in (data.get('target_chat_ids') or []) if str(x).strip()]
        start_at = (data.get('start_at') or '').strip()
        interval_hours = int(data.get('interval_hours') or 6)

        if not target_chat_ids:
            return web.json_response({"status": "error", "message": "请选择至少一个群/频道"}, status=400)
        if not start_at:
            return web.json_response({"status": "error", "message": "start_at 不能为空"}, status=400)
        if interval_hours <= 0:
            return web.json_response({"status": "error", "message": "interval_hours 必须大于 0"}, status=400)

        task_id = await database.create_push_task(
            task_name=task_name,
            content=content,
            image_file_id=None,
            do_translate=do_translate,
            target_chat_ids=target_chat_ids,
            start_at=start_at,
            interval_hours=interval_hours,
            status='running',
            next_run_at=start_at,
        )
        await notify_web_clients('push_task_update', {"task_id": task_id, "action": 'created'})
        return web.json_response({"status": "ok", "task_id": task_id})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_list_push_tasks(request):
    """P2-4：列出推送任务状态"""
    try:
        tasks = await database.list_push_tasks()
        return web.json_response({"status": "ok", "data": tasks, "total": len(tasks)})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_update_push_task_status(request):
    """P2-4：暂停/恢复任务"""
    try:
        data = await request.json()
        task_id = int(data.get('task_id'))
        status = (data.get('status') or '').strip()
        if status not in ('paused', 'running'):
            return web.json_response({"status": "error", "message": "status 仅支持 paused/running"}, status=400)
        ok = await database.update_push_task_status(task_id, status)
        if not ok:
            return web.json_response({"status": "error", "message": "任务不存在"}, status=404)
        await notify_web_clients('push_task_update', {"task_id": task_id, "status": status})
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_run_push_task_now(request):
    """P2-4：立即执行任务一次"""
    try:
        data = await request.json()
        task_id = int(data.get('task_id'))
        tasks = await database.list_push_tasks()
        task = next((t for t in tasks if int(t.get('task_id')) == task_id), None)
        if not task:
            return web.json_response({"status": "error", "message": "任务不存在"}, status=404)

        summary = await _send_broadcast_to_chats(
            chat_ids=task.get('target_chat_ids') or [],
            content=(task.get('content') or ''),
            image_file=None,
            do_translate=bool(task.get('do_translate')),
            retries=1,
        )
        now = datetime.now()
        next_run = now + timedelta(hours=max(int(task.get('interval_hours') or 1), 1))
        await database.update_push_task_run_result(
            task_id=task_id,
            last_run_at=now.isoformat(),
            next_run_at=next_run.isoformat(),
            summary=summary,
            retry_count=int(task.get('retry_count') or 0),
        )
        await notify_web_clients('push_task_update', {"task_id": task_id, "summary": summary, "next_run_at": next_run.isoformat()})
        return web.json_response({"status": "ok", "summary": summary, "next_run_at": next_run.isoformat()})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_list_monitors(request):
    """列出所有钱包监听"""
    try:
        active_only = request.query.get('active_only', 'false').lower() == 'true'
        rows = await database.get_all_monitors_for_admin(active_only=active_only)

        # 附加近 20 笔出入金统计，便于前端展示风险与概览
        for row in rows:
            txs = await database.get_recent_monitor_transactions(row['monitor_id'], limit=20)
            row['inflow_20'] = round(sum(t['amount'] for t in txs if t.get('direction') == 'in'), 2)
            row['outflow_20'] = round(sum(t['amount'] for t in txs if t.get('direction') == 'out'), 2)
            row['profit_20'] = round(row['inflow_20'] - row['outflow_20'], 2)

        return web.json_response({"status": "ok", "data": rows})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_create_monitor(request):
    """新增钱包监听（管理端）"""
    try:
        data = await request.json()
        user_id = int(data.get('user_id') or 0)
        address = (data.get('address') or '').strip()
        network = (data.get('network') or '').strip().lower()
        remark = (data.get('remark') or data.get('tag') or '').strip()

        if not user_id or not address or network not in ('trc20', 'bsc'):
            return web.json_response({"status": "error", "message": "参数不完整或网络类型错误"}, status=400)

        await database.save_wallet_monitor(user_id, address, network, remark=remark)
        await notify_web_clients("monitor_update", {"action": "created", "user_id": user_id, "address": address, "network": network.upper(), "remark": remark})
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_list_user_query_monitors(request):
    """列出其他用户的查询地址（非管理员自有）"""
    try:
        rows = await database.get_user_query_addresses(active_only=True)
        for row in rows:
            txs = await database.get_recent_monitor_transactions(row['monitor_id'], limit=10)
            row['inflow_20'] = round(sum(t['amount'] for t in txs if t.get('direction') == 'in'), 2)
            row['outflow_20'] = round(sum(t['amount'] for t in txs if t.get('direction') == 'out'), 2)
        return web.json_response({"status": "ok", "data": rows})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_claim_monitor(request):
    """将用户查询地址转为管理员自有监听地址"""
    try:
        monitor_id = int(request.match_info['monitor_id'])
        ok = await database.claim_monitor_as_admin(monitor_id)
        if ok:
            await notify_web_clients("monitor_update", {"action": "claimed", "monitor_id": monitor_id})
            return web.json_response({"status": "ok"})
        return web.json_response({"status": "error", "message": "未找到该地址"}, status=404)
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_delete_monitor(request):
    """删除钱包监听（管理端）"""
    try:
        monitor_id = int(request.match_info['monitor_id'])
        ok = await database.delete_monitor_admin(monitor_id)
        if ok:
            await notify_web_clients("monitor_update", {"action": "deleted", "monitor_id": monitor_id})
            return web.json_response({"status": "ok"})
        return web.json_response({"status": "error", "message": "监听不存在"}, status=404)
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_update_monitor_remark(request):
    """更新监听备注"""
    try:
        monitor_id = int(request.match_info['monitor_id'])
        data = await request.json()
        remark = (data.get('remark') or '').strip()
        ok = await database.update_monitor_remark(monitor_id, remark)
        if ok:
            await notify_web_clients('monitor_update', {'action': 'remark_updated', 'monitor_id': monitor_id, 'remark': remark})
            return web.json_response({'status': 'ok'})
        return web.json_response({'status': 'error', 'message': '监听不存在'}, status=404)
    except Exception as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_monitor_pnl_summary(request):
    """获取当日/周/月盈亏台账汇总（支持单地址、多地址过滤）"""
    try:
        uid = request.query.get('user_id')
        user_id = int(uid) if uid else None

        addresses_raw = (request.query.get('addresses') or '').strip()
        addresses = [x.strip() for x in addresses_raw.split(',') if x.strip()] if addresses_raw else []

        monitor_ids_raw = (request.query.get('monitor_ids') or '').strip()
        monitor_ids = [int(x) for x in monitor_ids_raw.split(',') if x.strip().isdigit()] if monitor_ids_raw else []

        day = await database.get_monitor_pnl_summary('day', user_id=user_id, addresses=addresses, monitor_ids=monitor_ids)
        day_prev = await database.get_monitor_pnl_summary('day', user_id=user_id, addresses=addresses, monitor_ids=monitor_ids, previous=True)
        week = await database.get_monitor_pnl_summary('week', user_id=user_id, addresses=addresses, monitor_ids=monitor_ids)
        week_prev = await database.get_monitor_pnl_summary('week', user_id=user_id, addresses=addresses, monitor_ids=monitor_ids, previous=True)
        month = await database.get_monitor_pnl_summary('month', user_id=user_id, addresses=addresses, monitor_ids=monitor_ids)
        month_prev = await database.get_monitor_pnl_summary('month', user_id=user_id, addresses=addresses, monitor_ids=monitor_ids, previous=True)

        def calc_mom(curr: float, prev: float) -> Optional[float]:
            if prev == 0:
                return None if curr == 0 else 100.0
            return ((curr - prev) / abs(prev)) * 100.0

        series_day = await database.get_monitor_period_series('day', user_id=user_id, addresses=addresses, monitor_ids=monitor_ids)
        series_week = await database.get_monitor_period_series('week', user_id=user_id, addresses=addresses, monitor_ids=monitor_ids)
        series_month = await database.get_monitor_period_series('month', user_id=user_id, addresses=addresses, monitor_ids=monitor_ids)
        tx_list = await database.get_monitor_transactions_list('month', user_id=user_id, addresses=addresses, monitor_ids=monitor_ids, limit=300)

        return web.json_response({
            'status': 'ok',
            'data': {
                'day': day,
                'week': week,
                'month': month,
                'comparison': {
                    'day_vs_prev_day': {
                        'current': day['profit'],
                        'previous': day_prev['profit'],
                        'change': day['profit'] - day_prev['profit'],
                        'change_percent': calc_mom(day['profit'], day_prev['profit']),
                    },
                    'week_vs_prev_week': {
                        'current': week['profit'],
                        'previous': week_prev['profit'],
                        'change': week['profit'] - week_prev['profit'],
                        'change_percent': calc_mom(week['profit'], week_prev['profit']),
                    },
                    'month_vs_prev_month': {
                        'current': month['profit'],
                        'previous': month_prev['profit'],
                        'change': month['profit'] - month_prev['profit'],
                        'change_percent': calc_mom(month['profit'], month_prev['profit']),
                    },
                },
                'series': {
                    'day': series_day,
                    'week': series_week,
                    'month': series_month,
                },
                'transactions': tx_list,
            }
        })
    except Exception as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_update_monitor_tx_meta(request):
    """更新链上流水业务字段"""
    try:
        data = await request.json()
        tx_id = (data.get('tx_id') or '').strip()
        if not tx_id:
            return web.json_response({'status': 'error', 'message': 'tx_id 不能为空'}, status=400)

        ok = await database.update_monitor_transaction_meta(
            tx_id=tx_id,
            income_reason=(data.get('income_reason') or '').strip() or None,
            expense_reason=(data.get('expense_reason') or '').strip() or None,
            payer=(data.get('payer') or '').strip() or None,
            payee=(data.get('payee') or '').strip() or None,
        )
        if not ok:
            return web.json_response({'status': 'error', 'message': '流水不存在'}, status=404)
        return web.json_response({'status': 'ok'})
    except Exception as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_batch_update_monitor_tx_meta(request):
    """批量更新链上流水业务字段"""
    try:
        data = await request.json()
        items = data.get('items') or []
        if not isinstance(items, list) or len(items) == 0:
            return web.json_response({'status': 'error', 'message': 'items 不能为空'}, status=400)

        updated = await database.batch_update_monitor_transaction_meta(items)
        return web.json_response({'status': 'ok', 'updated': updated})
    except Exception as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_export_monitor_transactions(request):
    """导出链上流水 Excel（支持 day/week/month/year，自动列宽，对方地址别名填充）"""
    try:
        period = (request.query.get('period') or 'month').strip().lower()
        if period not in ('day', 'week', 'month', 'year'):
            period = 'month'

        uid = request.query.get('user_id')
        user_id = int(uid) if uid else None

        addresses_raw = (request.query.get('addresses') or '').strip()
        addresses = [x.strip() for x in addresses_raw.split(',') if x.strip()] if addresses_raw else []

        monitor_ids_raw = (request.query.get('monitor_ids') or '').strip()
        monitor_ids = [int(x) for x in monitor_ids_raw.split(',') if x.strip().isdigit()] if monitor_ids_raw else []

        annotated_only = (request.query.get('annotated_only') or 'false').lower() == 'true'

        # 对方地址别名字典
        alias_map = await database.get_counterparty_alias_map()

        if period == 'year':
            year_val = int(request.query.get('year') or datetime.now().year)
            start_ms = int(datetime(year_val, 1, 1).timestamp() * 1000)
            end_ms = int(datetime(year_val + 1, 1, 1).timestamp() * 1000)
            rows = await database.get_monitor_transactions_for_export(
                start_ms=start_ms, end_ms=end_ms,
                user_id=user_id, addresses=addresses, monitor_ids=monitor_ids,
                annotated_only=annotated_only,
            )
            pnl = {'start_ms': start_ms, 'end_ms': end_ms,
                   'inflow': sum(float(r.get('amount') or 0) for r in rows if r.get('direction') == 'in'),
                   'outflow': sum(float(r.get('amount') or 0) for r in rows if r.get('direction') == 'out'),
                   'profit': 0}
            pnl['profit'] = round(pnl['inflow'] - pnl['outflow'], 2)
        else:
            pnl = await database.get_monitor_pnl_summary(period, user_id=user_id, addresses=addresses, monitor_ids=monitor_ids)
            rows = await database.get_monitor_transactions_for_export(
                start_ms=pnl['start_ms'], end_ms=pnl['end_ms'],
                user_id=user_id, addresses=addresses, monitor_ids=monitor_ids,
                annotated_only=annotated_only,
            )

        try:
            from openpyxl import Workbook
            from openpyxl.utils import get_column_letter
            from openpyxl.styles import Font, PatternFill, Alignment
        except Exception:
            return web.json_response({'status': 'error', 'message': '缺少 openpyxl 依赖，请先安装'}, status=500)

        wb = Workbook()
        ws = wb.active
        ws.title = '链上流水'
        headers = ['时间', '地址备注', '地址', '网络', '方向', '金额(USDT)', '对方地址', '对方备注',
                   '收入原因', '支出原因', '谁打款', '打款给谁', '风险等级', '风险备注', 'TXID']
        ws.append(headers)
        # 表头加粗+底色
        header_fill = PatternFill('solid', fgColor='1E3A5F')
        for col_idx, _ in enumerate(headers, 1):
            cell = ws.cell(row=1, column=col_idx)
            cell.font = Font(bold=True, color='FFFFFF')
            cell.fill = header_fill
            cell.alignment = Alignment(horizontal='center')

        # 监听地址备注 map
        monitors_list = await database.get_all_monitors_for_admin(active_only=False)
        addr_remark_map = {m['address']: (m.get('remark') or '') for m in monitors_list}

        for r in rows:
            dt = datetime.fromtimestamp(int(r['tx_timestamp']) / 1000).strftime('%Y-%m-%d %H:%M:%S')
            addr = r.get('address') or ''
            counterparty = r.get('counterparty') or ''
            # 自动填充付款方/收款方（若用户未填则用别名）
            direction = r.get('direction', '')
            payer_val = r.get('payer') or ''
            payee_val = r.get('payee') or ''
            if not payer_val and direction == 'in':
                payer_val = alias_map.get(counterparty, '')
            if not payee_val and direction == 'out':
                payee_val = alias_map.get(counterparty, '')
            ws.append([
                dt,
                addr_remark_map.get(addr, ''),
                addr,
                (r.get('network') or '').upper(),
                '收入' if direction == 'in' else '支出',
                float(r.get('amount') or 0),
                counterparty,
                alias_map.get(counterparty, ''),
                r.get('income_reason') or '',
                r.get('expense_reason') or '',
                payer_val,
                payee_val,
                r.get('risk_level') or '',
                r.get('risk_note') or '',
                r.get('tx_id') or '',
            ])

        # 汇总 sheet
        summary_ws = wb.create_sheet('汇总')
        summary_ws.append(['周期', period])
        summary_ws.append(['开始时间', datetime.fromtimestamp(pnl['start_ms'] / 1000).strftime('%Y-%m-%d %H:%M:%S')])
        summary_ws.append(['结束时间', datetime.fromtimestamp(pnl['end_ms'] / 1000).strftime('%Y-%m-%d %H:%M:%S')])
        summary_ws.append(['总收入(USDT)', round(float(pnl['inflow']), 2)])
        summary_ws.append(['总支出(USDT)', round(float(pnl['outflow']), 2)])
        summary_ws.append(['净利润(USDT)', round(float(pnl['profit']), 2)])
        summary_ws.append(['流水总笔数', len(rows)])

        # 年度月度明细 sheet（仅 year 导出时）
        if period == 'year':
            year_val2 = int(request.query.get('year') or datetime.now().year)
            monthly = await database.get_year_monitor_summary(
                user_id=user_id, addresses=addresses, monitor_ids=monitor_ids, year=year_val2
            )
            yw = wb.create_sheet(f'{year_val2}年月度汇总')
            yw.append(['月份', '收入(USDT)', '支出(USDT)', '净利润(USDT)', '交易笔数'])
            for mrow in monthly:
                yw.append([f"{mrow['month']}月", mrow['inflow'], mrow['outflow'], mrow['profit'], mrow['tx_count']])
            # 年度合计行
            yw.append(['全年合计',
                       round(sum(m['inflow'] for m in monthly), 2),
                       round(sum(m['outflow'] for m in monthly), 2),
                       round(sum(m['profit'] for m in monthly), 2),
                       sum(m['tx_count'] for m in monthly)])

        # 自动列宽（所有 sheet）
        for sheet in wb.worksheets:
            for col_cells in sheet.columns:
                max_len = 0
                col_letter = get_column_letter(col_cells[0].column)
                for cell in col_cells:
                    try:
                        val = str(cell.value or '')
                        # 中文字符宽度约2倍
                        clen = sum(2 if '\u4e00' <= c <= '\u9fff' else 1 for c in val)
                        max_len = max(max_len, clen)
                    except Exception:
                        pass
                sheet.column_dimensions[col_letter].width = min(max(max_len + 2, 8), 60)

        bio = io.BytesIO()
        wb.save(bio)
        bio.seek(0)

        filename = f"monitor_export_{period}_{datetime.now().strftime('%Y%m%d_%H%M%S')}.xlsx"
        return web.Response(
            body=bio.getvalue(),
            headers={'Content-Disposition': f'attachment; filename="{filename}"'},
            content_type='application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'
        )
    except Exception as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_get_monitor_transactions(request):
    """按地址+周期分页查询流水（Stripe风格账单面板）"""
    try:
        monitor_id = request.query.get('monitor_id')
        monitor_id = int(monitor_id) if monitor_id else None
        period = (request.query.get('period') or 'month').strip().lower()
        if period not in ('day', 'week', 'month', 'year'):
            period = 'month'
        direction_filter = request.query.get('direction') or None
        page = max(1, int(request.query.get('page') or 1))
        page_size = min(200, max(10, int(request.query.get('page_size') or 50)))
        uid = request.query.get('user_id')
        user_id = int(uid) if uid else None
        result = await database.get_monitor_transactions_paged(
            monitor_id=monitor_id,
            period=period,
            direction_filter=direction_filter,
            page=page,
            page_size=page_size,
            user_id=user_id,
        )
        # 用别名自动填充 counterparty_name
        alias_map = await database.get_counterparty_alias_map()
        for tx in result['txs']:
            cp = tx.get('counterparty') or ''
            tx['counterparty_name'] = alias_map.get(cp, '')
        return web.json_response({'status': 'ok', **result})
    except Exception as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_get_counterparty_aliases(request):
    """获取所有对方地址别名"""
    try:
        rows = await database.get_counterparty_aliases()
        return web.json_response({'status': 'ok', 'data': rows})
    except Exception as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_upsert_counterparty_alias(request):
    """新增或更新对方地址别名"""
    try:
        data = await request.json()
        address = (data.get('address') or '').strip()
        name = (data.get('name') or '').strip()
        note = (data.get('note') or '').strip()
        if not address or not name:
            return web.json_response({'status': 'error', 'message': 'address 和 name 不能为空'}, status=400)
        alias_id = await database.upsert_counterparty_alias(address, name, note)
        return web.json_response({'status': 'ok', 'alias_id': alias_id})
    except Exception as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_delete_counterparty_alias(request):
    """删除对方地址别名"""
    try:
        alias_id = int(request.match_info['alias_id'])
        ok = await database.delete_counterparty_alias(alias_id)
        if not ok:
            return web.json_response({'status': 'error', 'message': '别名不存在'}, status=404)
        return web.json_response({'status': 'ok'})
    except Exception as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def api_year_monitor_summary(request):
    """年度月度盈亏汇总"""
    try:
        year = int(request.query.get('year') or datetime.now().year)
        uid = request.query.get('user_id')
        user_id = int(uid) if uid else None
        addresses_raw = (request.query.get('addresses') or '').strip()
        addresses = [x.strip() for x in addresses_raw.split(',') if x.strip()] if addresses_raw else []
        monitor_ids_raw = (request.query.get('monitor_ids') or '').strip()
        monitor_ids = [int(x) for x in monitor_ids_raw.split(',') if x.strip().isdigit()] if monitor_ids_raw else []
        monthly = await database.get_year_monitor_summary(user_id=user_id, addresses=addresses, monitor_ids=monitor_ids, year=year)
        total_inflow = round(sum(m['inflow'] for m in monthly), 2)
        total_outflow = round(sum(m['outflow'] for m in monthly), 2)
        total_profit = round(sum(m['profit'] for m in monthly), 2)
        return web.json_response({
            'status': 'ok',
            'year': year,
            'monthly': monthly,
            'total': {'inflow': total_inflow, 'outflow': total_outflow, 'profit': total_profit}
        })
    except Exception as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)

        uid = request.query.get('user_id')
        user_id = int(uid) if uid else None

        addresses_raw = (request.query.get('addresses') or '').strip()
        addresses = [x.strip() for x in addresses_raw.split(',') if x.strip()] if addresses_raw else []

        monitor_ids_raw = (request.query.get('monitor_ids') or '').strip()
        monitor_ids = [int(x) for x in monitor_ids_raw.split(',') if x.strip().isdigit()] if monitor_ids_raw else []

        annotated_only = (request.query.get('annotated_only') or 'false').lower() == 'true'

        pnl = await database.get_monitor_pnl_summary(period, user_id=user_id, addresses=addresses, monitor_ids=monitor_ids)
        rows = await database.get_monitor_transactions_for_export(
            start_ms=pnl['start_ms'],
            end_ms=pnl['end_ms'],
            user_id=user_id,
            addresses=addresses,
            monitor_ids=monitor_ids,
            annotated_only=annotated_only,
        )

        try:
            from openpyxl import Workbook
        except Exception:
            return web.json_response({'status': 'error', 'message': '缺少 openpyxl 依赖，请先安装'}, status=500)

        wb = Workbook()
        ws = wb.active
        ws.title = '链上流水'
        ws.append([
            '时间', '地址', '网络', '方向', '金额(USDT)', '对方地址',
            '收入原因', '支出原因', '谁打款', '打款给谁', '风险等级', '风险备注', 'TXID'
        ])

        for r in rows:
            dt = datetime.fromtimestamp(int(r['tx_timestamp']) / 1000).strftime('%Y-%m-%d %H:%M:%S')
            ws.append([
                dt,
                r.get('address') or '',
                (r.get('network') or '').upper(),
                '收入' if r.get('direction') == 'in' else '支出',
                float(r.get('amount') or 0),
                r.get('counterparty') or '',
                r.get('income_reason') or '',
                r.get('expense_reason') or '',
                r.get('payer') or '',
                r.get('payee') or '',
                r.get('risk_level') or '',
                r.get('risk_note') or '',
                r.get('tx_id') or '',
            ])

        summary = wb.create_sheet('汇总')
        summary.append(['周期', period])
        summary.append(['开始时间', datetime.fromtimestamp(pnl['start_ms'] / 1000).strftime('%Y-%m-%d %H:%M:%S')])
        summary.append(['结束时间', datetime.fromtimestamp(pnl['end_ms'] / 1000).strftime('%Y-%m-%d %H:%M:%S')])
        summary.append(['收入', round(float(pnl['inflow']), 2)])
        summary.append(['支出', round(float(pnl['outflow']), 2)])
        summary.append(['盈亏', round(float(pnl['profit']), 2)])

        bio = io.BytesIO()
        wb.save(bio)
        bio.seek(0)

        filename = f"monitor_export_{period}_{datetime.now().strftime('%Y%m%d_%H%M%S')}.xlsx"
        headers = {
            'Content-Disposition': f'attachment; filename="{filename}"'
        }
        return web.Response(
            body=bio.getvalue(),
            headers=headers,
            content_type='application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'
        )
    except Exception as e:
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def _sync_potential_users_from_group_messages(limit: int = 2000):
    """从已记录的群组消息中回填潜在用户。"""
    from handlers.group_monitor import DEFAULT_KEYWORDS, _profile_tags_from_keywords

    limit = max(1, min(int(limit), 10000))

    async with aiosqlite.connect(config.DB_NAME) as db:
        db.row_factory = aiosqlite.Row

        async with db.execute(
            """
            SELECT group_id, group_title, custom_keywords
            FROM tracked_groups
            WHERE is_monitored = 1
            """
        ) as cursor:
            groups = await cursor.fetchall()

        if not groups:
            return {
                "scanned_messages": 0,
                "matched_messages": 0,
                "upserted_users": 0,
                "groups": 0,
            }

        group_kw_map = {}
        group_title_map = {}
        group_ids = []
        for g in groups:
            gid = int(g["group_id"])
            group_ids.append(gid)
            group_title_map[gid] = g["group_title"] or ""
            custom_kws = [k.strip() for k in (g["custom_keywords"] or "").split(",") if k.strip()]
            group_kw_map[gid] = list(set(DEFAULT_KEYWORDS + custom_kws))

        placeholders = ",".join(["?"] * len(group_ids))
        query = f"""
            SELECT group_id, user_id, username, content
            FROM group_messages
            WHERE group_id IN ({placeholders})
              AND sender_type = 'user'
              AND content IS NOT NULL
              AND TRIM(content) <> ''
            ORDER BY COALESCE(message_ts, CAST(strftime('%s', timestamp) AS INTEGER)) DESC
            LIMIT ?
        """

        params = tuple(group_ids + [limit])
        async with db.execute(query, params) as cursor:
            rows = await cursor.fetchall()

    scanned_messages = len(rows)
    matched_messages = 0
    upserted_users = set()

    for row in rows:
        group_id = int(row["group_id"])
        text = (row["content"] or "").strip()
        if not text:
            continue

        keywords_pool = group_kw_map.get(group_id, DEFAULT_KEYWORDS)
        text_lower = text.lower()
        matched_keywords = [kw for kw in keywords_pool if kw and kw.lower() in text_lower]
        profile_tags = _profile_tags_from_keywords(matched_keywords, text)

        if not matched_keywords and not profile_tags:
            continue

        matched_messages += 1
        user_id = int(row["user_id"])
        await database.upsert_potential_user(
            user_id=user_id,
            username=row["username"] or f"User{user_id}",
            first_name=None,
            last_name=None,
            message_text=text,
            keywords=",".join(sorted(set(matched_keywords))),
            profile_tags=profile_tags,
            group_id=group_id,
            group_title=group_title_map.get(group_id, ""),
        )
        upserted_users.add(user_id)

    return {
        "scanned_messages": scanned_messages,
        "matched_messages": matched_messages,
        "upserted_users": len(upserted_users),
        "groups": len(group_ids),
    }


async def api_sync_potential_users(request):
    """手动触发潜在用户回填同步。"""
    try:
        limit = int(request.query.get('limit', 2000))
        stats = await _sync_potential_users_from_group_messages(limit=limit)

        global _tags_cache
        _tags_cache = {}

        await notify_web_clients("potential_sync", {"stats": stats})

        summary = (
            f"同步完成：扫描 {stats['scanned_messages']} 条消息，"
            f"命中 {stats['matched_messages']} 条，"
            f"更新 {stats['upserted_users']} 位潜在用户。"
        )
        return web.json_response({"status": "ok", "summary": summary, "data": stats})
    except Exception as e:
        logger.error(f"同步潜在用户失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_get_potential_users(request):
    """获取潜在用户列表 (支持分页与画像/关键词过滤)"""
    try:
        scope = _request_scope_context(request)
        limit = min(int(request.query.get('limit', 50)), 500)
        offset = int(request.query.get('offset', 0))
        keyword = request.query.get('keyword')
        tag = request.query.get('tag')
        group_id = request.query.get('group_id')
        if group_id: group_id = int(group_id)
        
        result = await database.get_potential_users(
            limit=limit,
            offset=offset,
            keyword=keyword,
            tag=tag,
            group_id=group_id,
            tenant_id=scope['tenant_id'] if scope['is_merchant'] else None,
        )
        return web.json_response({"status": "ok", "data": result['users'], "total": result['total']})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_get_potential_tags(request):
    """获取所有潜在用户画像标签 (60s 缓存)"""
    global _tags_cache
    try:
        scope = _request_scope_context(request)
        now = time.time()
        cache_key = scope['tenant_id'] if scope['is_merchant'] else '__global__'
        cache_entry = _tags_cache.get(cache_key) or {"data": [], "expire_at": 0}
        if now > cache_entry["expire_at"]:
            tags = await database.get_potential_tags(
                tenant_id=scope['tenant_id'] if scope['is_merchant'] else None,
            )
            cache_entry = {"data": tags, "expire_at": now + 60}
            _tags_cache[cache_key] = cache_entry
        return web.json_response({"status": "ok", "data": cache_entry["data"], "total": len(cache_entry["data"])})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_list_groups(request):
    """获取所有追踪的群组列表 (分页 2026 规格，含实时状态)"""
    try:
        scope = _request_scope_context(request)
        page = int(request.query.get('page', 1)); size = int(request.query.get('size', 20))
        result = await database.get_tracked_groups_paginated(
            page,
            size,
            tenant_id=scope['tenant_id'] if scope['is_merchant'] else None,
        )
        
        # 增加实时状态标记
        from datetime import datetime, timedelta
        now = datetime.now()
        threshold = now - timedelta(minutes=10)
        
        for g in result['groups']:
            updated_at = datetime.fromisoformat(g['updated_at'].replace(' ', 'T'))
            g['is_online'] = updated_at > threshold
            
        return web.json_response({"status": "ok", "data": result['groups'], "total": result['total']})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_get_monitor_stats(request):
    """2026 规格：获取群组实时监测面板数据"""
    try:
        groups = await database.get_tracked_groups_with_stats()
        online_count = sum(1 for g in groups if g['is_online'])
        monitored_count = sum(1 for g in groups if g['is_monitored'])
        
        return web.json_response({
            "status": "ok",
            "summary": {
                "total": len(groups),
                "online": online_count,
                "monitored": monitored_count
            },
            "groups": groups
        })
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_toggle_group_listen(request):
    """切换群组监听状态"""
    try:
        scope = _request_scope_context(request)
        data = await request.json()
        group_id = int(data.get('group_id')); listen_status = bool(data.get('listen_status'))
        await database.set_group_listen_status(
            group_id,
            listen_status,
            tenant_id=scope['tenant_id'] if scope['is_merchant'] else None,
        )
        from handlers.group_monitor import clear_group_cache
        clear_group_cache(group_id)
        await notify_web_clients("group_listen_update", {"group_id": group_id, "new_status": listen_status})
        return web.json_response({"code": 200, "msg": "状态已更新", "new_status": listen_status})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_toggle_group_revoke_sync(request):
    """切换群组信息校对（撤回同步删除）开关"""
    try:
        scope = _request_scope_context(request)
        data = await request.json()
        group_id = int(data.get('group_id'))
        enabled = bool(data.get('enabled'))
        await database.set_group_delete_on_revoke(
            group_id,
            enabled,
            tenant_id=scope['tenant_id'] if scope['is_merchant'] else None,
        )
        await notify_web_clients("group_revoke_sync_update", {"group_id": group_id, "enabled": enabled})
        return web.json_response({"status": "ok", "enabled": enabled})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_create_revoke_sync_chat(request):
    """保存信息校对暂存群 ID（管理员手动建群拉入 bot 后提交）"""
    try:
        if not _bot_instance:
            return web.json_response({"status": "error", "message": "bot 未初始化"}, status=500)
        data = await request.json()
        chat_id_input = str(data.get('chat_id', '')).strip()
        if not chat_id_input:
            # 未传 chat_id：返回当前已保存的暂存群信息
            existing = await database.get_admin_setting('revoke_sync_chat_id')
            if existing:
                try:
                    chat = await _bot_instance.get_chat(int(existing))
                    return web.json_response({"status": "ok", "chat_id": int(existing), "title": chat.title, "already_exists": True})
                except Exception:
                    return web.json_response({"status": "ok", "chat_id": int(existing), "title": "(获取失败)", "already_exists": True})
            return web.json_response({"status": "ok", "chat_id": None, "already_exists": False})
        # 验证 bot 是否在该群内
        try:
            chat_id = int(chat_id_input)
            chat = await _bot_instance.get_chat(chat_id)
        except Exception as e:
            return web.json_response({"status": "error", "message": f"无法获取该群信息，请确认 bot 已在群内且群 ID 正确：{e}"}, status=400)
        await database.save_admin_setting('revoke_sync_chat_id', str(chat_id))
        logger.info(f"[信息校对] 暂存群已设置: {chat_id} ({chat.title})")
        return web.json_response({"status": "ok", "chat_id": chat_id, "title": chat.title, "already_exists": False})
    except Exception as e:
        logger.error(f"api_create_revoke_sync_chat error: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_get_revoke_sync_chat(request):
    """获取当前校对暂存群信息"""
    try:
        existing = await database.get_admin_setting('revoke_sync_chat_id')
        if not existing:
            return web.json_response({"status": "ok", "chat_id": None})
        if not _bot_instance:
            return web.json_response({"status": "ok", "chat_id": int(existing)})
        try:
            chat = await _bot_instance.get_chat(int(existing))
            return web.json_response({"status": "ok", "chat_id": int(existing), "title": chat.title})
        except Exception:
            return web.json_response({"status": "ok", "chat_id": int(existing), "title": "(群信息获取失败)"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_promote_potential_user(request):
    """潜在用户转正"""
    try:
        user_id = int(request.match_info['user_id'])
        ok = await database.promote_potential_user_to_member(user_id)
        if ok: await notify_web_clients("potential_promoted", {"user_id": user_id})
        return web.json_response({"status": "ok" if ok else "error"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_mark_read(request):
    """标记已读"""
    try:
        scope = _request_scope_context(request)
        user_id = int(request.match_info['user_id'])
        await database.mark_messages_as_read(
            user_id,
            tenant_id=scope['tenant_id'] if scope['is_merchant'] else None,
        )
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def api_delete_chat(request):
    """软删除会话"""
    try:
        scope = _request_scope_context(request)
        user_id = int(request.match_info['user_id'])
        await database.soft_delete_chat(
            user_id,
            tenant_id=scope['tenant_id'] if scope['is_merchant'] else None,
            merchant_id=scope['merchant_id'] if scope['is_merchant'] else None,
        )
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def api_get_file(request):
    """文件代理"""
    try:
        file_id = request.match_info['file_id']
        if not _bot_instance: return web.Response(status=500)
        file_info = await _bot_instance.get_file(file_id)
        return web.json_response({"status": "ok", "url": file_info.file_path})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def api_translate(request):
    """实时翻译预览"""
    try:
        data = await request.json(); text = data.get('text', ''); target_lang = data.get('target_lang', 'English')
        from services import translator
        lang_map = {'中文': 'zh-CN', 'English': 'en', 'Russian': 'ru', 'Japanese': 'ja'}
        translated = await translator.translate_text_with_shield(text, lang_map.get(target_lang, 'en'), is_to_admin=False)
        return web.json_response({"status": "ok", "translated_text": translated})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def api_upload_image(request):
    """图片上传发送"""
    try:
        user_id = int(request.match_info['user_id']); reader = await request.multipart(); field = await reader.next()
        if not field: return web.json_response({"status": "error"}, status=400)
        file_data = await field.read()
        if len(file_data) > _MAX_UPLOAD_SIZE:
            return web.json_response({"status": "error", "message": f"图片大小超过限制 ({_MAX_UPLOAD_SIZE // 1024 // 1024}MB)"}, status=400)
        msg_db_id = await database.save_message(from_userid=config.ADMIN_ID, to_userid=user_id, content="[图片]", msg_type='photo', is_read=True, username="Admin", status='sending')
        t = asyncio.create_task(_execute_web_send_task(user_id, msg_db_id, "[图片]", "[图片]", file_data, False, None, "text", None, None, "", 0))
        _web_bg_tasks.add(t); t.add_done_callback(_web_bg_tasks.discard)
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def api_get_config(request):
    """系统配置获取"""
    try:
        data = {"BOT_TOKEN": config.BOT_TOKEN, "ADMIN_ID": str(config.ADMIN_ID), "global_auto_translate": await database.get_admin_setting("global_auto_translate", "false")}
        return web.json_response({"status": "ok", "data": data})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def api_save_config(request):
    """配置保存"""
    try:
        data = await request.json()
        if 'global_auto_translate' in data: await database.save_admin_setting("global_auto_translate", str(data['global_auto_translate']).lower())
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def api_get_group_members(request):
    """群成员获取（优先实时）"""
    try:
        group_id = request.query.get('group_id')
        members = await database.get_group_members_live(int(group_id), bot=_bot_instance)

        # 头像补全：并发抓取，尽量全量
        async def _enrich_one(m):
            uid = int(m.get('user_id') or 0)
            avatar_file_id = await _fetch_member_avatar_file_id(uid)
            return {**m, "avatar_file_id": avatar_file_id}

        batch_size = 12
        enriched = []
        for i in range(0, len(members), batch_size):
            chunk = members[i:i + batch_size]
            chunk_rows = await asyncio.gather(*[_enrich_one(x) for x in chunk], return_exceptions=True)
            for idx, row in enumerate(chunk_rows):
                if isinstance(row, Exception):
                    enriched.append({**chunk[idx], "avatar_file_id": None})
                else:
                    enriched.append(row)

        return web.json_response({"status": "ok", "members": enriched})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_get_group_member_permissions(request):
    """获取群成员当前管理员权限（P1-3：支持权限回显与编辑）"""
    try:
        if not _bot_instance:
            return web.json_response({"status": "error", "message": "bot 未初始化"}, status=500)

        group_id = int(request.query.get('group_id'))
        user_id = int(request.query.get('user_id'))

        member = await _bot_instance.get_chat_member(group_id, user_id)
        status = getattr(member, 'status', '')

        perm_keys = [
            'can_manage_chat',
            'can_delete_messages',
            'can_manage_video_chats',
            'can_restrict_members',
            'can_promote_members',
            'can_change_info',
            'can_invite_users',
            'can_pin_messages',
            'can_post_messages',
            'can_edit_messages',
            'can_post_stories',
            'can_edit_stories',
            'can_delete_stories',
            'is_anonymous',
        ]

        permissions = {k: bool(getattr(member, k, False)) for k in perm_keys}
        if status == 'creator':
            for k in perm_keys:
                permissions[k] = True if k != 'is_anonymous' else bool(getattr(member, 'is_anonymous', False))

        return web.json_response({
            "status": "ok",
            "member_status": status,
            "is_admin": status in ('administrator', 'creator'),
            "is_owner": status == 'creator',
            "permissions": permissions,
        })
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_group_member_action(request):
    """群成员基础管理动作：禁言/解禁/移除"""
    try:
        if not _bot_instance:
            return web.json_response({"status": "error", "message": "bot 未初始化"}, status=500)

        data = await request.json()
        group_id = int(data.get('group_id'))
        user_id = int(data.get('user_id'))
        action = (data.get('action') or '').strip().lower()
        duration_minutes = int(data.get('duration_minutes') or 30)

        if action not in ('mute', 'unmute', 'kick'):
            return web.json_response({"status": "error", "message": "不支持的动作"}, status=400)

        member = await _bot_instance.get_chat_member(group_id, user_id)
        if member.status in ('administrator', 'creator'):
            return web.json_response({"status": "error", "message": "不能操作管理员或群主"}, status=400)

        if action == 'mute':
            from telegram import ChatPermissions
            until_date = datetime.utcnow() + timedelta(minutes=max(1, duration_minutes))
            await _bot_instance.restrict_chat_member(
                chat_id=group_id,
                user_id=user_id,
                permissions=ChatPermissions(can_send_messages=False),
                until_date=until_date,
            )
        elif action == 'unmute':
            from telegram import ChatPermissions
            await _bot_instance.restrict_chat_member(
                chat_id=group_id,
                user_id=user_id,
                permissions=ChatPermissions(
                    can_send_messages=True,
                    can_send_polls=True,
                    can_send_other_messages=True,
                    can_add_web_page_previews=True,
                    can_change_info=False,
                    can_invite_users=True,
                    can_pin_messages=False,
                ),
            )
        else:  # kick
            await _bot_instance.ban_chat_member(chat_id=group_id, user_id=user_id)

        await notify_web_clients("group_activity", {"group_id": group_id})
        return web.json_response({"status": "ok", "action": action})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_group_member_action_batch(request):
    """群成员批量管理动作（P2）"""
    try:
        if not _bot_instance:
            return web.json_response({"status": "error", "message": "bot 未初始化"}, status=500)

        data = await request.json()
        group_id = int(data.get('group_id'))
        action = (data.get('action') or '').strip().lower()
        duration_minutes = int(data.get('duration_minutes') or 30)
        user_ids_raw = data.get('user_ids') or []

        if action not in ('mute', 'unmute', 'kick'):
            return web.json_response({"status": "error", "message": "不支持的动作"}, status=400)

        user_ids = []
        for uid in user_ids_raw:
            try:
                user_ids.append(int(uid))
            except Exception:
                continue

        if not user_ids:
            return web.json_response({"status": "error", "message": "未选择成员"}, status=400)

        success_ids = []
        failed = []

        for user_id in user_ids:
            try:
                member = await _bot_instance.get_chat_member(group_id, user_id)
                if member.status in ('administrator', 'creator'):
                    failed.append({"user_id": user_id, "reason": "不能操作管理员或群主"})
                    continue

                if action == 'mute':
                    from telegram import ChatPermissions
                    until_date = datetime.utcnow() + timedelta(minutes=max(1, duration_minutes))
                    await _bot_instance.restrict_chat_member(
                        chat_id=group_id,
                        user_id=user_id,
                        permissions=ChatPermissions(can_send_messages=False),
                        until_date=until_date,
                    )
                elif action == 'unmute':
                    from telegram import ChatPermissions
                    await _bot_instance.restrict_chat_member(
                        chat_id=group_id,
                        user_id=user_id,
                        permissions=ChatPermissions(
                            can_send_messages=True,
                            can_send_polls=True,
                            can_send_other_messages=True,
                            can_add_web_page_previews=True,
                            can_change_info=False,
                            can_invite_users=True,
                            can_pin_messages=False,
                        ),
                    )
                else:
                    await _bot_instance.ban_chat_member(chat_id=group_id, user_id=user_id)

                success_ids.append(user_id)
            except Exception as ex:
                failed.append({"user_id": user_id, "reason": str(ex)})

        await notify_web_clients("group_activity", {"group_id": group_id})
        return web.json_response({
            "status": "ok",
            "action": action,
            "success_ids": success_ids,
            "failed": failed,
            "summary": {
                "total": len(user_ids),
                "success": len(success_ids),
                "failed": len(failed),
            }
        })
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_group_admin_action_batch(request):
    """群管理员批量操作（P2）：批量设管理员 / 批量撤销管理员"""
    try:
        if not _bot_instance:
            return web.json_response({"status": "error", "message": "bot 未初始化"}, status=500)

        data = await request.json()
        group_id = int(data.get('group_id'))
        action = (data.get('action') or '').strip().lower()
        user_ids_raw = data.get('user_ids') or []

        if action not in ('promote', 'demote'):
            return web.json_response({"status": "error", "message": "不支持的动作"}, status=400)

        user_ids = []
        for uid in user_ids_raw:
            try:
                user_ids.append(int(uid))
            except Exception:
                continue

        if not user_ids:
            return web.json_response({"status": "error", "message": "未选择成员"}, status=400)

        success_ids = []
        failed = []

        promote_kwargs = {
            "can_manage_chat": True,
            "can_delete_messages": True,
            "can_manage_video_chats": False,
            "can_restrict_members": True,
            "can_promote_members": False,
            "can_change_info": False,
            "can_invite_users": True,
            "can_pin_messages": True,
            "can_post_messages": False,
            "can_edit_messages": False,
            "can_post_stories": False,
            "can_edit_stories": False,
            "can_delete_stories": False,
            "is_anonymous": False,
        }
        demote_kwargs = {
            "can_manage_chat": False,
            "can_delete_messages": False,
            "can_manage_video_chats": False,
            "can_restrict_members": False,
            "can_promote_members": False,
            "can_change_info": False,
            "can_invite_users": False,
            "can_pin_messages": False,
            "can_post_messages": False,
            "can_edit_messages": False,
            "can_post_stories": False,
            "can_edit_stories": False,
            "can_delete_stories": False,
            "is_anonymous": False,
        }

        for user_id in user_ids:
            try:
                member = await _bot_instance.get_chat_member(group_id, user_id)
                if member.status == 'creator':
                    failed.append({"user_id": user_id, "reason": "不能操作群主"})
                    continue

                if action == 'promote':
                    await _bot_instance.promote_chat_member(chat_id=group_id, user_id=user_id, **promote_kwargs)
                else:
                    await _bot_instance.promote_chat_member(chat_id=group_id, user_id=user_id, **demote_kwargs)

                success_ids.append(user_id)
            except Exception as ex:
                failed.append({"user_id": user_id, "reason": str(ex)})

        await notify_web_clients("group_activity", {"group_id": group_id})
        return web.json_response({
            "status": "ok",
            "action": action,
            "success_ids": success_ids,
            "failed": failed,
            "summary": {
                "total": len(user_ids),
                "success": len(success_ids),
                "failed": len(failed),
            }
        })
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_set_group_admin(request):
    """设定群管理员（P1-2：支持细粒度权限）"""
    try:
        if not _bot_instance:
            return web.json_response({"status": "error", "message": "bot 未初始化"}, status=500)

        data = await request.json()
        group_id = int(data.get('group_id'))
        user_id = int(data.get('user_id'))
        custom_permissions = data.get('permissions') if isinstance(data.get('permissions'), dict) else None

        member = await _bot_instance.get_chat_member(group_id, user_id)
        if member.status == 'creator':
            return web.json_response({"status": "error", "message": "群主无需设置管理员"}, status=400)

        if custom_permissions:
            promote_kwargs = {
                "can_manage_chat": bool(custom_permissions.get("can_manage_chat", True)),
                "can_delete_messages": bool(custom_permissions.get("can_delete_messages", True)),
                "can_manage_video_chats": bool(custom_permissions.get("can_manage_video_chats", False)),
                "can_restrict_members": bool(custom_permissions.get("can_restrict_members", True)),
                "can_promote_members": bool(custom_permissions.get("can_promote_members", False)),
                "can_change_info": bool(custom_permissions.get("can_change_info", False)),
                "can_invite_users": bool(custom_permissions.get("can_invite_users", True)),
                "can_pin_messages": bool(custom_permissions.get("can_pin_messages", True)),
                "can_post_messages": bool(custom_permissions.get("can_post_messages", False)),
                "can_edit_messages": bool(custom_permissions.get("can_edit_messages", False)),
                "can_post_stories": bool(custom_permissions.get("can_post_stories", False)),
                "can_edit_stories": bool(custom_permissions.get("can_edit_stories", False)),
                "can_delete_stories": bool(custom_permissions.get("can_delete_stories", False)),
                "is_anonymous": bool(custom_permissions.get("is_anonymous", False)),
            }
        else:
            promote_kwargs = {
                "can_manage_chat": True,
                "can_delete_messages": True,
                "can_manage_video_chats": False,
                "can_restrict_members": True,
                "can_promote_members": False,
                "can_change_info": False,
                "can_invite_users": True,
                "can_pin_messages": True,
                "can_post_messages": False,
                "can_edit_messages": False,
                "can_post_stories": False,
                "can_edit_stories": False,
                "can_delete_stories": False,
                "is_anonymous": False,
            }

        await _bot_instance.promote_chat_member(
            chat_id=group_id,
            user_id=user_id,
            **promote_kwargs,
        )

        await notify_web_clients("group_activity", {"group_id": group_id})
        return web.json_response({"status": "ok", "message": "管理员设置成功", "permissions": promote_kwargs})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_demote_group_admin(request):
    """撤销群管理员权限（P2）"""
    try:
        if not _bot_instance:
            return web.json_response({"status": "error", "message": "bot 未初始化"}, status=500)

        data = await request.json()
        group_id = int(data.get('group_id'))
        user_id = int(data.get('user_id'))

        member = await _bot_instance.get_chat_member(group_id, user_id)
        if member.status == 'creator':
            return web.json_response({"status": "error", "message": "不能撤销群主权限"}, status=400)

        await _bot_instance.promote_chat_member(
            chat_id=group_id,
            user_id=user_id,
            can_manage_chat=False,
            can_delete_messages=False,
            can_manage_video_chats=False,
            can_restrict_members=False,
            can_promote_members=False,
            can_change_info=False,
            can_invite_users=False,
            can_pin_messages=False,
            can_post_messages=False,
            can_edit_messages=False,
            can_post_stories=False,
            can_edit_stories=False,
            can_delete_stories=False,
            is_anonymous=False,
        )

        await notify_web_clients("group_activity", {"group_id": group_id})
        return web.json_response({"status": "ok", "message": "已撤销管理员权限"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_get_group_messages(request):
    """2026 规格：获取群组消息历史 (用于爬取展示)"""
    try:
        scope = _request_scope_context(request)
        group_id = request.query.get('group_id')
        limit = int(request.query.get('limit', 50))
        offset = int(request.query.get('offset', 0))
        messages = await database.get_group_messages(
            int(group_id),
            limit,
            offset,
            tenant_id=scope['tenant_id'] if scope['is_merchant'] else None,
        )
        return web.json_response({"status": "ok", "messages": messages})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_get_group_photo(request):
    """获取群组真实头像 file_id（通过 bot.get_chat 调用 Telegram API）"""
    try:
        group_id = int(request.query.get('group_id', 0))
        if not group_id:
            return web.json_response({"status": "error", "message": "group_id 不能为空"}, status=400)
        if not _bot_instance:
            return web.json_response({"status": "error", "message": "bot 未初始化"}, status=500)
        chat = await _bot_instance.get_chat(group_id)
        photo = getattr(chat, 'photo', None)
        if photo and photo.small_file_id:
            return web.json_response({"status": "ok", "file_id": photo.small_file_id})
        else:
            return web.json_response({"status": "ok", "file_id": None})
    except Exception as e:
        logger.debug(f"获取群头像失败 group_id={request.query.get('group_id')}: {e}")
        return web.json_response({"status": "ok", "file_id": None})


async def api_get_group_member_count(request):
    """获取群组实际成员总数（通过 Telegram API）"""
    try:
        group_id = int(request.query.get('group_id', 0))
        if not group_id:
            return web.json_response({"status": "error", "message": "group_id 不能为空"}, status=400)
        if not _bot_instance:
            return web.json_response({"status": "error", "message": "bot 未初始化"}, status=500)
        count = await _bot_instance.get_chat_member_count(group_id)
        return web.json_response({"status": "ok", "count": count})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_send_group_message(request):
    """在群内发送消息（非引用）"""
    try:
        data = await request.json()
        group_id = int(data.get('group_id'))
        content = (data.get('content') or '').strip()

        if not content:
            return web.json_response({"status": "error", "message": "消息内容不能为空"}, status=400)
        if not _bot_instance:
            return web.json_response({"status": "error", "message": "bot 未初始化"}, status=500)

        sent = await _bot_instance.send_message(chat_id=group_id, text=content)

        await database.save_group_message(
            group_id=group_id,
            user_id=config.ADMIN_ID,
            username='Admin',
            content=content,
            msg_type='text',
            tg_msg_id=sent.message_id if sent else None,
            sender_type='admin',
            quoted_tg_msg_id=None,
            quoted_text=None,
            message_ts=int(sent.date.timestamp()) if sent and sent.date else None,
        )
        await notify_web_clients("group_activity", {"group_id": group_id})
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_reply_group_message(request):
    """在群内回复指定消息（管理员面板触发）"""
    try:
        data = await request.json()
        group_id = int(data.get('group_id'))
        tg_msg_id = int(data.get('tg_msg_id'))
        content = (data.get('content') or '').strip()
        quoted_text = (data.get('quoted_text') or '').strip()

        if not content:
            return web.json_response({"status": "error", "message": "回复内容不能为空"}, status=400)
        if not _bot_instance:
            return web.json_response({"status": "error", "message": "bot 未初始化"}, status=500)

        sent = await _bot_instance.send_message(
            chat_id=group_id,
            text=content,
            reply_to_message_id=tg_msg_id,
        )

        await database.save_group_message(
            group_id=group_id,
            user_id=config.ADMIN_ID,
            username='Admin',
            content=content,
            msg_type='text',
            tg_msg_id=sent.message_id if sent else None,
            sender_type='admin',
            quoted_tg_msg_id=tg_msg_id,
            quoted_text=quoted_text or None,
            message_ts=int(sent.date.timestamp()) if sent and sent.date else None,
        )
        await notify_web_clients("group_activity", {"group_id": group_id})
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_delete_group_message(request):
    """删除群广告消息（删除 TG 消息 + 清理记录）"""
    try:
        data = await request.json()
        group_id = int(data.get('group_id'))
        tg_msg_id = int(data.get('tg_msg_id'))
        user_id = int(data.get('user_id', 0))

        if not _bot_instance:
            return web.json_response({"status": "error", "message": "bot 未初始化"}, status=500)

        if user_id:
            member = await _bot_instance.get_chat_member(group_id, user_id)
            if member.status in ('administrator', 'creator'):
                return web.json_response({"status": "error", "message": "不能删除群管理员或群主消息"}, status=400)

        await _bot_instance.delete_message(chat_id=group_id, message_id=tg_msg_id)
        await database.delete_group_message(group_id, tg_msg_id)
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def api_update_group_config(request):
    """旧版群组配置更新 (兼容)"""
    try:
        scope = _request_scope_context(request)
        data = await request.json(); group_id = int(data.get('group_id')); is_monitored = bool(data.get('is_monitored')); custom_keywords = data.get('custom_keywords', '')
        tenant_id = scope['tenant_id'] if scope['is_merchant'] else None
        await database.set_group_listen_status(group_id, is_monitored, tenant_id=tenant_id)
        await database.update_group_custom_keywords(group_id, custom_keywords, tenant_id=tenant_id)
        from handlers.group_monitor import clear_group_cache
        clear_group_cache(group_id)
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_get_group_verify_config(request):
    """获取群组入群验证配置"""
    try:
        scope = _request_scope_context(request)
        group_id = int(request.rel_url.query.get('group_id', 0))
        if not group_id:
            return web.json_response({"status": "error", "message": "group_id 不能为空"}, status=400)
        cfg = await database.get_group_verify_config(
            group_id,
            tenant_id=scope['tenant_id'] if scope['is_merchant'] else None,
        )
        return web.json_response({"status": "ok", "config": cfg})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_save_group_verify_config(request):
    """保存群组入群验证配置"""
    try:
        scope = _request_scope_context(request)
        data = await request.json()
        group_id = int(data.get('group_id', 0))
        if not group_id:
            return web.json_response({"status": "error", "message": "group_id 不能为空"}, status=400)
        await database.save_group_verify_config(
            group_id,
            data,
            tenant_id=scope['tenant_id'] if scope['is_merchant'] else None,
        )
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def start_web_server(bot):
    """启动 Web Server"""
    global _push_scheduler_task, _push_scheduler_started
    set_bot_instance(bot)
    app = web.Application(middlewares=[auth_middleware])
    app.router.add_get('/', index_handler)
    app.router.add_get('/admin', index_handler)
    app.router.add_get('/miniapp', index_handler)
    app.router.add_get('/favicon.ico', favicon_handler)
    app.router.add_get('/healthz', api_health)
    app.router.add_get('/api/health', api_health)
    # ── 前端 SPA 静态资源路由（/assets）──
    _dist_dir = _frontend_dist_dir()
    app.router.add_get('/admin/{path_info:.*}', index_handler)
    if os.path.isdir(os.path.join(_dist_dir, 'assets')):
        app.router.add_static('/assets', os.path.join(_dist_dir, 'assets'), name='frontend_assets')
    app.router.add_static('/static', __import__('os').path.join(__import__('os').path.dirname(__import__('os').path.abspath(__file__)), 'static'), show_index=False)
    # ── SPA fallback：非 API/WS/Static 路径默认进入当前 Web Admin。──
    async def _spa_fallback(request):
        return await index_handler(request)

    app.router.add_get('/{path_info:(?!api|ws|static|assets|admin|miniapp).+}', _spa_fallback)
    app.router.add_post('/api/login', api_login)
    app.router.add_post('/api/login/otp', api_login_otp)
    app.router.add_post('/api/login/tma', api_login_tma)
    app.router.add_post('/api/miniapp/bootstrap', api_miniapp_bootstrap)
    app.router.add_get('/api/miniapp/context', api_miniapp_context)
    app.router.add_post('/api/miniapp/logout_revoke', api_miniapp_logout_revoke)
    app.router.add_get('/api/csrf-token', api_get_csrf_token)
    if _saas_proxy_enabled():
        app.router.add_route('*', '/api/session/context', saas_proxy_handler)
    else:
        app.router.add_get('/api/session/context', api_get_session_context)
        app.router.add_post('/api/session/context', api_update_session_context)
    app.router.add_get('/api/ai/pipeline_mode', api_get_ai_pipeline_mode)
    app.router.add_post('/api/ai/pipeline_mode', api_set_ai_pipeline_mode)

    # ── SaaS 商户管理路由 ──────────────────────────────────────────────
    if _saas_proxy_enabled():
        app.router.add_route('*', '/api/platform/{tail:.*}', saas_proxy_handler)
        app.router.add_route('*', '/api/merchant/{tail:.*}', saas_proxy_handler)
        logger.info("SaaS routes proxied to Rust backend: %s", _saas_backend_url())
    else:
        from backend.app.api import saas as saas_api

        # 平台管理员 API
        app.router.add_get('/api/platform/merchants', saas_api.api_platform_list_merchants)
        app.router.add_post('/api/platform/merchants', saas_api.api_platform_create_merchant)
        app.router.add_delete('/api/platform/merchants/{merchant_id}', saas_api.api_platform_delete_merchant)
        app.router.add_post('/api/platform/merchants/confirm_onboarding', saas_api.api_platform_confirm_merchant_onboarding)
        app.router.add_get('/api/platform/plans', saas_api.api_platform_list_plans)
        app.router.add_post('/api/platform/plans', saas_api.api_platform_upsert_plan)
        app.router.add_get('/api/platform/subscriptions', saas_api.api_platform_list_subscriptions)
        app.router.add_get('/api/platform/payment_orders', saas_api.api_platform_list_payment_orders)
        app.router.add_post('/api/platform/payment_orders/confirm', saas_api.api_platform_confirm_payment)
        app.router.add_post('/api/platform/payment_orders/reject', saas_api.api_platform_reject_payment)
        app.router.add_get('/api/platform/runtime_statuses', saas_api.api_platform_list_runtime_statuses)
        app.router.add_get('/api/platform/payment_config', saas_api.api_get_platform_payment_config)
        app.router.add_post('/api/platform/payment_config', saas_api.api_upsert_platform_payment_config)

        # 商户自服务 API
        app.router.add_get('/api/merchant/plans', saas_api.api_merchant_list_plans)
        app.router.add_get('/api/merchant/profile', saas_api.api_merchant_get_profile)
        app.router.add_post('/api/merchant/status', saas_api.api_merchant_update_status)
        app.router.add_post('/api/merchant/bot_token', saas_api.api_merchant_save_bot_token)
        app.router.add_get('/api/merchant/subscription', saas_api.api_merchant_get_subscription)
        app.router.add_post('/api/merchant/payment_orders', saas_api.api_merchant_create_payment_order)
        app.router.add_get('/api/merchant/payment_orders/latest', saas_api.api_merchant_get_latest_payment_order)
        app.router.add_post('/api/merchant/payment_orders/mark_paid', saas_api.api_merchant_mark_payment_order_paid)
        app.router.add_post('/api/merchant/payment_orders/submit_tx', saas_api.api_merchant_submit_payment_tx_hash)
        app.router.add_get('/api/merchant/features', saas_api.api_merchant_get_features)
        app.router.add_get('/api/merchant/ai/quota', saas_api.api_merchant_get_ai_quota)
        app.router.add_get('/api/merchant/ai/usage', saas_api.api_merchant_get_ai_usage)
        app.router.add_get('/api/merchant/audit_logs', saas_api.api_merchant_list_audit_logs)
        app.router.add_get('/api/merchant/payment_config', saas_api.api_merchant_get_platform_payment_config)

    app.router.add_get('/ws', websocket_handler)
    app.router.add_get('/api/users', api_get_users)
    app.router.add_post("/api/ai/chat", api_ai_chat_bridge)
    app.router.add_post("/api/agent/tools/telegram-sdk-query", api_agent_telegram_sdk_query)
    app.router.add_get("/api/agent/tools/manifest", api_agent_tool_manifest)
    app.router.add_get("/api/agent/ad-fingerprints", api_agent_ad_fingerprints)
    app.router.add_post("/api/agent/ad-fingerprints", api_agent_ad_fingerprints)
    app.router.add_delete("/api/agent/ad-fingerprints/{text_hash}", api_agent_delete_ad_fingerprint)
    app.router.add_post("/api/agent/tools/query-ad-clusters", api_query_ad_clusters)
    app.router.add_post("/api/agent/tools/delete-ad-cluster", api_delete_ad_cluster)
    app.router.add_get("/api/ad-clusters", api_query_ad_clusters)
    app.router.add_post("/api/ad-pipeline/events/group-message", api_ad_pipeline_event)
    app.router.add_post("/api/ad-pipeline/backfill", api_ad_pipeline_backfill)
    if not _saas_proxy_enabled():
        from backend.app.api import ai as ai_api
        app.router.add_post("/api/ai/detect_ad", ai_api.api_detect_advertising)
    app.router.add_post('/api/messages/{user_id}/ai', api_ai_chat_bridge)
    app.router.add_get('/api/messages/{user_id}', api_get_messages)
    app.router.add_post('/api/messages/{user_id}', api_send_message)
    app.router.add_post('/api/broadcast', api_broadcast_message)
    app.router.add_get('/api/broadcast/targets', api_get_broadcast_targets)
    app.router.add_post('/api/broadcast/targets/sync', api_sync_broadcast_targets)
    app.router.add_post('/api/broadcast/chats', api_broadcast_to_chats)
    app.router.add_post('/api/broadcast/send', api_broadcast_unified)
    app.router.add_post('/api/broadcast/unified', api_broadcast_unified)
    app.router.add_get('/api/push_tasks', api_list_push_tasks)
    app.router.add_post('/api/push_tasks', api_create_push_task)
    app.router.add_post('/api/push_tasks/status', api_update_push_task_status)
    app.router.add_post('/api/push_tasks/run_now', api_run_push_task_now)
    app.router.add_get('/api/monitors', api_list_monitors)
    app.router.add_post('/api/monitors', api_create_monitor)
    app.router.add_get('/api/monitors/pnl_summary', api_monitor_pnl_summary)
    app.router.add_post('/api/monitors/tx_meta', api_update_monitor_tx_meta)
    app.router.add_post('/api/monitors/tx_meta/batch', api_batch_update_monitor_tx_meta)
    app.router.add_get('/api/monitors/export', api_export_monitor_transactions)
    app.router.add_get('/api/monitors/transactions', api_get_monitor_transactions)
    app.router.add_get('/api/monitors/year_summary', api_year_monitor_summary)
    app.router.add_get('/api/monitors/counterparty_aliases', api_get_counterparty_aliases)
    app.router.add_post('/api/monitors/counterparty_aliases', api_upsert_counterparty_alias)
    app.router.add_delete('/api/monitors/counterparty_aliases/{alias_id}', api_delete_counterparty_alias)
    app.router.add_get('/api/monitors/user_queries', api_list_user_query_monitors)
    app.router.add_post('/api/monitors/{monitor_id}/claim', api_claim_monitor)
    app.router.add_delete('/api/monitors/{monitor_id}', api_delete_monitor)
    app.router.add_post('/api/monitors/{monitor_id}/remark', api_update_monitor_remark)
    app.router.add_get('/api/potential_users/tags', api_get_potential_tags) # 必须早于 potential_users
    app.router.add_post('/api/potential_users/sync', api_sync_potential_users)
    app.router.add_get('/api/potential_users', api_get_potential_users)
    app.router.add_post('/api/potential_users/{user_id}/promote', api_promote_potential_user)
    app.router.add_get('/api/group_listen/list', api_list_groups)
    app.router.add_get('/api/group_listen/stats', api_get_monitor_stats)
    app.router.add_post('/api/group_listen/toggle', api_toggle_group_listen)
    app.router.add_post('/api/group_listen/revoke_sync', api_toggle_group_revoke_sync)
    app.router.add_post('/api/group_listen/create_sync_chat', api_create_revoke_sync_chat)
    app.router.add_get('/api/group_listen/sync_chat_info', api_get_revoke_sync_chat)
    app.router.add_get('/api/groups', api_list_groups) # 兼容
    app.router.add_post('/api/groups/config', api_update_group_config) # 兼容
    app.router.add_post('/api/mark_read/{user_id}', api_mark_read)
    app.router.add_post('/api/delete_chat/{user_id}', api_delete_chat)
    app.router.add_get('/api/group_members', api_get_group_members)
    app.router.add_get('/api/group_members/permissions', api_get_group_member_permissions)
    app.router.add_post('/api/group_members/action', api_group_member_action)
    app.router.add_post('/api/group_members/action_batch', api_group_member_action_batch)
    app.router.add_post('/api/group_members/admin_batch', api_group_admin_action_batch)
    app.router.add_post('/api/group_members/promote_admin', api_set_group_admin)
    app.router.add_post('/api/group_members/demote_admin', api_demote_group_admin)
    app.router.add_get('/api/group_messages', api_get_group_messages)
    app.router.add_get('/api/group_photo', api_get_group_photo)
    app.router.add_get('/api/group_member_count', api_get_group_member_count)
    app.router.add_post('/api/group_messages/send', api_send_group_message)
    app.router.add_get('/api/group_verify_config', api_get_group_verify_config)
    app.router.add_post('/api/group_verify_config', api_save_group_verify_config)

    app.router.add_post('/api/config', api_save_config)
    app.router.add_post('/api/group_messages/reply', api_reply_group_message)
    app.router.add_post('/api/group_messages/delete', api_delete_group_message)
    app.router.add_get('/api/file/{file_id}', api_get_file)
    app.router.add_post('/api/translate', api_translate)
    app.router.add_post('/api/upload/{user_id}', api_upload_image)
    app.router.add_get('/api/config', api_get_config)
    app.router.add_post('/api/config', api_save_config)

    if not _push_scheduler_started:
        _push_scheduler_task = asyncio.create_task(_run_push_scheduler_loop())
        _push_scheduler_started = True
    
    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, '127.0.0.1', 8080)
    await site.start()
    logger.info("Web 管理端已启动: http://127.0.0.1:8080")
