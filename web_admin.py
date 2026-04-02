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
from datetime import datetime, timedelta
import database
import config
import weakref
import re
import aiosqlite
import time
from html import escape
from typing import Optional, Dict, Any, List, Tuple
from services.secret_manager import SecretManager
from backend.api.web_admin_monitor import (
    api_list_monitors,
    validate_wallet_address,
    api_create_monitor,
    api_delete_monitor,
    api_update_monitor_remark,
    api_monitor_pnl_summary,
    api_update_monitor_tx_meta,
    api_batch_update_monitor_tx_meta,
    api_export_monitor_transactions,
    _sync_potential_users_from_group_messages,
    api_sync_potential_users,
    api_get_potential_users,
    api_get_potential_tags,
    api_list_groups,
    api_get_monitor_stats,
    api_toggle_group_listen,
    api_toggle_group_revoke_sync,
    api_create_revoke_sync_chat,
    api_get_revoke_sync_chat,
    api_promote_potential_user,
    api_mark_read,
    api_delete_chat,
)


logger = logging.getLogger(__name__)

# 全局存储 bot 实例以发送消息
_bot_instance = None

# WebSocket 客户端连接池
ws_clients = weakref.WeakSet()

# OTP 票据存储 {otp_str: expire_timestamp}
import secrets as _secrets
_otp_store: dict = {}

# 登录速率限制存储 {ip: [timestamp, ...]}
_login_rate_store: dict = {}

# CSRF Token 存储 {csrf_token: expire_timestamp}
_csrf_token_store: dict = {}

# 加密入口路由存储 {route_token: expire_timestamp}
_entry_route_store: dict = {}
ENTRY_ROUTE_TTL_SECONDS = int(os.getenv("ENTRY_ROUTE_TTL_SECONDS", "120"))

def generate_otp() -> str:
    """生成一个 5 分钟有效的一次性登录票据 (OTP)"""
    now = time.time()
    expired = [k for k, v in _otp_store.items() if v < now]
    for k in expired:
        _otp_store.pop(k, None)
    otp = _secrets.token_urlsafe(24)
    _otp_store[otp] = now + config.SecurityConfig.OTP_EXPIRY_SECONDS
    return otp


def generate_secure_entry_route() -> str:
    """生成短时有效的加密入口路由 token"""
    now = time.time()
    expired = [k for k, v in _entry_route_store.items() if v < now]
    for k in expired:
        _entry_route_store.pop(k, None)

    token = _secrets.token_urlsafe(18)
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
    csrf_token = _secrets.token_urlsafe(32)
    _csrf_token_store[csrf_token] = now + 86400  # 24 小时
    return csrf_token

def _check_login_rate_limit(ip: str) -> bool:
    """检查登录速率限制，返回 True 表示允许登录，False 表示超出限制"""
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

# 2026 规格：发送队列计数器与性能统计
active_send_tasks = 0
last_performance_metrics = {}

# P2-4 任务调度器状态
_push_scheduler_task = None
_push_scheduler_started = False

# 标签缓存与过期时间
_tags_cache = {"data": [], "expire_at": 0}
# 成员头像 file_id 缓存（避免频繁打 Telegram API）
_member_avatar_cache: Dict[int, Dict[str, object]] = {}

AI_ASSISTANT_USER_ID = int(os.getenv("AI_ASSISTANT_USER_ID", "900000001"))
AI_ASSISTANT_NAME = os.getenv("AI_ASSISTANT_NAME", "AI 助手")
AI_ASSISTANT_MODEL = os.getenv("AI_ASSISTANT_MODEL", "gemini-2.5-flash-lite")


def _build_sandbox_id(user_id: Optional[int], chat_id: Optional[int] = None) -> str:
    base = f"{int(user_id or 0)}:{int(chat_id or 0)}"
    digest = hashlib.sha256(base.encode()).hexdigest()[:24]
    return f"sbx_{digest}"


def _estimate_cost_usd(model: str, usage: Dict[str, int]) -> float:
    # 仅用于后台粗略统计，不作为计费依据
    total_tokens = int((usage or {}).get('total_tokens', 0) or 0)
    if total_tokens <= 0:
        return 0.0

    m = (model or '').lower()
    # 可在后台后续调整，这里给出保守估算
    if 'pro' in m:
        per_1k = 0.003
    elif 'flash' in m or 'lite' in m:
        per_1k = 0.0007
    else:
        per_1k = 0.001
    return round((total_tokens / 1000.0) * per_1k, 6)


def _pick_vertex_api_key() -> str:
    return ''


def _mask_sk(token: str) -> str:
    if not token or len(token) < 10:
        return '***'
    return f"{token[:6]}...{token[-4:]}"


def _normalize_oneapi_base_url(raw_url: str) -> str:
    url = (raw_url or '').strip()
    if not url:
        return ''
    if not re.match(r'^https?://', url):
        url = f'https://{url}'
    url = url.rstrip('/')
    if not url.endswith('/v1'):
        url = f"{url}/v1"
    return url


def _to_sk_format(raw_key: str) -> str:
    key = (raw_key or '').strip()
    if not key:
        return ''
    if key.startswith('sk-'):
        return key
    digest = hashlib.sha256(key.encode('utf-8')).hexdigest()[:28]
    return f'sk-{digest}'


def _latest_gemini_models() -> List[str]:
    return [
        'gemini-2.5-pro',
        'gemini-2.5-flash',
        'gemini-2.5-flash-lite',
        'gemini-2.0-flash',
        'gemini-2.0-flash-lite',
        'gemini-1.5-pro',
        'gemini-1.5-flash',
    ]


def _parse_sk_from_auth_header(header: str) -> str:
    if not header or not header.startswith('Bearer '):
        return ''
    return header.split(' ', 1)[1].strip()


def _resolve_sandbox_id_from_request(
    request,
    body: Optional[Dict[str, Any]] = None,
    default_user_id: Optional[int] = None,
) -> str:
    body = body or {}

    explicit = (
        request.headers.get('X-Sandbox-ID')
        or request.query.get('sandbox_id')
        or str(body.get('sandbox_id') or '')
        or str(body.get('session_id') or '')
        or str(body.get('chat_session_id') or '')
        or str(body.get('exec_session_id') or '')
    ).strip()
    if explicit:
        return explicit[:64]

    user_part = (
        request.headers.get('X-User-ID')
        or body.get('user_id')
        or default_user_id
        or 0
    )
    chat_part = (
        request.headers.get('X-Chat-ID')
        or request.query.get('chat_id')
        or body.get('chat_id')
        or body.get('conversation_id')
        or 0
    )
    try:
        return _build_sandbox_id(int(user_part), int(chat_part))
    except Exception:
        return _build_sandbox_id(int(default_user_id or 0), 0)


def _resolve_user_id_from_request(
    request,
    body: Optional[Dict[str, Any]] = None,
    default_user_id: Optional[int] = None,
) -> Optional[int]:
    body = body or {}
    raw = (
        request.headers.get('X-User-ID')
        or request.query.get('user_id')
        or body.get('user_id')
        or body.get('uid')
        or default_user_id
    )
    try:
        return int(raw) if raw is not None else None
    except Exception:
        return int(default_user_id) if default_user_id is not None else None


async def _resolve_vertex_runtime_config() -> Dict[str, str]:
    """兼容旧函数名：实际返回当前 LLM 路由配置（Vertex 或 OpenAI 兼容）"""
    llm_api_mode = (await database.get_admin_setting('llm_api_mode', 'rented') or 'rented').strip().lower()
    llm_model_type = (await database.get_admin_setting('llm_model_type', 'openai') or 'openai').strip().lower()

    model = (
        await database.get_admin_setting('vertex_model', '')
        or await database.get_admin_setting('llm_model', AI_ASSISTANT_MODEL)
        or AI_ASSISTANT_MODEL
    )
    base_url = (await database.get_admin_setting('llm_base_url', config.LLM_BASE_URL) or config.LLM_BASE_URL or '').strip()

    if llm_api_mode == 'own':
        if llm_model_type == 'deepseek':
            own_key = (await SecretManager.get_config('DEEPSEEK_API_KEY') or config.DEEPSEEK_API_KEY or '').strip()
            provider = 'openai_compatible'
        elif llm_model_type == 'gemini':
            own_key = (
                await SecretManager.get_config('VERTEX_API_KEY')
                or await SecretManager.get_config('GEMINI_API_KEY')
                or config.GEMINI_API_KEY
                or ''
            ).strip()
            provider = 'vertex'
        else:
            own_key = (await SecretManager.get_config('LLM_API_KEY') or config.LLM_API_KEY or '').strip()
            provider = 'openai_compatible'

        return {
            'provider': provider,
            'model': model,
            'api_key': own_key,
            'base_url': base_url,
            'key_source': 'own',
            'model_type': llm_model_type,
        }

    # rented 模式：保持原有 Vertex/Gemini 线路
    pool_key = (config.GEMINI_API_KEY or '').strip()
    return {
        'provider': 'vertex',
        'model': model,
        'api_key': pool_key,
        'base_url': '',
        'key_source': 'pool',
        'model_type': 'gemini',
    }


def _normalize_openai_messages(messages: list) -> list:
    normalized = []
    for msg in messages or []:
        if not isinstance(msg, dict):
            continue
        role = str(msg.get('role') or 'user')
        content = msg.get('content', '')
        if isinstance(content, list):
            content = ''.join([
                part.get('text', '')
                for part in content
                if isinstance(part, dict) and part.get('type') == 'text'
            ])
        elif not isinstance(content, str):
            content = str(content)
        normalized.append({'role': role, 'content': content})

    if not normalized:
        normalized = [{'role': 'user', 'content': ''}]
    return normalized


async def _call_openai_compatible_chat(
    messages: list,
    model: str,
    api_key: str,
    base_url: str,
    timeout_seconds: int = 90,
) -> tuple[str, dict]:
    if not api_key:
        raise ValueError('未配置 OneAPI/OpenAI 兼容 Key')

    api_url = (base_url or config.LLM_BASE_URL or '').rstrip('/')
    if not api_url:
        raise ValueError('未配置 OneAPI/OpenAI Base URL')
    if not api_url.endswith('/chat/completions'):
        api_url += '/chat/completions'

    payload = {
        'model': model,
        'messages': _normalize_openai_messages(messages),
        'temperature': 0.6,
    }

    timeout = aiohttp.ClientTimeout(total=timeout_seconds)
    async with aiohttp.ClientSession(timeout=timeout) as session:
        async with session.post(
            api_url,
            headers={
                'Authorization': f'Bearer {api_key}',
                'Content-Type': 'application/json',
            },
            json=payload,
        ) as resp:
            raw = await resp.text()
            if resp.status >= 400:
                raise RuntimeError(f'OneAPI/OpenAI 错误({resp.status}): {raw[:500]}')
            try:
                data = json.loads(raw)
            except Exception:
                raise RuntimeError('OneAPI/OpenAI 返回非 JSON 响应')

    try:
        answer = str(data['choices'][0]['message']['content'] or '')
    except Exception:
        raise RuntimeError('OneAPI/OpenAI 未返回有效内容')

    usage_raw = data.get('usage') or {}
    usage = {
        'prompt_tokens': int(usage_raw.get('prompt_tokens', 0) or 0),
        'completion_tokens': int(usage_raw.get('completion_tokens', 0) or 0),
        'total_tokens': int(usage_raw.get('total_tokens', 0) or 0),
    }
    return answer, usage


async def _call_vertex_chat(
    messages: list,
    model: str,
    sandbox_id: str = 'default',
    user_id: Optional[int] = None,
    channel: str = 'web_ai',
) -> tuple[str, dict]:
    runtime = await _resolve_vertex_runtime_config()
    api_key = runtime.get('api_key', '')
    if not api_key:
        raise ValueError('未配置可用 API Key')

    model_name = (model or runtime.get('model') or AI_ASSISTANT_MODEL).strip() or AI_ASSISTANT_MODEL

    if runtime.get('provider') == 'openai_compatible':
        text, usage = await _call_openai_compatible_chat(
            messages=messages,
            model=model_name,
            api_key=api_key,
            base_url=runtime.get('base_url', ''),
            timeout_seconds=90,
        )
    else:
        url = f"https://aiplatform.googleapis.com/v1/publishers/google/models/{model_name}:streamGenerateContent"
        payload = {
            'contents': _build_gemini_contents_from_openai(messages),
            'generationConfig': {
                'temperature': 0.6,
            },
        }

        timeout = aiohttp.ClientTimeout(total=90)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            async with session.post(
                url,
                params={'key': api_key},
                json=payload,
                headers={
                    'Content-Type': 'application/json',
                    'X-Sandbox-ID': sandbox_id,
                },
            ) as resp:
                raw = await resp.text()
                if resp.status >= 400:
                    raise RuntimeError(f'Vertex API 错误({resp.status}): {raw[:500]}')
                try:
                    data = json.loads(raw)
                except Exception:
                    raise RuntimeError('Vertex API 返回非 JSON 响应')

        text, usage = _extract_gemini_text_and_usage(data)
        if not text:
            raise RuntimeError('Vertex API 未返回文本内容')

    if not usage:
        usage = {'prompt_tokens': 0, 'completion_tokens': 0, 'total_tokens': 0}

    await database.save_ai_usage_log(
        channel=f"{channel}_{'own' if runtime.get('key_source') == 'own' else 'pool'}",
        user_id=user_id,
        sandbox_id=sandbox_id,
        model=model_name,
        prompt_tokens=usage.get('prompt_tokens', 0),
        completion_tokens=usage.get('completion_tokens', 0),
        total_tokens=usage.get('total_tokens', 0),
        estimated_cost_usd=_estimate_cost_usd(model_name, usage),
    )

    return text, usage


def _is_ai_contact(user_id: int) -> bool:
    return int(user_id) == AI_ASSISTANT_USER_ID

def set_bot_instance(bot):
    global _bot_instance
    _bot_instance = bot

async def websocket_handler(request):
    """处理 WebSocket 连接（支持 URL 参数 token 认证）"""
    # 从 URL 查询参数中提取 token（支持 Cloudflare 隧道环境）
    token_from_url = request.rel_url.query.get('token', '')
    
    # 验证 token
    if token_from_url != config.WEB_ADMIN_TOKEN:
        # 尝试从 Cookie 或 Authorization 头获取 token
        auth_header = request.headers.get('Authorization', '')
        cookie_token = request.cookies.get('admin_token', '')
        
        token = None
        if auth_header.startswith('Bearer '):
            token = auth_header.split(' ')[1]
        elif cookie_token:
            token = cookie_token
        elif token_from_url:
            token = token_from_url
        
        # 本地开发环境免认证
        if request.remote not in ('127.0.0.1', 'localhost') and token != config.WEB_ADMIN_TOKEN:
            logger.warning(f"WebSocket 连接被拒绝（认证失败）: 来自 {request.remote}")
            return web.Response(status=401, text="Unauthorized")
    
    ws = web.WebSocketResponse()
    await ws.prepare(request)
    
    ws_clients.add(ws)
    logger.info(f"Web 管理端: WebSocket 客户端已连接 (来自 {request.remote})")
    
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
    """Get member avatar file_id with cache."""
    if not _bot_instance or not user_id:
        return None

    now_ts = time.time()
    cached = _member_avatar_cache.get(int(user_id))
    if cached and float(cached.get('expire_at', 0)) > now_ts:
        return cached.get('file_id')

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

    _member_avatar_cache[int(user_id)] = {
        'file_id': file_id,
        'expire_at': now_ts + (600 if file_id else 60),
    }
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


async def _send_broadcast_to_chats(chat_ids, content, image_file=None, do_translate=False, retries=1, buttons=None):
    """面向群/频道执行广播，返回成功失败汇总。支持自定义按钮"""
    if not _bot_instance:
        return {"total": 0, "success": 0, "failed": 0, "success_ids": [], "failed_items": []}

    from services.security_center import security_check_flow
    is_passed, verified_text, error_msg, _, _ = await security_check_flow(
        content or '', image_bytes=image_file, do_translate=do_translate
    )
    if not is_passed:
        raise ValueError(error_msg or '安全校验失败')

    # 构建 InlineKeyboardButton（自定义跳转按钮）
    reply_markup = None
    if buttons and isinstance(buttons, list):
        from telegram import InlineKeyboardButton, InlineKeyboardMarkup
        keyboard_rows = []
        current_row = []
        for btn in buttons:
            btn_text = (btn.get('text') or '').strip()
            btn_url = (btn.get('url') or '').strip()
            if not (btn_text and btn_url):
                continue

            # 兼容常见输入：@channel / t.me/channel / telegram.me/channel
            if btn_url.startswith('@'):
                btn_url = f"https://t.me/{btn_url[1:]}"
            elif btn_url.startswith('t.me/'):
                btn_url = f"https://{btn_url}"
            elif btn_url.startswith('telegram.me/'):
                btn_url = f"https://{btn_url}"

            # Telegram InlineKeyboardButton 仅接受 http/https URL
            if btn_url.startswith(('http://', 'https://')):
                current_row.append(InlineKeyboardButton(text=btn_text, url=btn_url))
                if len(current_row) >= 2:
                    keyboard_rows.append(current_row)
                    current_row = []
        if current_row:
            keyboard_rows.append(current_row)
        if keyboard_rows:
            reply_markup = InlineKeyboardMarkup(keyboard_rows)

    success_ids = []
    failed_items = []

    for chat_id in list(dict.fromkeys([int(x) for x in chat_ids if str(x).strip()])):
        attempt = 0
        sent_ok = False
        last_err = None
        while attempt <= retries and not sent_ok:
            attempt += 1
            try:
                if image_file:
                    bio = io.BytesIO(image_file)
                    bio.name = 'broadcast.jpg'
                    await _bot_instance.send_photo(chat_id=chat_id, photo=bio, caption=verified_text or None, reply_markup=reply_markup)
                else:
                    await _bot_instance.send_message(chat_id=chat_id, text=verified_text, reply_markup=reply_markup)
                sent_ok = True
                success_ids.append(chat_id)
            except Exception as ex:
                last_err = str(ex)
                if attempt <= retries:
                    await asyncio.sleep(0.5)

        if not sent_ok:
            failed_items.append({"chat_id": chat_id, "reason": last_err or 'unknown'})

    return {
        "total": len(list(dict.fromkeys([int(x) for x in chat_ids if str(x).strip()]))),
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
    2026 规格：Bearer Token 校验中间件（安全加固版）。
    - 使用哈希比较替代明文比较（防时序攻击）
    - 对状态变更请求（POST/DELETE/PATCH/PUT）强制校验 CSRF Token
    """
    # 1. 静态资源与首页免登 (支持 Token/OTP 自动注入)
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

    # 2. 检查 Token
    auth_header = request.headers.get('Authorization')
    cookie_token = request.cookies.get('admin_token')

    token = None
    if auth_header and auth_header.startswith('Bearer '):
        token = auth_header.split(' ')[1]
    elif cookie_token:
        token = cookie_token

    # 3. 校验逻辑：使用哈希比较（修复 P0: 凭证明文比较）
    token_valid = False
    if token:
        token_valid = config.verify_admin_token(token)
        # 修复 High: Token 无过期机制 —— 检查 Token 是否已过期
        if token_valid and _is_token_expired(token):
            logger.warning(f"过期 Token 被拒绝: 来自 {request.remote}")
            token_valid = False

    # 4. 本地开发环境免登 (127.0.0.1)
    if not token_valid:
        if request.remote == '127.0.0.1' or request.host.startswith('localhost:'):
            token_valid = True

    if not token_valid:
        logger.warning(f"拒绝未授权访问: {request.path} 来自 {request.remote}")
        return web.json_response({"status": "error", "message": "Unauthorized"}, status=401)

    # 5. CSRF 校验：对状态变更请求强制验证（修复 P0: 无CSRF保护）
    # 本地开发环境 (127.0.0.1) 和 WebSocket 跳过 CSRF 校验
    is_local = request.remote == '127.0.0.1' or request.host.startswith('localhost:')
    if not is_local and request.method in ('POST', 'DELETE', 'PATCH', 'PUT'):
        # 文件上传接口豁免（multipart 请求无法携带 JSON body CSRF）
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

    # 6. 执行请求处理器，并为响应附加安全响应头（修复 Medium: 无内容安全策略）
    response = await handler(request)
    response.headers['X-Content-Type-Options'] = 'nosniff'
    response.headers['X-Frame-Options'] = 'DENY'
    response.headers['X-XSS-Protection'] = '1; mode=block'
    response.headers['Referrer-Policy'] = 'strict-origin-when-cross-origin'
    # CSP：允许同源资源 + CDN（Vue/Element Plus）
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


# 前端 dist 目录路径
_DIST_DIR = os.path.join(os.path.dirname(__file__), 'static', 'dist')
_INDEX_HTML = os.path.join(_DIST_DIR, 'index.html')


async def secure_entry_handler(request):
    """加密入口路由：校验一次性 token 后跳转到首页，避免固定入口暴露"""
    route_token = request.match_info.get('route_token', '')
    if not consume_secure_entry_route(route_token):
        return web.json_response({"status": "error", "message": "入口已失效，请重新获取"}, status=410)

    otp = request.rel_url.query.get('otp', '')
    target = f"/?otp={otp}" if otp else '/'
    raise web.HTTPFound(target)


async def index_handler(request):
    """根路径：提供 Vite 构建的前端 index.html"""
    try:
        return web.FileResponse(_INDEX_HTML)
    except FileNotFoundError:
        return web.json_response({
            "status": "error",
            "message": "前端未构建，请先运行 npm run build",
            "hint": "cd frontend && npm run build"
        }, status=503)


async def spa_fallback_handler(request):
    """SPA fallback：所有非 /api /ws /static 路径均返回 index.html，支持前端路由"""
    try:
        return web.FileResponse(_INDEX_HTML)
    except FileNotFoundError:
        return web.Response(status=404)


# Token 签发时间存储 {token_hash: issued_at_timestamp}（修复 High: Token 无过期机制）
_token_issued_store: dict = {}

def _issue_token(token: str) -> None:
    """记录 Token 签发时间，用于过期检查"""
    import hashlib
    key = hashlib.sha256(token.encode()).hexdigest()[:16]
    _token_issued_store[key] = time.time()

def _is_token_expired(token: str) -> bool:
    """检查 Token 是否已超过 90 天过期（修复 High: Token 无过期机制）"""
    import hashlib
    key = hashlib.sha256(token.encode()).hexdigest()[:16]
    issued_at = _token_issued_store.get(key)
    if issued_at is None:
        # 未记录签发时间（旧 Token）—— 宽容通过，不强制踢出，仅记录日志
        return False
    expiry_seconds = config.SecurityConfig.TOKEN_EXPIRY_DAYS * 86400
    return (time.time() - issued_at) > expiry_seconds


# CSP 违规报告收集端点（修复 Low: CSP report-uri 未配置）
async def api_csp_report(request):
    """接收浏览器 CSP 违规报告，记录日志"""
    try:
        body = await request.json()
        report = body.get('csp-report', body)
        logger.warning(f"[CSP违规] blocked-uri={report.get('blocked-uri')} "
                       f"violated={report.get('violated-directive')} "
                       f"source={report.get('source-file')}:{report.get('line-number')} "
                       f"来自 {request.remote}")
    except Exception:
        pass
    return web.Response(status=204)


async def api_login(request):
    """POST /api/login — Token 登录，返回 token（安全加固：哈希比较 + 速率限制）"""
    try:
        # 速率限制检查（修复 P0: 无速率限制）
        client_ip = request.remote or '0.0.0.0'
        if not _check_login_rate_limit(client_ip):
            logger.warning(f"登录速率限制触发: 来自 {client_ip}")
            return web.json_response({"status": "error", "message": "请求过于频繁，请稍后再试"}, status=429)

        body = await request.json()
        token = body.get('token', '')
        # 使用哈希比较（修复 P0: 凭证明文存储）
        if token and config.verify_admin_token(token):
            _issue_token(token)  # 记录签发时间（修复 High: Token 无过期机制）
            return web.json_response({"status": "ok", "token": config.WEB_ADMIN_TOKEN})
        logger.warning(f"登录失败（Token 错误）: 来自 {client_ip}")
        return web.json_response({"status": "error", "message": "Token 错误"}, status=401)
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=400)


async def api_login_otp(request):
    """POST /api/login/otp — OTP 一次性票据登录，验证后返回长效 token（安全加固：速率限制）"""
    try:
        # 速率限制检查
        client_ip = request.remote or '0.0.0.0'
        if not _check_login_rate_limit(client_ip):
            logger.warning(f"OTP 登录速率限制触发: 来自 {client_ip}")
            return web.json_response({"status": "error", "message": "请求过于频繁，请稍后再试"}, status=429)

        body = await request.json()
        otp = body.get('otp', '')
        now = time.time()
        expire = _otp_store.get(otp)
        if not expire or expire < now:
            return web.json_response({"status": "error", "message": "OTP 无效或已过期"}, status=401)
        # 一次性消费（已正确实现，确认保留）
        _otp_store.pop(otp, None)
        _issue_token(config.WEB_ADMIN_TOKEN)  # 记录签发时间（修复 High: Token 无过期机制）
        logger.info(f"OTP 登录成功，已消费票据，来自 {client_ip}")
        return web.json_response({"status": "ok", "token": config.WEB_ADMIN_TOKEN})
    except Exception as e:
        logger.error(f"OTP 登录失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=400)


async def api_get_csrf_token(request):
    """GET /api/csrf-token — 获取 CSRF Token（修复 P0: 无CSRF保护）"""
    try:
        csrf_token = generate_csrf_token()
        return web.json_response({"status": "ok", "token": csrf_token})
    except Exception as e:
        logger.error(f"生成 CSRF Token 失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_search(request):
    """GET /api/search?q=keyword&group_id=&user_id=&limit= — 全局搜索私聊和群聊消息"""
    try:
        keyword = request.rel_url.query.get('q', '').strip()
        if not keyword or len(keyword) < 1:
            return web.json_response({"status": "error", "message": "搜索关键词不能为空"}, status=400)
        group_id_str = request.rel_url.query.get('group_id', '')
        user_id_str = request.rel_url.query.get('user_id', '')
        limit = min(int(request.rel_url.query.get('limit', 30)), 100)
        group_id = int(group_id_str) if group_id_str.isdigit() else None
        user_id = int(user_id_str) if user_id_str.isdigit() else None
        results = await database.search_messages(keyword, group_id=group_id, user_id=user_id, limit=limit)
        return web.json_response({"status": "ok", "data": results, "keyword": keyword})
    except Exception as e:
        logger.error(f"api_search error: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def blocked_legacy_static_handler(request):
    """拦截旧版 static/js/*.js 请求，防止浏览器加载错误的旧逻辑。"""
    path = request.path
    logger.warning(f"[旧版资源拦截] 已阻止访问旧版静态文件: {path}")
    return web.Response(
        text=f"// 旧版文件 {path} 已停用，请使用新版前端。",
        content_type='application/javascript',
        status=410,  # Gone
    )


def _build_gemini_contents_from_openai(messages: list) -> list:
    contents = []
    for msg in messages or []:
        role = (msg.get('role') or 'user').lower()
        text = msg.get('content', '')

        if isinstance(text, list):
            text = ''.join([
                part.get('text', '')
                for part in text
                if isinstance(part, dict) and part.get('type') == 'text'
            ])
        elif not isinstance(text, str):
            text = str(text)

        gemini_role = 'model' if role == 'assistant' else 'user'
        if role == 'system':
            gemini_role = 'user'
        contents.append({
            'role': gemini_role,
            'parts': [{'text': text or ''}],
        })
    return contents


def _extract_gemini_text_and_usage(payload) -> tuple[str, dict]:
    chunks = payload if isinstance(payload, list) else [payload]
    texts = []
    usage = {}

    for chunk in chunks:
        if not isinstance(chunk, dict):
            continue
        candidates = chunk.get('candidates') or []
        if candidates:
            parts = (((candidates[0] or {}).get('content') or {}).get('parts') or [])
            texts.extend([str(p.get('text', '')) for p in parts if isinstance(p, dict) and p.get('text')])

        meta = chunk.get('usageMetadata') or {}
        if meta:
            usage = {
                'prompt_tokens': int(meta.get('promptTokenCount', 0) or 0),
                'completion_tokens': int(meta.get('candidatesTokenCount', 0) or 0),
                'total_tokens': int(meta.get('totalTokenCount', 0) or 0),
            }

    return ''.join(texts).strip(), usage



async def api_openai_chat_completions(request):
    """OpenAI 兼容格式转发到 Vertex Gemini（支持 stream SSE）"""
    try:
        body = await request.json()
        messages = body.get('messages') or []
        if not isinstance(messages, list) or not messages:
            return web.json_response({'error': {'message': 'messages 不能为空'}}, status=400)

        model = body.get('model') or AI_ASSISTANT_MODEL
        stream = bool(body.get('stream'))
        sandbox_id = _resolve_sandbox_id_from_request(request, body=body)
        user_id = _resolve_user_id_from_request(request, body=body)

        answer, usage = await _call_vertex_chat(
            messages,
            model=model,
            sandbox_id=sandbox_id,
            user_id=user_id,
            channel='openai_chat_completions',
        )
        now_ts = int(time.time())

        if not stream:
            return web.json_response({
                'id': f'chatcmpl-{now_ts}',
                'object': 'chat.completion',
                'created': now_ts,
                'model': model,
                'choices': [
                    {
                        'index': 0,
                        'message': {'role': 'assistant', 'content': answer},
                        'finish_reason': 'stop',
                    }
                ],
                'usage': usage or {'prompt_tokens': 0, 'completion_tokens': 0, 'total_tokens': 0},
            })

        resp = web.StreamResponse(
            status=200,
            headers={
                'Content-Type': 'text/event-stream; charset=utf-8',
                'Cache-Control': 'no-cache',
                'Connection': 'keep-alive',
            },
        )
        await resp.prepare(request)

        chunk_id = f'chatcmpl-{now_ts}'
        created = now_ts
        await resp.write((
            f"data: {json.dumps({'id': chunk_id, 'object': 'chat.completion.chunk', 'created': created, 'model': model, 'choices': [{'index': 0, 'delta': {'role': 'assistant'}, 'finish_reason': None}]}, ensure_ascii=False)}\n\n"
        ).encode('utf-8'))
        await resp.write((
            f"data: {json.dumps({'id': chunk_id, 'object': 'chat.completion.chunk', 'created': created, 'model': model, 'choices': [{'index': 0, 'delta': {'content': answer}, 'finish_reason': None}]}, ensure_ascii=False)}\n\n"
        ).encode('utf-8'))
        await resp.write((
            f"data: {json.dumps({'id': chunk_id, 'object': 'chat.completion.chunk', 'created': created, 'model': model, 'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'stop'}]}, ensure_ascii=False)}\n\n"
        ).encode('utf-8'))
        await resp.write(b"data: [DONE]\n\n")
        await resp.write_eof()
        return resp
    except ValueError as e:
        return web.json_response({'error': {'message': str(e)}}, status=400)
    except Exception as e:
        logger.error(f'api_openai_chat_completions 错误: {e}')
        return web.json_response({'error': {'message': 'AI 转发失败'}}, status=500)


async def api_openai_models(request):
    """OpenAI 兼容模型列表"""
    try:
        runtime = await _resolve_vertex_runtime_config()
        provider = runtime.get('provider')

        if provider == 'openai_compatible':
            api_key = runtime.get('api_key', '')
            base_url = (runtime.get('base_url') or config.LLM_BASE_URL or '').rstrip('/')
            if not base_url:
                raise ValueError('未配置 OneAPI/OpenAI Base URL')
            if not api_key:
                raise ValueError('未配置 OneAPI/OpenAI Key')

            models_url = base_url if base_url.endswith('/models') else f"{base_url}/models"
            timeout = aiohttp.ClientTimeout(total=20)
            async with aiohttp.ClientSession(timeout=timeout) as session:
                async with session.get(
                    models_url,
                    headers={
                        'Authorization': f'Bearer {api_key}',
                        'Content-Type': 'application/json',
                    },
                ) as resp:
                    raw = await resp.text()
                    if resp.status >= 400:
                        raise RuntimeError(f'OneAPI/OpenAI models 错误({resp.status}): {raw[:500]}')
                    try:
                        data = json.loads(raw)
                    except Exception:
                        raise RuntimeError('OneAPI/OpenAI models 返回非 JSON 响应')

            if isinstance(data, dict) and isinstance(data.get('data'), list):
                return web.json_response({'object': 'list', 'data': data.get('data')})

        # fallback: 本地声明可用模型
        active_model = (runtime.get('model') or AI_ASSISTANT_MODEL).strip() or AI_ASSISTANT_MODEL
        model_ids = []
        for m in [active_model, AI_ASSISTANT_MODEL]:
            if m and m not in model_ids:
                model_ids.append(m)

        now_ts = int(time.time())
        data = [{
            'id': m,
            'object': 'model',
            'created': now_ts,
            'owned_by': 'google-vertex',
        } for m in model_ids]
        return web.json_response({'object': 'list', 'data': data})
    except Exception as e:
        logger.error(f'api_openai_models 错误: {e}')
        return web.json_response({'error': {'message': '获取模型列表失败'}}, status=500)


async def api_openai_exec(request):
    """Exec 协议兼容接口（支持 SSE）"""
    try:
        body = await request.json()
        model = (body.get('model') or AI_ASSISTANT_MODEL).strip() or AI_ASSISTANT_MODEL
        sandbox_id = _resolve_sandbox_id_from_request(request, body=body)
        user_id = _resolve_user_id_from_request(request, body=body)
        stream = bool(body.get('stream'))

        messages = body.get('messages')
        if not isinstance(messages, list) or not messages:
            input_text = body.get('input') or body.get('prompt') or ''
            if isinstance(input_text, list):
                input_text = '\n'.join([str(x) for x in input_text])
            input_text = str(input_text or '').strip()
            if not input_text:
                return web.json_response({'error': {'message': 'messages 或 input 不能为空'}}, status=400)
            messages = [{'role': 'user', 'content': input_text}]

        answer, usage = await _call_vertex_chat(
            messages,
            model=model,
            sandbox_id=sandbox_id,
            user_id=user_id,
            channel='openai_exec',
        )

        if not stream:
            return web.json_response({
                'status': 'ok',
                'model': model,
                'sandbox_id': sandbox_id,
                'output_text': answer,
                'usage': usage or {'prompt_tokens': 0, 'completion_tokens': 0, 'total_tokens': 0},
            })

        resp = web.StreamResponse(
            status=200,
            headers={
                'Content-Type': 'text/event-stream; charset=utf-8',
                'Cache-Control': 'no-cache',
                'Connection': 'keep-alive',
            },
        )
        await resp.prepare(request)
        await resp.write((f"event: message\ndata: {json.dumps({'type': 'message.delta', 'delta': answer, 'model': model, 'sandbox_id': sandbox_id}, ensure_ascii=False)}\n\n").encode('utf-8'))
        await resp.write((f"event: done\ndata: {json.dumps({'type': 'done', 'usage': usage or {'prompt_tokens': 0, 'completion_tokens': 0, 'total_tokens': 0}}, ensure_ascii=False)}\n\n").encode('utf-8'))
        await resp.write_eof()
        return resp
    except ValueError as e:
        return web.json_response({'error': {'message': str(e)}}, status=400)
    except Exception as e:
        logger.error(f'api_openai_exec 错误: {e}')
        return web.json_response({'error': {'message': 'Exec 转发失败'}}, status=500)


async def api_ai_chat(request):
    """给前端 AI 联系人的简化聊天接口"""
    try:
        data = await request.json()
        user_id = int(request.match_info['user_id'])
        if not _is_ai_contact(user_id):
            return web.json_response({'status': 'error', 'message': '非 AI 会话'}, status=400)

        content = (data.get('content') or '').strip()
        if not content:
            return web.json_response({'status': 'error', 'message': 'content 不能为空'}, status=400)

        await database.save_message(
            from_userid=config.ADMIN_ID,
            to_userid=AI_ASSISTANT_USER_ID,
            content=content,
            msg_type='text',
            is_read=True,
            username='Admin',
            status='sent',
            admin_userid=str(config.ADMIN_ID),
        )

        messages = [
            {'role': 'user', 'content': content},
        ]
        sandbox_id = _resolve_sandbox_id_from_request(request, body=data, default_user_id=user_id)
        answer, _ = await _call_vertex_chat(
            messages,
            model=data.get('model') or AI_ASSISTANT_MODEL,
            sandbox_id=sandbox_id,
            user_id=user_id,
            channel='web_ai_chat',
        )

        await database.save_message(
            from_userid=AI_ASSISTANT_USER_ID,
            to_userid=config.ADMIN_ID,
            content=answer,
            msg_type='text',
            is_read=True,
            username=AI_ASSISTANT_NAME,
            status='sent',
        )

        return web.json_response({'status': 'ok', 'reply': answer, 'sandbox_id': sandbox_id})
    except Exception as e:
        logger.error(f'api_ai_chat 错误: {e}')
        return web.json_response({'status': 'error', 'message': 'AI 回复失败'}, status=500)


async def api_get_users(request):
    """获取来访用户列表（包含 AI 助手联系人）"""
    try:
        users = await database.get_inbox_list()

        if not any(int(u.get('from_userid', 0) or 0) == AI_ASSISTANT_USER_ID for u in users):
            users.insert(0, {
                'from_userid': AI_ASSISTANT_USER_ID,
                'user_id': AI_ASSISTANT_USER_ID,
                'from_username': AI_ASSISTANT_NAME,
                'username': AI_ASSISTANT_NAME,
                'summary': '🤖 AI 助手在线',
                'unread_count': 0,
                'last_active': datetime.now().isoformat(),
                'last_msg_time': datetime.now().isoformat(),
                'avatar_file_id': None,
                'is_ai_assistant': True,
            })

        return web.json_response({"status": "ok", "data": users})
    except Exception as e:
        logger.error(f"api_get_users 错误: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def api_get_messages(request):
    """获取与某用户的聊天历史（安全加固：用户隔离 + 消息内容 HTML 转义）"""
    try:
        user_id = int(request.match_info['user_id'])
        page = int(request.query.get('page', 1))
        limit = 50
        offset = (page - 1) * limit

        if not _is_ai_contact(user_id):
            # 修复 High: 消息访问无用户隔离 —— 校验 user_id 是否在本 Bot 的来访用户列表中
            known = await database.get_inbox_list()
            known_ids = {int(u['user_id']) for u in known if u.get('user_id')}
            if known_ids and user_id not in known_ids:
                logger.warning(f"消息隔离拦截: 尝试访问非授权用户 {user_id} 的聊天历史")
                return web.json_response({"status": "error", "message": "无权访问该用户的消息"}, status=403)

        messages = await database.get_chat_history(user_id, limit, offset)

        # 修复 Medium: 消息内容 XSS —— 对 content 字段做 HTML 转义
        for msg in messages:
            if isinstance(msg.get('content'), str):
                msg['content'] = escape(msg['content'])
            if isinstance(msg.get('quoted_text'), str):
                msg['quoted_text'] = escape(msg['quoted_text'])

        return web.json_response({"status": "ok", "data": messages})
    except Exception as e:
        logger.error(f"api_get_messages 错误: {e}")
        # 修复 Medium: 错误消息 XSS —— 不向前端返回原始异常信息
        return web.json_response({"status": "error", "message": "获取消息失败，请重试"}, status=500)


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
                caption_text += f"\n{escape(final_send_content)}"
            sent_msg = await safe_send_photo(
                context=None,
                chat_id=user_id,
                photo=photo_stream,
                caption=caption_text,
                parse_mode="HTML",
                reply_to_message_id=reply_to_msg_id,
            )
        else:
            sent_msg = await safe_send_message(
                context=None,
                chat_id=user_id,
                text=f"{admin_tag}\n{escape(final_send_content)}",
                parse_mode="HTML",
                reply_to_message_id=reply_to_msg_id,
            )
        tg_send_duration = (time.time() - tg_send_start) * 1000
        if sent_msg:
            # 图片消息补写 file_id，避免前端仅显示“[图片]”无法预览
            if image_file:
                try:
                    sent_photo_file_id = None
                    photos = getattr(sent_msg, 'photo', None)
                    if photos and len(photos) > 0:
                        sent_photo_file_id = photos[-1].file_id
                    if sent_photo_file_id:
                        async with aiosqlite.connect(config.DB_NAME) as _db:
                            await _db.execute(
                                "UPDATE messages SET file_id = ? WHERE msg_id = ?",
                                (sent_photo_file_id, msg_db_id),
                            )
                            await _db.commit()
                except Exception as file_id_err:
                    logger.warning(f"补写 file_id 失败 msg_id={msg_db_id}: {file_id_err}")

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
        user_id = int(request.match_info['user_id'])
        
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
                elif part.name == 'image':
                    # 修复 Medium: 文件上传无类型/大小限制
                    max_size = 10 * 1024 * 1024  # 10MB
                    image_buffer = bytearray()
                    while True:
                        chunk = await part.read_chunk(size=64 * 1024)
                        if not chunk:
                            break
                        image_buffer.extend(chunk)
                        if len(image_buffer) > max_size:
                            return web.json_response({"status": "error", "message": "图片大小不能超过 10MB"}, status=400)

                    image_data = bytes(image_buffer)
                    # 校验文件魔数（前4字节）确认为图片类型
                    is_webp = image_data.startswith(b'RIFF') and len(image_data) >= 12 and image_data[8:12] == b'WEBP'
                    _ALLOWED_MAGIC = [
                        b'\xff\xd8\xff',         # JPEG
                        b'\x89PNG',              # PNG
                        b'GIF8',                 # GIF
                    ]
                    if not (is_webp or any(image_data.startswith(m) for m in _ALLOWED_MAGIC)):
                        return web.json_response({"status": "error", "message": "仅支持 JPEG/PNG/GIF/WebP 图片格式"}, status=400)
                    image_file = image_data
                    
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
            admin_userid=str(config.ADMIN_ID)
        )
        
        asyncio.create_task(
            _execute_web_send_task(
                user_id, msg_db_id, raw_input_text, final_send_content, image_file,
                is_actually_translated, quoted_text, quoted_type, quoted_file_id,
                reply_to_msg_id, ocr_tag, security_check_duration,
            )
        )
        return web.json_response({"status": "ok"})
    except Exception as e:
        logger.error(f"api_send_message 错误: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_broadcast_message(request):
    """向多名用户一键发送相同内容"""
    try:
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
                status="sending", admin_userid=str(config.ADMIN_ID)
            )
            count += 1
            asyncio.create_task(_execute_web_send_task(int(uid), msg_db_id, raw_input_text, verified_text, image_file, False, None, "text", None, None, ocr_tag, 0))
        return web.json_response({"status": "ok", "count": count})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


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


# 广播速率限制存储 {ip: [timestamp, ...]}
_broadcast_rate_store: dict = {}

def _check_broadcast_rate_limit(ip: str) -> bool:
    """广播接口速率限制：同一 IP 每分钟最多 10 次（修复 Medium: 广播接口无速率限制）"""
    now = time.time()
    window = 60
    max_calls = 10
    attempts = _broadcast_rate_store.get(ip, [])
    attempts = [t for t in attempts if now - t < window]
    if len(attempts) >= max_calls:
        return False
    attempts.append(now)
    _broadcast_rate_store[ip] = attempts
    return True


def _validate_channel_link(link: str) -> bool:
    """
    校验频道链接格式是否合法（修复 High: 频道链接无规范化验证 CVSS 7.0）。
    支持: @username, https://t.me/username, t.me/username
    拒绝: 任意 http/https 非 t.me 链接、含特殊字符的注入尝试
    """
    import re as _re
    if not link or len(link) > 200:
        return False
    link = link.strip()
    # @username 格式
    if _re.match(r'^@[a-zA-Z][a-zA-Z0-9_]{3,31}$', link):
        return True
    # https://t.me/username 或 t.me/username
    if _re.match(r'^(https://)?t\.me/[a-zA-Z][a-zA-Z0-9_]{3,31}$', link):
        return True
    # https://t.me/joinchat/xxx 邀请链接
    if _re.match(r'^(https://)?t\.me/(joinchat/|\+)[a-zA-Z0-9_\-]{10,}$', link):
        return True
    return False


async def api_broadcast_to_chats(request):
    """P2-3：面向群/频道的一键群发（安全加固：速率限制 + 频道链接验证）"""
    try:
        # 修复 Medium: 广播接口无速率限制
        client_ip = request.remote or '0.0.0.0'
        if not _check_broadcast_rate_limit(client_ip):
            logger.warning(f"广播速率限制触发: 来自 {client_ip}")
            return web.json_response({"status": "error", "message": "广播请求过于频繁，请稍后再试"}, status=429)

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

        if not chat_ids or (not content and not image_file):
            return web.json_response({"status": "error", "message": "参数不完整"}, status=400)

        # 修复 High: 频道链接无规范化验证 —— chat_id 必须为整数（群组ID）或合法频道链接
        validated_chat_ids = []
        for cid in chat_ids:
            cid_str = str(cid).strip()
            # 纯数字（群组/用户 ID）
            if cid_str.lstrip('-').isdigit():
                validated_chat_ids.append(int(cid_str))
            # 频道链接格式
            elif _validate_channel_link(cid_str):
                validated_chat_ids.append(cid_str)
            else:
                logger.warning(f"非法 chat_id 被过滤: {cid_str!r}")
        if not validated_chat_ids:
            return web.json_response({"status": "error", "message": "没有合法的目标群组/频道"}, status=400)

        summary = await _send_broadcast_to_chats(chat_ids=validated_chat_ids, content=content, image_file=image_file, do_translate=do_translate, retries=max(retry_count, 0))
        await notify_web_clients('broadcast_chat_result', summary)
        return web.json_response({"status": "ok", "summary": summary, "failed_items": summary.get('failed_items', [])})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_broadcast_unified(request):
    """
    统一群发 API：支持混合发送（用户 + 群 + 频道）+ 自定义跳转按钮 + 图片
    POST /api/broadcast/unified
    Body (JSON):
    {
        "user_ids": [123, 456],           // 可选：用户 ID 列表
        "chat_ids": [-100xxx, -100yyy],   // 可选：群/频道 ID 列表
        "content": "消息正文",             // 必填：消息内容
        "image": "base64_encoded_or_url", // 可选：图片（multipart 也支持）
        "do_translate": false,            // 可选：是否翻译
        "buttons": [                      // 可选：自定义跳转按钮
            {"text": "👉 了解详情", "url": "https://example.com"},
            {"text": "📢 频道", "url": "https://t.me/channel"}
        ]
    }
    """
    try:
        client_ip = request.remote or '0.0.0.0'
        if not _check_broadcast_rate_limit(client_ip):
            logger.warning(f"广播速率限制触发: 来自 {client_ip}")
            return web.json_response({"status": "error", "message": "广播请求过于频繁，请稍后再试"}, status=429)

        content = ''
        do_translate = False
        user_ids = []
        chat_ids = []
        buttons = []
        image_file = None

        if request.content_type == 'application/json':
            data = await request.json()
            content = (data.get('content') or '').strip()
            do_translate = bool(data.get('do_translate', False))
            user_ids = data.get('user_ids') or []
            chat_ids = data.get('chat_ids') or []
            buttons = data.get('buttons') or []
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
                    user_ids = json.loads(await part.text())
                elif part.name == 'chat_ids':
                    chat_ids = json.loads(await part.text())
                elif part.name == 'buttons':
                    buttons = json.loads(await part.text())
                elif part.name == 'image':
                    image_file = await part.read()

        if not user_ids and not chat_ids:
            return web.json_response({"status": "error", "message": "请选择至少一个发送目标"}, status=400)
        if not content and not image_file:
            return web.json_response({"status": "error", "message": "消息内容或图片至少选填一个"}, status=400)

        # 校验按钮 URL 安全性
        validated_buttons = []
        for btn in buttons:
            btn_text = (btn.get('text') or '').strip()
            btn_url = (btn.get('url') or '').strip()
            if btn_text and btn_url:
                # 兼容 @username 格式，自动转换为 https://t.me/username
                if btn_url.startswith('@'):
                    btn_url = f"https://t.me/{btn_url[1:]}"
                # 只允许 http/https/t.me 链接
                if btn_url.startswith(('http://', 'https://', 'https://t.me/', 't.me/')):
                    validated_buttons.append({"text": btn_text, "url": btn_url})

        # 合并所有目标 ID（用户 + 群/频道）
        all_targets = []
        # 用户 ID（正整数）
        for uid in user_ids:
            uid_str = str(uid).strip()
            if uid_str.isdigit():
                all_targets.append(int(uid_str))
        # 群/频道 ID（可正可负整数）
        for cid in chat_ids:
            cid_str = str(cid).strip()
            if cid_str.lstrip('-').isdigit():
                all_targets.append(int(cid_str))
            elif _validate_channel_link(cid_str):
                # 频道链接需要特殊处理（Telegram API 不直接支持链接发送）
                logger.warning(f"频道链接目标暂不支持直接发送: {cid_str}")

        if not all_targets:
            return web.json_response({"status": "error", "message": "没有合法的目标"}, status=400)

        # 执行广播
        summary = await _send_broadcast_to_chats(
            chat_ids=all_targets,
            content=content,
            image_file=image_file,
            do_translate=do_translate,
            buttons=validated_buttons
        )

        # 记录到数据库（审计追踪）
        for uid in user_ids:
            uid_str = str(uid).strip()
            if uid_str.isdigit():
                await database.save_message(
                    from_userid=config.ADMIN_ID,
                    to_userid=int(uid_str),
                    content=content[:100] + ('...' if len(content) > 100 else ''),
                    msg_type="photo" if image_file else "text",
                    is_read=True,
                    username="Admin",
                    status="sending",
                    admin_userid=str(config.ADMIN_ID)
                )

        await notify_web_clients('broadcast_unified_result', summary)

        # 发送群发报告到管理员 Telegram
        if _bot_instance and config.ADMIN_ID:
            # 目标列表（优先展示提交目标，便于复盘）
            target_list = [str(x) for x in all_targets]
            success_ids = [str(x) for x in (summary.get('success_ids') or [])]
            failed_items = summary.get('failed_items') or []

            report_lines = [
                "📢 <b>群发报告</b>",
                "━━━━━━━━━━━━━━━━",
                f"📊 总计: {summary['total']} 个目标",
                f"✅ 成功: {summary['success']} 个",
                f"❌ 失败: {summary['failed']} 个",
                "━━━━━━━━━━━━━━━━",
                f"📝 内容: {escape(content[:50])}{'...' if len(content) > 50 else ''}",
            ]
            if validated_buttons:
                btn_texts = [f"  🔗 {escape(b['text'])}: {escape(b['url'])}" for b in validated_buttons]
                report_lines.append(f"🔘 按钮 ({len(validated_buttons)}个):")
                report_lines.extend(btn_texts)

            if target_list:
                report_lines.append("━━━━━━━━━━━━━━━━")
                report_lines.append(f"🎯 目标列表 ({len(target_list)}个):")
                for t in target_list[:30]:
                    report_lines.append(f"  • {escape(t)}")
                if len(target_list) > 30:
                    report_lines.append(f"  ... 还有 {len(target_list) - 30} 个")

            if success_ids:
                report_lines.append("━━━━━━━━━━━━━━━━")
                report_lines.append(f"✅ 成功目标 ({len(success_ids)}个):")
                for sid in success_ids[:20]:
                    report_lines.append(f"  • {escape(sid)}")
                if len(success_ids) > 20:
                    report_lines.append(f"  ... 还有 {len(success_ids) - 20} 个")

            if failed_items:
                report_lines.append("━━━━━━━━━━━━━━━━")
                report_lines.append("⚠️ 失败明细:")
                for item in failed_items[:20]:
                    cid = escape(str(item.get('chat_id', 'unknown')))
                    reason = escape(str(item.get('reason', 'unknown'))[:80])
                    report_lines.append(f"  • {cid}: {reason}")
                if len(failed_items) > 20:
                    report_lines.append(f"  ... 还有 {len(failed_items) - 20} 条")

            report_text = "\n".join(report_lines)
            try:
                await _bot_instance.send_message(chat_id=config.ADMIN_ID, text=report_text, parse_mode="HTML")
            except Exception as e:
                logger.warning(f"发送群发报告到 TG 失败: {e}")

        return web.json_response({
            "status": "ok",
            "summary": summary,
            "failed_items": summary.get('failed_items', [])
        })
    except Exception as e:
        logger.error(f"统一群发失败: {e}", exc_info=True)
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
    """P2-4：列出推送任务状态（安全加固：按管理员ID过滤，防止跨租户任务泄露）"""
    try:
        tasks = await database.list_push_tasks()
        # 修复 High: 推送任务无权限隔离 —— 仅返回属于当前管理员的任务
        admin_id = config.ADMIN_ID
        tasks = [t for t in tasks if int(t.get('created_by', admin_id) or admin_id) == admin_id]
        return web.json_response({"status": "ok", "data": tasks, "total": len(tasks)})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_update_push_task_status(request):
    """P2-4：暂停/恢复任务（安全加固：校验任务归属）"""
    try:
        data = await request.json()
        task_id = int(data.get('task_id'))
        status = (data.get('status') or '').strip()
        if status not in ('paused', 'running'):
            return web.json_response({"status": "error", "message": "status 仅支持 paused/running"}, status=400)

        # 修复 High: 推送任务操作无归属校验 —— 确认任务属于当前管理员
        admin_id = config.ADMIN_ID
        tasks = await database.list_push_tasks()
        task = next((t for t in tasks if int(t.get('task_id')) == task_id), None)
        if not task:
            return web.json_response({"status": "error", "message": "任务不存在"}, status=404)
        if int(task.get('created_by', admin_id) or admin_id) != admin_id:
            logger.warning(f"权限拦截: 尝试操作非本管理员任务 task_id={task_id}")
            return web.json_response({"status": "error", "message": "无权操作此任务"}, status=403)

        ok = await database.update_push_task_status(task_id, status)
        if not ok:
            return web.json_response({"status": "error", "message": "任务不存在"}, status=404)
        await notify_web_clients('push_task_update', {"task_id": task_id, "status": status})
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_run_push_task_now(request):
    """P2-4：立即执行任务一次（安全加固：校验任务归属）"""
    try:
        data = await request.json()
        task_id = int(data.get('task_id'))
        tasks = await database.list_push_tasks()
        task = next((t for t in tasks if int(t.get('task_id')) == task_id), None)
        if not task:
            return web.json_response({"status": "error", "message": "任务不存在"}, status=404)

        # 修复 High: 推送任务操作无归属校验
        admin_id = config.ADMIN_ID
        if int(task.get('created_by', admin_id) or admin_id) != admin_id:
            logger.warning(f"权限拦截: 尝试立即执行非本管理员任务 task_id={task_id}")
            return web.json_response({"status": "error", "message": "无权操作此任务"}, status=403)

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




# 钱包地址格式验证正则（修复 P0: 地址输入无格式验证 CVSS 9.1）
import re as _re
_TRON_ADDR_RE = _re.compile(r'^T[1-9A-HJ-NP-Za-km-z]{33}$')
_ETH_BSC_ADDR_RE = _re.compile(r'^0x[0-9a-fA-F]{40}$')










































async def api_delete_single_message(request):
    """删除单条私聊消息（安全加固：校验消息归属，防止跨用户 IDOR 删除）"""
    try:
        user_id = int(request.match_info['user_id'])
        data = await request.json()
        tg_msg_id = data.get('tg_msg_id')
        if not tg_msg_id:
            return web.json_response({"status": "error", "message": "tg_msg_id 不能为空"}, status=400)

        # 修复 High: 消息删除无所有权校验 —— 先确认消息确实属于该 user_id
        async with database.aiosqlite.connect(database.DB_NAME) as db:
            cursor = await db.execute(
                "SELECT 1 FROM messages WHERE tg_msg_id=? AND user_id=?",
                (int(tg_msg_id), user_id)
            )
            row = await cursor.fetchone()
            if not row:
                logger.warning(f"消息删除拦截: tg_msg_id={tg_msg_id} 不属于 user_id={user_id}")
                return web.json_response({"status": "error", "message": "消息不存在或无权删除"}, status=403)

            # 尝试通过 bot 删除 Telegram 端消息
            if _bot_instance:
                try:
                    await _bot_instance.delete_message(chat_id=user_id, message_id=int(tg_msg_id))
                except Exception as del_err:
                    logger.warning(f"删除 TG 消息失败 (可能已过期): {del_err}")

            await db.execute("DELETE FROM messages WHERE tg_msg_id=? AND user_id=?", (int(tg_msg_id), user_id))
            await db.commit()

        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": "删除消息失败"}, status=500)

async def api_get_file(request):
    """文件代理"""
    try:
        file_id = request.match_info['file_id']
        if not _bot_instance:
            return web.Response(status=500)

        file_info = await _bot_instance.get_file(file_id)
        file_path = getattr(file_info, 'file_path', '') or ''
        if not file_path:
            return web.json_response({"status": "error", "message": "file_path 为空"}, status=500)

        # Telegram 有时返回完整 URL，有时返回相对路径
        if file_path.startswith('http'):
            file_url = file_path
        else:
            file_url = f"https://api.telegram.org/file/bot{config.BOT_TOKEN}/{file_path}"
        return web.json_response({"status": "ok", "url": file_url})
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
        msg_db_id = await database.save_message(from_userid=config.ADMIN_ID, to_userid=user_id, content="[图片]", msg_type='photo', is_read=True, username="Admin", status='sending')
        asyncio.create_task(_execute_web_send_task(user_id, msg_db_id, "[图片]", "[图片]", file_data, False, None, "text", None, None, "", 0))
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def api_get_config(request):
    """系统配置获取"""
    try:
        data = {
            "BOT_TOKEN": config.BOT_TOKEN,
            "ADMIN_ID": str(config.ADMIN_ID),
            "global_auto_translate": await database.get_admin_setting("global_auto_translate", "false"),
            "translation_engine": await database.get_admin_setting("translation_engine", "llm"),
            "system_prompt": await database.get_admin_setting("system_prompt", ""),
            "llm_api_mode": await database.get_admin_setting("llm_api_mode", "rented"),
            "llm_model_type": await database.get_admin_setting("llm_model_type", "openai"),
            "llm_base_url": await database.get_admin_setting("llm_base_url", config.LLM_BASE_URL),
            "llm_model": await database.get_admin_setting("llm_model", config.LLM_MODEL_NAME),
            "llm_api_key": "",
            "gemini_model_options": _latest_gemini_models(),
        }
        return web.json_response({"status": "ok", "data": data})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_vertex_oneclick_config(request):
    """Vertex 一键转换为 OneAPI/OpenAI 兼容配置"""
    try:
        data = await request.json()

        vertex_api_key = str(data.get('vertex_api_key', '')).strip()
        if not vertex_api_key:
            return web.json_response({"status": "error", "message": "请先输入 Vertex API Key"}, status=400)

        oneapi_base_url = _normalize_oneapi_base_url(str(data.get('oneapi_base_url', '')).strip())
        if not oneapi_base_url:
            return web.json_response({"status": "error", "message": "请先输入 OneAPI Base URL"}, status=400)

        raw_relay_key = str(data.get('relay_api_key', '')).strip() or vertex_api_key
        relay_api_key = _to_sk_format(raw_relay_key)

        model = str(data.get('model', '')).strip() or AI_ASSISTANT_MODEL
        if not model:
            model = 'gemini-2.5-flash-lite'

        await SecretManager.set_config('GEMINI_API_KEY', vertex_api_key)
        await SecretManager.set_config('LLM_API_KEY', relay_api_key)

        await database.save_admin_setting('llm_api_mode', 'own')
        await database.save_admin_setting('llm_model_type', 'openai')
        await database.save_admin_setting('llm_base_url', oneapi_base_url)
        await database.save_admin_setting('llm_model', model)

        model_infer_url = f"{oneapi_base_url}/chat/completions"
        model_list_url = f"{oneapi_base_url}/models"

        return web.json_response({
            "status": "ok",
            "data": {
                "provider": "oneapi-openai-compatible",
                "model": model,
                "oneapi_base_url": oneapi_base_url,
                "model_infer_url": model_infer_url,
                "model_list_url": model_list_url,
                "relay_api_key": relay_api_key,
                "relay_api_key_masked": _mask_sk(relay_api_key),
                "openai_example": {
                    "base_url": oneapi_base_url,
                    "api_key": relay_api_key,
                    "model": model,
                },
            },
            "message": "已完成一键配置：Vertex → OneAPI(OpenAI 兼容)",
        })
    except Exception as e:
        logger.error(f"api_vertex_oneclick_config 错误: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_save_config(request):
    """配置保存"""
    try:
        data = await request.json()

        if 'global_auto_translate' in data:
            await database.save_admin_setting("global_auto_translate", str(data['global_auto_translate']).lower())

        if 'translation_engine' in data and str(data['translation_engine']).strip():
            await database.save_admin_setting("translation_engine", str(data['translation_engine']).strip())

        if 'system_prompt' in data:
            await database.save_admin_setting("system_prompt", str(data['system_prompt']).strip())

        llm_api_mode = str(data.get('llm_api_mode', '')).strip().lower()
        if llm_api_mode in ('rented', 'own'):
            await database.save_admin_setting("llm_api_mode", llm_api_mode)

        llm_model_type = str(data.get('llm_model_type', '')).strip().lower()
        if llm_model_type in ('openai', 'deepseek', 'gemini'):
            await database.save_admin_setting("llm_model_type", llm_model_type)

        llm_base_url = str(data.get('llm_base_url', '')).strip()
        if llm_base_url:
            await database.save_admin_setting("llm_base_url", llm_base_url)

        llm_model = str(data.get('llm_model', '')).strip()
        if llm_model:
            await database.save_admin_setting("llm_model", llm_model)

        llm_api_key = str(data.get('llm_api_key', '')).strip()
        if llm_api_key:
            key_name = 'LLM_API_KEY'
            if llm_model_type == 'deepseek':
                key_name = 'DEEPSEEK_API_KEY'
            elif llm_model_type == 'gemini':
                key_name = 'GEMINI_API_KEY'
            await SecretManager.set_config(key_name, llm_api_key)

        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def api_get_group_members(request):
    """群成员获取（优先实时）（安全加固：校验群组归属）"""
    try:
        group_id = request.query.get('group_id')
        if not group_id:
            return web.json_response({"status": "error", "message": "group_id 不能为空"}, status=400)

        # 修复 Medium: 群成员获取无群组归属校验
        if not await _check_group_is_tracked(int(group_id)):
            logger.warning(f"权限拦截: 尝试获取非授权群组 {group_id} 的成员")
            return web.json_response({"status": "error", "message": "无权访问此群组"}, status=403)

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


async def _check_group_is_tracked(group_id: int) -> bool:
    """
    校验 group_id 是否在本 Bot 追踪的群组列表中（修复 P0: 群组操作无权限检查）。
    防止攻击者传入任意 group_id 操作非授权群组（BOLA/IDOR 漏洞）。
    """
    try:
        async with aiosqlite.connect(config.DB_NAME, timeout=10) as db:
            cursor = await db.execute(
                "SELECT 1 FROM tracked_groups WHERE group_id = ?",
                (group_id,)
            )
            row = await cursor.fetchone()
            return row is not None
    except Exception as e:
        logger.error(f"群组归属校验失败: {e}")
        return False


async def api_group_member_action(request):
    """群成员基础管理动作：禁言/解禁/移除（安全加固：校验群组归属）"""
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

        # 修复 P0: 群组操作无权限检查 —— 校验群组是否属于本 Bot 管理的群组
        if not await _check_group_is_tracked(group_id):
            logger.warning(f"权限拦截: 尝试操作非授权群组 {group_id}")
            return web.json_response({"status": "error", "message": "无权操作此群组"}, status=403)

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
    """群成员批量管理动作（P2）（安全加固：校验群组归属）"""
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

        # 修复 P0: 群组操作无权限检查
        if not await _check_group_is_tracked(group_id):
            logger.warning(f"权限拦截: 批量操作非授权群组 {group_id}")
            return web.json_response({"status": "error", "message": "无权操作此群组"}, status=403)

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
    """设定群管理员（P1-2：支持细粒度权限）（安全加固：校验群组归属）"""
    try:
        if not _bot_instance:
            return web.json_response({"status": "error", "message": "bot 未初始化"}, status=500)

        data = await request.json()
        group_id = int(data.get('group_id'))
        user_id = int(data.get('user_id'))
        custom_permissions = data.get('permissions') if isinstance(data.get('permissions'), dict) else None

        # 修复 P0: 群组操作无权限检查
        if not await _check_group_is_tracked(group_id):
            logger.warning(f"权限拦截: 尝试在非授权群组 {group_id} 设置管理员")
            return web.json_response({"status": "error", "message": "无权操作此群组"}, status=403)

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
    """撤销群管理员权限（P2）（安全加固：校验群组归属）"""
    try:
        if not _bot_instance:
            return web.json_response({"status": "error", "message": "bot 未初始化"}, status=500)

        data = await request.json()
        group_id = int(data.get('group_id'))
        user_id = int(data.get('user_id'))

        # 修复 P0: 群组操作无权限检查
        if not await _check_group_is_tracked(group_id):
            logger.warning(f"权限拦截: 尝试在非授权群组 {group_id} 撤销管理员")
            return web.json_response({"status": "error", "message": "无权操作此群组"}, status=403)

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
    """2026 规格：获取群组消息历史 (用于爬取展示)（安全加固：校验群组归属）"""
    try:
        group_id = request.query.get('group_id')
        if not group_id:
            return web.json_response({"status": "error", "message": "group_id 不能为空"}, status=400)

        # 修复 Medium: 群组消息访问无归属校验
        if not await _check_group_is_tracked(int(group_id)):
            logger.warning(f"权限拦截: 尝试获取非授权群组 {group_id} 的消息")
            return web.json_response({"status": "error", "message": "无权访问此群组"}, status=403)

        limit = int(request.query.get('limit', 50))
        offset = int(request.query.get('offset', 0))
        messages = await database.get_group_messages(int(group_id), limit, offset)
        return web.json_response({"status": "ok", "messages": messages})
    except Exception as e:
        return web.json_response({"status": "error", "message": "获取群组消息失败"}, status=500)


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
    """在群内发送消息（支持引用回复）"""
    try:
        data = await request.json()
        group_id = int(data.get('group_id'))
        content = (data.get('content') or '').strip()
        # 支持引用回复：前端传入 reply_to_tg_msg_id
        reply_to_tg_msg_id = data.get('reply_to_tg_msg_id') or data.get('reply_to_message_id')
        if reply_to_tg_msg_id:
            reply_to_tg_msg_id = int(reply_to_tg_msg_id)
        quoted_text = (data.get('quoted_text') or '').strip() or None

        if not content:
            return web.json_response({"status": "error", "message": "消息内容不能为空"}, status=400)
        if not _bot_instance:
            return web.json_response({"status": "error", "message": "bot 未初始化"}, status=500)

        sent = await _bot_instance.send_message(
            chat_id=group_id,
            text=content,
            reply_to_message_id=reply_to_tg_msg_id,  # Telegram 端显示为引用回复
        )

        await database.save_group_message(
            group_id=group_id,
            user_id=config.ADMIN_ID,
            username='Admin',
            content=content,
            msg_type='text',
            tg_msg_id=sent.message_id if sent else None,
            sender_type='admin',
            quoted_tg_msg_id=reply_to_tg_msg_id,
            quoted_text=quoted_text,
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
        data = await request.json(); group_id = int(data.get('group_id')); is_monitored = bool(data.get('is_monitored')); custom_keywords = data.get('custom_keywords', '')
        await database.set_group_listen_status(group_id, is_monitored)
        await database.update_group_custom_keywords(group_id, custom_keywords)
        from handlers.group_monitor import clear_group_cache
        clear_group_cache(group_id)
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_get_group_verify_config(request):
    """获取群组入群验证配置"""
    try:
        group_id = int(request.rel_url.query.get('group_id', 0))
        if not group_id:
            return web.json_response({"status": "error", "message": "group_id 不能为空"}, status=400)
        cfg = await database.get_group_verify_config(group_id)
        return web.json_response({"status": "ok", "config": cfg})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_save_group_verify_config(request):
    """保存群组入群验证配置"""
    try:
        data = await request.json()
        group_id = int(data.get('group_id', 0))
        if not group_id:
            return web.json_response({"status": "error", "message": "group_id 不能为空"}, status=400)
        await database.save_group_verify_config(group_id, data)
        # 清除 group_monitor 缓存，使新配置立即生效
        try:
            from handlers.group_monitor import clear_group_cache
            clear_group_cache(group_id)
        except Exception:
            pass
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_get_verify_stats(request):
    """获取群内频道验证统计：已验证 vs 待验证人数"""
    try:
        group_id = int(request.rel_url.query.get('group_id', 0))
        if not group_id:
            return web.json_response({"status": "error", "message": "group_id 不能为空"}, status=400)
        stats = await database.get_verify_stats(group_id)
        return web.json_response({"status": "ok", "data": stats})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_get_verify_growth(request):
    """按天/小时统计通过验证进入频道的用户数"""
    try:
        group_id = request.rel_url.query.get('group_id')
        period = request.rel_url.query.get('period', 'day')  # 'day' | 'hour'
        if group_id:
            group_id = int(group_id)
        data = await database.get_verify_growth(group_id=group_id, period=period)
        summary = await database.get_verify_growth_summary()
        return web.json_response({"status": "ok", "data": data, "summary": summary})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def start_web_server(bot):
    """启动 Web Server"""
    global _push_scheduler_task, _push_scheduler_started
    set_bot_instance(bot)
    app = web.Application(
        middlewares=[auth_middleware],
        # 允许较大图片上传，避免群发带图时触发 413 导致前端显示“网络错误”
        client_max_size=15 * 1024**2,
    )
    app.router.add_get('/', index_handler)
    app.router.add_get('/e/{route_token}', secure_entry_handler)
    app.router.add_get('/ws', websocket_handler)
    # 前端静态资源（Vite 构建产物）
    if os.path.isdir(_DIST_DIR):
        app.router.add_static('/assets', os.path.join(_DIST_DIR, 'assets'), name='frontend_assets')
    # SPA fallback：前端路由（非 /api /ws /static 开头的路径）
    app.router.add_get('/{path_info:(?!api|ws|static).+}', spa_fallback_handler)

    # OTP / Token 登录接口（免 auth 中间件，需在 auth_middleware 白名单里）
    app.router.add_post('/api/csp-report', api_csp_report)
    app.router.add_post('/api/login', api_login)
    app.router.add_post('/api/login/otp', api_login_otp)
    app.router.add_get('/api/csrf-token', api_get_csrf_token)
    app.router.add_get('/api/search', api_search)

    # 旧版 static/js/*.js 拦截（返回 410 Gone，防止浏览器加载旧逻辑）
    app.router.add_get('/static/js/{filename:(?!vendor/).+}', blocked_legacy_static_handler)
    app.router.add_get('/api/users', api_get_users)
    app.router.add_get('/api/messages/{user_id}', api_get_messages)
    app.router.add_post('/api/messages/{user_id}', api_send_message)
    app.router.add_post('/api/messages/{user_id}/ai', api_ai_chat)
    app.router.add_post('/v1/chat/completions', api_openai_chat_completions)
    app.router.add_get('/v1/models', api_openai_models)
    app.router.add_post('/v1/exec', api_openai_exec)
    # OpenAI 兼容别名（供前端调试面板直接调用）
    app.router.add_get('/api/openai/models', api_openai_models)
    app.router.add_post('/api/openai/exec', api_openai_exec)
    app.router.add_post('/api/messages/{user_id}/delete', api_delete_single_message)
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
    app.router.add_get('/api/group_stats/verify_stats', api_get_verify_stats)
    app.router.add_get('/api/group_stats/verify_growth', api_get_verify_growth)
    app.router.add_get('/api/group_listen/verified_users', get_verified_users_handler)
    app.router.add_get('/api/group_listen/unverified_users', get_unverified_users_handler)
    app.router.add_post('/api/group_listen/send_channel_reminder', send_channel_reminder_handler)
    app.router.add_post('/api/group_listen/delete_reminder_message', delete_reminder_message_handler)

    app.router.add_post('/api/group_messages/reply', api_reply_group_message)
    app.router.add_post('/api/group_messages/delete', api_delete_group_message)
    app.router.add_get('/api/file/{file_id}', api_get_file)
    app.router.add_post('/api/translate', api_translate)
    app.router.add_post('/api/upload/{user_id}', api_upload_image)
    app.router.add_get('/api/config', api_get_config)
    app.router.add_post('/api/config', api_save_config)
    app.router.add_post('/api/config/vertex/oneclick', api_vertex_oneclick_config)

    # 注册财务路由（/api/v1/monitors/finance/*）
    await register_finance_routes(app)

    if not _push_scheduler_started:
        _push_scheduler_task = asyncio.create_task(_run_push_scheduler_loop())
        _push_scheduler_started = True
    
    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, '127.0.0.1', 8080)
    await site.start()
    logger.info("Web 管理端已启动: http://127.0.0.1:8080")


# ============ 已验证用户 API 端点 (v5.1+) ============

async def get_verified_users_handler(request):
    """GET /api/group_listen/verified_users - 获取已验证用户列表"""
    try:
        group_id = request.rel_url.query.get('group_id')
        period = request.rel_url.query.get('period', 'all')
        
        logger.info(f"获取已验证用户: group_id={group_id}, period={period}")
        
        if period not in ('today', 'all'):
            period = 'all'
        
        if group_id:
            # 特定群组的已验证用户
            group_id = int(group_id)
            users = await database.get_verified_users(group_id, period)
            logger.info(f"特定群组查询: group_id={group_id}, 返回 {len(users)} 个用户")
        else:
            # 全局已验证用户（所有群组）
            users = await database.get_all_verified_users(period)
            logger.info(f"全局查询: 返回 {len(users)} 个用户")
        
        return web.json_response({
            'status': 'ok',
            'data': users,
            'count': len(users)
        })
    except Exception as e:
        logger.error(f"获取已验证用户列表失败: {e}", exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)

async def get_verify_growth_summary_handler(request):
    """GET /api/group_stats/verify_growth - 获取验证增长统计"""
    try:
        group_id = request.rel_url.query.get('group_id')
        
        if group_id:
            group_id = int(group_id)
        else:
            group_id = None
        
        summary = await database.get_verify_growth_summary(group_id)
        
        return web.json_response({
            'status': 'ok',
            'summary': summary
        })
    except Exception as e:
        logger.error(f"获取验证统计失败: {e}")
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


# ============ 未关注频道用户提醒 API (v5.3+) ============

async def get_unverified_users_handler(request):
    """GET /api/group_listen/unverified_users - 获取未关注频道的用户列表"""
    try:
        group_id = request.rel_url.query.get('group_id')
        period = request.rel_url.query.get('period', 'all')
        
        logger.info(f"获取未关注用户: group_id={group_id}, period={period}")
        
        if period not in ('today', 'all'):
            period = 'all'
        
        if group_id:
            group_id = int(group_id)
        
        users = await database.get_unverified_users(group_id, period)
        logger.info(f"返回 {len(users)} 个未关注用户")
        
        return web.json_response({
            'status': 'ok',
            'data': users,
            'count': len(users)
        })
    except Exception as e:
        logger.error(f"获取未关注用户列表失败: {e}", exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


async def send_channel_reminder_handler(request):
    """POST /api/group_listen/send_channel_reminder - 发送频道关注提醒"""
    try:
        data = await request.json()
        group_id = int(data.get('group_id')) if data.get('group_id') is not None else None
        user_ids = data.get('user_ids', [])
        channel_link = data.get('channel_link', '').strip()
        
        if not group_id or not user_ids:
            return web.json_response({'status': 'error', 'message': '缺少必要参数'}, status=400)
        
        if not channel_link:
            return web.json_response({'status': 'error', 'message': '频道链接不能为空'}, status=400)
        
        # 规范化频道链接格式
        # 支持: @xxx, https://t.me/xxx, t.me/xxx, /xxx 等格式
        normalized_link = normalize_channel_link(channel_link)
        
        logger.info(f"发送频道提醒: group_id={group_id}, user_count={len(user_ids)}, channel_link={channel_link} -> {normalized_link}")
        
        if not _bot_instance:
            return web.json_response({'status': 'error', 'message': '机器人未初始化'}, status=500)
        
        from datetime import datetime, timedelta
        from telegram import InlineKeyboardButton, InlineKeyboardMarkup

        channel_id_for_callback = group_id
        try:
            verify_cfg = await database.get_group_verify_config(group_id)
            cfg_channel_id = verify_cfg.get('verify_channel_id') if isinstance(verify_cfg, dict) else None
            if cfg_channel_id is not None and str(cfg_channel_id).lstrip('-').isdigit():
                channel_id_for_callback = int(cfg_channel_id)
        except Exception:
            pass
        
        results = []
        for user_id in user_ids:
            try:
                user_id = int(user_id)
                if user_id <= 0:
                    logger.info(f"跳过非用户主体 user_id={user_id}")
                    results.append({
                        'user_id': user_id,
                        'status': 'skipped',
                        'reason': '非用户主体无需提醒'
                    })
                    continue

                logger.info(f"处理用户 {user_id}...")
                
                # 从数据库获取用户信息（包括用户名）
                user_info = await database.get_group_member(group_id, user_id)
                username = None
                if user_info:
                    username = user_info.get('username')
                    logger.info(f"用户 {user_id} 的用户名: {username}")
                else:
                    logger.warning(f"未找到用户 {user_id} 的信息")
                
                # 构建提醒消息
                mention = f"@{username}" if username else f"<a href=\"tg://user?id={user_id}\">用户</a>"
                
                text = f"""🔔 {mention}，请关注我们的频道以获得最新信息！

📢 频道链接: {normalized_link}
🤖 或点击下方按钮: 回补订阅解开禁言

⏰ 此消息将在12小时后自动删除"""
                
                # 构建按钮 - 支持多种链接格式
                buttons = [
                    [InlineKeyboardButton("📢 直接关注", url=get_button_url(normalized_link))],
                    [InlineKeyboardButton("✅ 回补订阅解开禁言", callback_data=f"subscribe_{group_id}_{channel_id_for_callback}")]
                ]
                keyboard = InlineKeyboardMarkup(buttons)
                
                # 直接发送消息
                logger.info(f"发送消息给用户 {username or user_id}...")
                msg = await _bot_instance.send_message(
                    chat_id=group_id,
                    text=text,
                    parse_mode='HTML',
                    reply_markup=keyboard
                )
                
                message_id = msg.message_id
                logger.info(f"✅ 消息发送成功，message_id={message_id}")
                
                # 保存提醒记录
                expires_at = (datetime.now() + timedelta(hours=12)).isoformat()
                reminder_id = await database.save_channel_reminder(
                    group_id, user_id, message_id, expires_at
                )
                
                logger.info(f"提醒记录已保存，reminder_id={reminder_id}")
                
                results.append({
                    'user_id': user_id,
                    'username': username,
                    'message_id': message_id,
                    'reminder_id': reminder_id,
                    'sent_at': datetime.now().isoformat()
                })
                
                logger.info(f"✅ 已发送提醒给用户 {username or user_id}")
            except Exception as e:
                logger.error(f"❌ 发送提醒给用户 {user_id} 失败: {e}", exc_info=True)
                results.append({
                    'user_id': user_id,
                    'error': str(e)
                })
        
        sent_count = len([r for r in results if 'message_id' in r])
        logger.info(f"本次发送完成: 成功 {sent_count}/{len(user_ids)}")
        
        return web.json_response({
            'status': 'ok',
            'sent_count': sent_count,
            'results': results
        })
    except Exception as e:
        logger.error(f"❌ 发送频道提醒失败: {e}", exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)


def normalize_channel_link(link: str) -> str:
    """
    规范化频道链接格式
    支持: @xxx, https://t.me/xxx, t.me/xxx, /xxx 等格式
    返回: @xxx 或 https://t.me/xxx 格式
    """
    link = link.strip()
    
    # 如果已经是 @xxx 格式，直接返回
    if link.startswith('@'):
        return link
    
    # 如果是 https://t.me/xxx 格式，直接返回
    if link.startswith('https://t.me/'):
        return link
    
    # 如果是 t.me/xxx 格式，转换为 https://t.me/xxx
    if link.startswith('t.me/'):
        return f"https://{link}"
    
    # 如果是 /xxx 格式，转换为 @xxx
    if link.startswith('/'):
        return f"@{link[1:]}"
    
    # 如果是 xxx 格式（没有前缀），转换为 @xxx
    if not link.startswith('http'):
        return f"@{link}"
    
    return link


def get_button_url(link: str) -> str:
    """
    获取按钮 URL
    Telegram 按钮只支持 http/https URL，不支持 @xxx 格式
    """
    link = link.strip()
    
    # 如果是 @xxx 格式，转换为 https://t.me/xxx
    if link.startswith('@'):
        return f"https://t.me/{link[1:]}"
    
    # 如果已经是 https://t.me/xxx 格式，直接返回
    if link.startswith('https://t.me/'):
        return link
    
    # 如果是 t.me/xxx 格式，转换为 https://t.me/xxx
    if link.startswith('t.me/'):
        return f"https://{link}"
    
    # 其他情况，假设是频道名称，转换为 https://t.me/xxx
    return f"https://t.me/{link}"


async def delete_reminder_message_handler(request):
    """POST /api/group_listen/delete_reminder_message - 删除提醒消息"""
    try:
        data = await request.json()
        group_id = data.get('group_id')
        message_id = data.get('message_id')
        reminder_id = data.get('reminder_id')
        
        if not group_id or not message_id:
            return web.json_response({'status': 'error', 'message': '缺少必要参数'}, status=400)
        
        logger.info(f"删除提醒消息: group_id={group_id}, message_id={message_id}")
        
        if not _bot_instance:
            return web.json_response({'status': 'error', 'message': '机器人未初始化'}, status=500)
        
        try:
            await _bot_instance.delete_message(chat_id=group_id, message_id=message_id)
            
            if reminder_id:
                await database.mark_reminder_deleted(reminder_id)
            
            logger.info(f"已删除提醒消息 {message_id}")
            
            return web.json_response({
                'status': 'ok',
                'message': '消息已删除'
            })
        except Exception as e:
            logger.error(f"删除消息失败: {e}")
            return web.json_response({'status': 'error', 'message': str(e)}, status=500)
    except Exception as e:
        logger.error(f"删除提醒消息失败: {e}", exc_info=True)
        return web.json_response({'status': 'error', 'message': str(e)}, status=500)

# ============ 财务 ERP API 路由注册 ============

async def register_finance_routes(app):
    """注册财务 ERP API 路由"""
    try:
        from finance_api import init_finance_routes
        await init_finance_routes(app)
        logger.info("✅ 财务 ERP API 路由已注册")
    except Exception as e:
        logger.error(f"❌ 财务 ERP API 路由注册失败: {e}")
