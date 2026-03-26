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
from datetime import datetime, timedelta
import database
import config
import weakref
import re
import aiosqlite
import time
from html import escape
from typing import Optional, Dict

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
_tags_cache = {"data": [], "expire_at": 0}
# 成员头像 file_id 缓存（避免频繁打 Telegram API）
_member_avatar_cache: Dict[int, Dict[str, object]] = {}

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


async def _send_broadcast_to_chats(chat_ids, content, image_file=None, do_translate=False, retries=1):
    """面向群/频道执行广播，返回成功失败汇总"""
    if not _bot_instance:
        return {"total": 0, "success": 0, "failed": 0, "success_ids": [], "failed_items": []}

    from services.security_center import security_check_flow
    is_passed, verified_text, error_msg, _, _ = await security_check_flow(
        content or '', image_bytes=image_file, do_translate=do_translate
    )
    if not is_passed:
        raise ValueError(error_msg or '安全校验失败')

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
                    await _bot_instance.send_photo(chat_id=chat_id, photo=bio, caption=verified_text or None)
                else:
                    await _bot_instance.send_message(chat_id=chat_id, text=verified_text)
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
    2026 规格：Bearer Token 校验中间件。
    """
    # 1. 静态资源与首页免登 (支持 Token 自动注入)
    if request.path in ['/', '/ws', '/api/login'] or request.path.startswith('/api/file/'):
        return await handler(request)
        
    # 2. 检查 Token
    auth_header = request.headers.get('Authorization')
    cookie_token = request.cookies.get('admin_token')
    
    token = None
    if auth_header and auth_header.startswith('Bearer '):
        token = auth_header.split(' ')[1]
    elif cookie_token:
        token = cookie_token
        
    # 3. 校验逻辑
    if token == config.WEB_ADMIN_TOKEN:
        return await handler(request)
        
    logger.warning(f"拒绝未授权访问: {request.path} 来自 {request.remote}")
    return web.json_response({"status": "error", "message": "Unauthorized"}, status=401)


async def api_login(request):
    """Token 登录校验接口"""
    try:
        data = await request.json()
        token = (data.get('token') or '').strip()
        if token == config.WEB_ADMIN_TOKEN:
            return web.json_response({"status": "ok"})
        return web.json_response({"status": "error", "message": "Token 错误"}, status=401)
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=400)


async def index_handler(request):
    """返回管理员前端页面 (支持免登 Token)"""
    token = request.query.get('token')
    if token == config.WEB_ADMIN_TOKEN:
        # 如果带了正确 Token 访问首页，将其存入 Cookie 以便后续请求
        response = web.Response(text="", status=302, headers={'Location': '/'})
        response.set_cookie('admin_token', token, max_age=86400 * 7, httponly=True)
        return response
        
    html_path = os.path.join(os.path.dirname(__file__), 'admin_new.html')
    try:
        with open(html_path, 'r', encoding='utf-8') as f:
            return web.Response(text=f.read(), content_type='text/html')
    except Exception as e:
        logger.error(f"加载 admin_new.html 失败: {e}")
        # 降级方案
        old_path = os.path.join(os.path.dirname(__file__), 'admin.html')
        if os.path.exists(old_path):
            with open(old_path, 'r', encoding='utf-8') as f:
                return web.Response(text=f.read(), content_type='text/html')
        return web.Response(text=f"Error loading admin UI: {e}", status=500)

async def api_get_users(request):
    """获取来访用户列表"""
    try:
        users = await database.get_inbox_list()
        return web.json_response({"status": "ok", "data": users})
    except Exception as e:
        logger.error(f"api_get_users 错误: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def api_get_messages(request):
    """获取与某用户的聊天历史"""
    try:
        user_id = int(request.match_info['user_id'])
        page = int(request.query.get('page', 1))
        limit = 50
        offset = (page - 1) * limit
        
        messages = await database.get_chat_history(user_id, limit, offset)
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
                elif part.name == 'image': image_file = await part.read()
                    
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
    """导出链上流水 Excel（可按地址/多个地址导出）"""
    try:
        period = (request.query.get('period') or 'month').strip().lower()
        if period not in ('day', 'week', 'month'):
            period = 'month'

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
        _tags_cache = {"data": [], "expire_at": 0}

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
        limit = min(int(request.query.get('limit', 50)), 500)
        offset = int(request.query.get('offset', 0))
        keyword = request.query.get('keyword')
        tag = request.query.get('tag')
        group_id = request.query.get('group_id')
        if group_id: group_id = int(group_id)
        
        result = await database.get_potential_users(limit=limit, offset=offset, keyword=keyword, tag=tag, group_id=group_id)
        return web.json_response({"status": "ok", "data": result['users'], "total": result['total']})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_get_potential_tags(request):
    """获取所有潜在用户画像标签 (60s 缓存)"""
    global _tags_cache
    try:
        now = time.time()
        if now > _tags_cache["expire_at"]:
            tags = await database.get_potential_tags()
            _tags_cache = {"data": tags, "expire_at": now + 60}
        return web.json_response({"status": "ok", "data": _tags_cache["data"], "total": len(_tags_cache["data"])})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_list_groups(request):
    """获取所有追踪的群组列表 (分页 2026 规格，含实时状态)"""
    try:
        page = int(request.query.get('page', 1)); size = int(request.query.get('size', 20))
        result = await database.get_tracked_groups_paginated(page, size)
        
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
        data = await request.json()
        group_id = int(data.get('group_id')); listen_status = bool(data.get('listen_status'))
        await database.set_group_listen_status(group_id, listen_status)
        from handlers.group_monitor import clear_group_cache
        clear_group_cache(group_id)
        await notify_web_clients("group_listen_update", {"group_id": group_id, "new_status": listen_status})
        return web.json_response({"code": 200, "msg": "状态已更新", "new_status": listen_status})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def api_toggle_group_revoke_sync(request):
    """切换群组信息校对（撤回同步删除）开关"""
    try:
        data = await request.json()
        group_id = int(data.get('group_id'))
        enabled = bool(data.get('enabled'))
        await database.set_group_delete_on_revoke(group_id, enabled)
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
        user_id = int(request.match_info['user_id'])
        await database.mark_messages_as_read(user_id)
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def api_delete_chat(request):
    """软删除会话"""
    try:
        user_id = int(request.match_info['user_id'])
        await database.soft_delete_chat(user_id)
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
        msg_db_id = await database.save_message(from_userid=config.ADMIN_ID, to_userid=user_id, content="[图片]", msg_type='photo', is_read=True, username="Admin", status='sending')
        asyncio.create_task(_execute_web_send_task(user_id, msg_db_id, "[图片]", "[图片]", file_data, False, None, "text", None, None, "", 0))
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
        group_id = request.query.get('group_id')
        limit = int(request.query.get('limit', 50))
        offset = int(request.query.get('offset', 0))
        messages = await database.get_group_messages(int(group_id), limit, offset)
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
        return web.json_response({"status": "ok"})
    except Exception as e:
        return web.json_response({"status": "error", "message": str(e)}, status=500)


async def start_web_server(bot):
    """启动 Web Server"""
    global _push_scheduler_task, _push_scheduler_started
    set_bot_instance(bot)
    app = web.Application(middlewares=[auth_middleware])
    app.router.add_get('/', index_handler)
    app.router.add_post('/api/login', api_login)
    app.router.add_get('/ws', websocket_handler)
    app.router.add_get('/api/users', api_get_users)
    app.router.add_get('/api/messages/{user_id}', api_get_messages)
    app.router.add_post('/api/messages/{user_id}', api_send_message)
    app.router.add_post('/api/broadcast', api_broadcast_message)
    app.router.add_get('/api/broadcast/targets', api_get_broadcast_targets)
    app.router.add_post('/api/broadcast/targets/sync', api_sync_broadcast_targets)
    app.router.add_post('/api/broadcast/chats', api_broadcast_to_chats)
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
