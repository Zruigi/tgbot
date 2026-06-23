# -*- coding: utf-8 -*-
"""
双向客服系统逻辑
"""
import logging
import uuid
import asyncio
import html
import os
from datetime import datetime, timedelta
from html import escape
from typing import Optional, Union, Dict, Any
from telegram import Update, Message, Bot, InlineKeyboardButton, InlineKeyboardMarkup
from telegram.constants import ParseMode
from telegram.ext import ContextTypes, ConversationHandler, CommandHandler, MessageHandler, CallbackQueryHandler, filters
from telegram.error import RetryAfter, TimedOut, NetworkError
import config
import database
from handlers.customer import (
    extract_target_user_info,
    classify_message_content,
    extract_quoted_message_info,
    prepare_translation_feedback,
    build_admin_feedback,
)
from services import translator
from langdetect import detect, LangDetectException
from services.security_shield import contains_address
from services.security_center import security_check_flow
from services.media_processor import MediaProcessor

logger = logging.getLogger(__name__)

# 待补发消息过期时间（分钟）
PENDING_SEND_EXPIRE_MINUTES = 30

# 初始化媒体处理器 (单例模式)
_media_processor: Optional[MediaProcessor] = None

def get_media_processor(bot: Bot) -> MediaProcessor:
    global _media_processor
    if not _media_processor:
        _media_processor = MediaProcessor(bot)
    return _media_processor

async def safe_send_media(context, chat_id, media_type, file_id=None, local_path=None, thumb_path=None, caption=None, parse_mode="HTML", max_retries=3, reply_to_message_id=None):
    """
    2026 规格：全媒体类型安全发送函数
    支持：photo, sticker, animation, video, video_note, voice, audio, document
    """
    import web_admin
    bot = context.bot if context else web_admin._bot_instance
    if not bot:
        raise Exception("No bot instance available")

    # 1. 自动注入重试逻辑
    for attempt in range(max_retries):
        try:
            # 准备媒体数据 (优先使用本地路径，以便发送转码后的内容)
            media = local_path if local_path and os.path.exists(local_path) else file_id
            if not media:
                raise ValueError("No media content provided (file_id or local_path)")

            # 根据类型分发
            if media_type == 'photo':
                return await bot.send_photo(chat_id=chat_id, photo=media, caption=caption, parse_mode=parse_mode, reply_to_message_id=reply_to_message_id)
            elif media_type == 'sticker':
                return await bot.send_sticker(chat_id=chat_id, sticker=media, reply_to_message_id=reply_to_message_id)
            elif media_type == 'animation':
                return await bot.send_animation(chat_id=chat_id, animation=media, caption=caption, parse_mode=parse_mode, reply_to_message_id=reply_to_message_id)
            elif media_type == 'video':
                return await bot.send_video(chat_id=chat_id, video=media, caption=caption, parse_mode=parse_mode, reply_to_message_id=reply_to_message_id, thumbnail=thumb_path)
            elif media_type == 'video_note':
                return await bot.send_video_note(chat_id=chat_id, video_note=media, reply_to_message_id=reply_to_message_id, thumbnail=thumb_path)
            elif media_type == 'voice':
                return await bot.send_voice(chat_id=chat_id, voice=media, caption=caption, parse_mode=parse_mode, reply_to_message_id=reply_to_message_id)
            elif media_type == 'audio':
                return await bot.send_audio(chat_id=chat_id, audio=media, caption=caption, parse_mode=parse_mode, reply_to_message_id=reply_to_message_id, thumbnail=thumb_path)
            elif media_type == 'document':
                return await bot.send_document(chat_id=chat_id, document=media, caption=caption, parse_mode=parse_mode, reply_to_message_id=reply_to_message_id, thumbnail=thumb_path)
            else:
                # 兜底发文字
                return await bot.send_message(chat_id=chat_id, text=f"{caption or '[媒体消息]'}\n(不支持的消息类型: {media_type})", parse_mode=parse_mode, reply_to_message_id=reply_to_message_id)

        except RetryAfter as e:
            wait_time = float(getattr(e, 'retry_after', 1))
            logger.warning(f"Media Send Retry: {e}, waiting {wait_time}s (Attempt {attempt+1}/{max_retries})")
            await asyncio.sleep(wait_time)
        except (TimedOut, NetworkError) as e:
            wait_time = float(2 ** attempt)
            logger.warning(f"Media Send Retry: {e}, waiting {wait_time}s (Attempt {attempt+1}/{max_retries})")
            await asyncio.sleep(wait_time)
        except Exception as e:
            if "Reply message not found" in str(e):
                reply_to_message_id = None # 忽略无效引用
                continue
            logger.error(f"Media Send Failed ({media_type}): {e}")
            if attempt == max_retries - 1:
                raise

    raise Exception("超过最大重试次数")

def is_private_admin_payload(message: Message) -> bool:
    """隐私拦截：识别管理员在配置菜单输入的敏感文本，避免进入待补发队列。"""
    text = (message.text or message.caption or "").strip()
    if not text:
        return False

    sensitive_prefixes = (
        "sk-", "AIza", "hf_", "ghp_", "xoxb-", "xoxp-", "Bearer ",
    )
    upper_text = text.upper()
    sensitive_keywords = (
        "API_KEY", "TOKEN", "SECRET", "LLM_API_KEY", "GEMINI_API_KEY", "DEEPSEEK_API_KEY",
    )

    return text.startswith(sensitive_prefixes) or any(k in upper_text for k in sensitive_keywords)


def is_pending_message_expired(pending_row: dict, expire_minutes: int = PENDING_SEND_EXPIRE_MINUTES) -> bool:
    ts = pending_row.get('timestamp') if pending_row else None
    if not ts:
        return False
    try:
        created_at = datetime.fromisoformat(str(ts).replace('Z', '+00:00').replace(' ', 'T'))
    except Exception:
        return False

    # SQLite CURRENT_TIMESTAMP 默认是 UTC；无 tzinfo 时按 UTC 处理，避免本地时区误判为已过期
    if created_at.tzinfo is None:
        return (datetime.utcnow() - created_at) > timedelta(minutes=expire_minutes)
    return (datetime.now(created_at.tzinfo) - created_at) > timedelta(minutes=expire_minutes)


def format_user_identity(user_id: Union[int, str], user_name: Optional[str] = None, tg_username: Optional[str] = None) -> str:
    """
    统一用户身份展示模板（简洁版）
    - 姓名
    - @用户名（无则显示 -）
    - user_id
    """
    display_name = user_name or f"User {user_id}"
    username_text = f"@{tg_username}" if tg_username else "-"
    return (
        f"用户: <a href=\"tg://user?id={user_id}\">{escape(str(display_name))}</a>\n"
        f"用户名: <code>{escape(username_text)}</code>\n"
        f"UserID: <code>{user_id}</code>"
    )


def format_admin_tag(admin_id: Union[int, str], admin_name: Optional[str], worker_no: str = "CS001") -> str:
    """
    格式化客服身份标签（简洁版）
    """
    display_name = admin_name or "Administrator"
    return f'客服 <a href="tg://user?id={admin_id}">{escape(str(display_name))}</a> ({worker_no})：'

async def safe_send_message(context, chat_id, text, parse_mode="HTML", max_retries=3, reply_to_message_id=None, reply_markup=None):
    """带指数退避重试机制的安全发送函数，并自动处理无效的引用 ID"""
    import web_admin
    bot = context.bot if context else web_admin._bot_instance
    if not bot:
        raise Exception("No bot instance available")
    
    # 2026 规格: 用户 ID 合法性校验
    if not chat_id or not str(chat_id).strip().replace('-', '').isdigit():
        logger.error(f"Invalid User ID: {chat_id}")
        return None

    for attempt in range(max_retries):
        try:
            return await bot.send_message(
                chat_id=chat_id,
                text=text,
                parse_mode=parse_mode,
                reply_to_message_id=reply_to_message_id,
                reply_markup=reply_markup
            )
        except RetryAfter as e:
            wait_time = float(getattr(e, 'retry_after', 1))
            logger.warning(f"触发限流，等待 {wait_time} 秒后重试 (Attempt {attempt+1}/{max_retries})")
            await asyncio.sleep(wait_time)
        except (TimedOut, NetworkError) as e:
            wait_time = float(2 ** attempt)
            logger.warning(f"网络错误: {e}, 等待 {wait_time} 秒后重试 (Attempt {attempt+1}/{max_retries})")
            await asyncio.sleep(wait_time)
        except Exception as e:
            error_msg = str(e)
            
            # 如果是引用消息不存在的错误，尝试去掉引用 ID 重新发送
            if "Reply message not found" in error_msg or "message to reply not found" in error_msg.lower():
                logger.warning(f"引用消息 {reply_to_message_id} 不存在，将尝试不带引用发送")
                try:
                    return await bot.send_message(chat_id=chat_id, text=text, parse_mode=parse_mode)
                except Exception as e2:
                    logger.error(f"去掉引用发送依然失败: {e2}")
                    raise
            
            # 2026 规格: 处理 ID 问题 (如用户已注销或 ID 变更)
            if "chat not found" in error_msg.lower() or "user not found" in error_msg.lower():
                logger.warning(f"ID {chat_id} 发送失败，尝试检索缓存映射或用户名回退...")
                # 尝试通过数据库查找该用户的最新 ID 或信息 (如果存在映射)
                # 此处为示意，实际可根据 database.get_user_by_username 等进行回退
                raise Exception(f"目标用户 ID {chat_id} 无效或用户已注销，无法送达。")

            logger.error(f"无法恢复的发送错误: {e}")
            raise
    raise Exception("超过最大重试次数")

async def safe_send_photo(context, chat_id, photo, caption=None, parse_mode="HTML", max_retries=3, reply_to_message_id=None, reply_markup=None):
    """带重试机制的安全发送图片函数"""
    import web_admin
    bot = context.bot if context else web_admin._bot_instance
    if not bot:
        raise Exception("No bot instance available")
        
    for attempt in range(max_retries):
        try:
            return await bot.send_photo(
                chat_id=chat_id,
                photo=photo,
                caption=caption,
                parse_mode=parse_mode,
                reply_to_message_id=reply_to_message_id,
                reply_markup=reply_markup,
                read_timeout=30,
                write_timeout=30,
                connect_timeout=30
            )
        except Exception as e:
            error_msg = str(e)
            if "Reply message not found" in error_msg or "message to reply not found" in error_msg.lower():
                logger.warning(f"引用消息 {reply_to_message_id} 不存在，将尝试不带引用发送图片")
                try:
                    return await bot.send_photo(
                        chat_id=chat_id,
                        photo=photo,
                        caption=caption,
                        parse_mode=parse_mode,
                        reply_markup=reply_markup
                    )
                except Exception as e2:
                    logger.error(f"去掉引用发送图片依然失败: {e2}")
                    raise
            if attempt < max_retries - 1:
                wait_time = 2 ** attempt
                await asyncio.sleep(wait_time)
                continue
            logger.error(f"安全发送图片失败: {e}")
            raise

async def cs_callback_handler(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """处理客服操作按钮的回调"""
    query = update.callback_query
    if not query: return
    
    data = query.data
    if not data: return
    
    user = update.effective_user
    if not user: return
    admin_id = user.id
    
    if int(admin_id) not in config.ADMIN_TG_IDS:
        await query.answer("权限不足", show_alert=True)
        return
    
    # 提取管理员实时 TG 姓名/用户名并同步数据库
    # 优先显示 First Name，这样最直观
    admin_tg_name = user.first_name or user.username or "Administrator"
    await database.update_admin_info(admin_id, admin_tg_name)
        
    parts = data.split(":")
    action = parts[0]
    target_user_id = int(parts[1]) if len(parts) > 1 and str(parts[1]).lstrip('-').isdigit() else None
    
    from telegram import InlineKeyboardButton, InlineKeyboardMarkup

    if action in {"select_other_user", "cancel_send", "user_page", "confirm_send_to"}:
        target_user_id = None

    if action == "mark_read":
        await database.mark_messages_as_read(target_user_id)
        await query.answer(f"✅ 已标记用户 {target_user_id} 为已读")
        # 修改原消息中的按钮，把“标记已读”去掉或变灰
        msg = query.message
        if msg and isinstance(msg, Message):
            if msg.caption:
                await query.edit_message_caption(caption=msg.caption + "\n(已标记已读)")
            elif msg.text:
                await query.edit_message_text(text=msg.text + "\n(已标记已读)")
        
    elif action == "quick_reply":
        keyboard = [
            [InlineKeyboardButton("👋 你好，请问有什么可以帮您？", callback_data=f"do_quick:{target_user_id}:1")],
            [InlineKeyboardButton("⏳ 请稍等，正在为您查询...", callback_data=f"do_quick:{target_user_id}:2")],
            [InlineKeyboardButton("✅ 好的，没问题。", callback_data=f"do_quick:{target_user_id}:3")],
            [InlineKeyboardButton("🔙 返回", callback_data=f"back_to_main:{target_user_id}")]
        ]
        await query.edit_message_reply_markup(reply_markup=InlineKeyboardMarkup(keyboard))
            
    elif action == "do_quick":
        choice = parts[2]
        replies = {
            "1": "你好，请问有什么可以帮您？",
            "2": "好的，请稍等，正在为您查询中，请保持关注。",
            "3": "好的，没问题。"
        }
        reply_text = replies.get(choice, "收到")
        
        try:
            # 执行常规发送逻辑
            # 2026 规格：注入身份标签
            # 强制优先使用本次交互获取的 admin_tg_name，而不是数据库旧值
            admin_info = await database.get_admin_info(admin_id)
            admin_display_name = admin_tg_name # 直接使用刚才从 User 对象提取的实名
            admin_worker_no = admin_info['worker_no'] if admin_info else "CS001"
            admin_tag = format_admin_tag(admin_id, admin_display_name, admin_worker_no)

            sent_msg = await safe_send_message(
                context=context,
                chat_id=target_user_id,
                text=f"{admin_tag}\n{reply_text}",
                parse_mode="HTML"
            )
            # 存入数据库
            if sent_msg:
                await database.save_message(
                    from_userid=admin_id,
                    to_userid=target_user_id,
                    content=reply_text,
                    status='sent',
                    tg_msg_id=sent_msg.message_id
                )
            # 同步给 Web 端
            import web_admin
            await web_admin.notify_web_clients("new_message", {"user_id": target_user_id})
            
            await query.answer("✅ 快捷回复已发送")
            # 恢复主管理按钮
            await back_to_main(query, target_user_id)
        except Exception as e:
            await query.answer(f"❌ 发送失败: {e}", show_alert=True)

    elif action == "back_to_main":
        await back_to_main(query, target_user_id)
        
    elif action == "close_session":
        await database.soft_delete_chat(target_user_id)
        await query.answer(f"📂 已结束并归档与用户 {target_user_id} 的会话")
        msg = query.message
        if msg and isinstance(msg, Message):
            if msg.caption:
                await query.edit_message_caption(caption=msg.caption + "\n(会话已关闭)")
            elif msg.text:
                await query.edit_message_text(text=msg.text + "\n(会话已关闭)")

    elif action == "quick_send":
        # 2026 规格：基于 UUID 的批量/顺序处理与幂等性保障
        target_user_id = int(parts[1])
        msg_uuid = parts[2] if len(parts) > 2 else None
        
        if not msg_uuid:
            await query.answer("❌ 消息索引丢失", show_alert=True)
            return

        pending_msg_for_expire = await database.get_pending_admin_msg(msg_uuid)
        if not pending_msg_for_expire:
            await query.answer("⚠️ 消息已处理或不存在", show_alert=True)
            return
        if is_pending_message_expired(pending_msg_for_expire):
            await database.update_pending_admin_msg_status(msg_uuid, 'expired')
            await database.delete_pending_admin_msg(msg_uuid)
            await query.edit_message_text("⏰ 这条待发送消息已超过30分钟，默认失效。")
            return
            
        # 1. 核心路由算法：检索该管理员名下所有早于或等于当前 UUID 的【待处理】消息
        async with database.aiosqlite.connect(database.DB_NAME) as db:
            db.row_factory = database.aiosqlite.Row
            async with db.execute("SELECT timestamp FROM pending_admin_messages WHERE msg_uuid = ?", (msg_uuid,)) as cursor:
                current_msg = await cursor.fetchone()
                if not current_msg:
                    await query.answer("⚠️ 消息已处理或不存在", show_alert=True)
                    return
                current_ts = current_msg['timestamp']
            
            async with db.execute(
                "SELECT * FROM pending_admin_messages WHERE admin_id = ? AND status = 'pending' AND timestamp <= ? ORDER BY timestamp ASC",
                (admin_id, current_ts)
            ) as cursor:
                pending_batch = await cursor.fetchall()

        if not pending_batch:
            await query.answer("⚠️ 批次已完成或已失效", show_alert=True)
            await query.edit_message_text("✅ 所有待处理消息已发送完毕。")
            return
            
        sent_count = 0
        try:
            for pending in pending_batch:
                p_uuid = pending['msg_uuid']
                reply_text = pending['content']
                msg_type = pending['msg_type']
                file_id = pending['file_id']
                
                logger.info(f"⚖️ [Batch-Send] Admin confirming UUID:{p_uuid} in batch to {target_user_id}")
                
                admin_info = await database.get_admin_info(admin_id)
                admin_display_name = admin_tg_name
                admin_worker_no = admin_info['worker_no'] if admin_info else "CS001"
                admin_tag = format_admin_tag(admin_id, admin_display_name, admin_worker_no)

                sent_msg = None
                if msg_type == 'text':
                    sent_msg = await safe_send_message(
                        context=context, chat_id=target_user_id,
                        text=f"{admin_tag}\n{reply_text}", parse_mode="HTML"
                    )
                elif file_id:
                    caption = f"{admin_tag}\n" + (reply_text if reply_text else "")
                    processor = get_media_processor(context.bot)
                    success, processed_path, thumb_path, _ = await processor.process_media(file_id, msg_type)
                    
                    try:
                        if msg_type in ['sticker', 'video_note']:
                            sent_msg = await safe_send_media(
                                context, target_user_id, msg_type, 
                                file_id=file_id if not processed_path else None,
                                local_path=processed_path, thumb_path=thumb_path
                            )
                            await safe_send_message(context, target_user_id, admin_tag, reply_to_message_id=sent_msg.message_id)
                        else:
                            sent_msg = await safe_send_media(
                                context, target_user_id, msg_type,
                                file_id=file_id if not processed_path else None,
                                local_path=processed_path, thumb_path=thumb_path,
                                caption=caption
                            )
                    finally:
                        if processed_path: processor.cleanup(processed_path, thumb_path)
                else:
                    sent_msg = await safe_send_message(
                        context=context, chat_id=target_user_id,
                        text=f"{admin_tag}\n{reply_text or '[媒体消息]'}", parse_mode="HTML"
                    )
                
                if sent_msg:
                    await database.save_message(
                        from_userid=admin_id, to_userid=target_user_id,
                        content=reply_text or f"[{msg_type}]", msg_type=msg_type,
                        file_id=file_id, status='sent', tg_msg_id=sent_msg.message_id
                    )
                    await database.update_pending_admin_msg_status(p_uuid, 'sent')
                    sent_count += 1
            
            if sent_count > 0:
                import web_admin
                await web_admin.notify_web_clients("new_message", {"user_id": target_user_id})
                target_identity = format_user_identity(target_user_id)
                await query.edit_message_text(
                    f"✅ <b>成功补发 {sent_count} 条消息</b>\n{target_identity}", 
                    parse_mode="HTML"
                )
            else:
                await query.answer("❌ 发送失败，请检查 ID 是否有效", show_alert=True)
        except Exception as e:
            logger.error(f"Batch send failed: {e}")
            await query.answer(f"❌ 批次处理部分失败: {e}", show_alert=True)

    elif action == "batch_send":
        user_id = parts[1] if len(parts) > 1 else None
        if not user_id:
            await query.answer("No user ID", show_alert=True)
            return
        
        try:
            chat_history = await database.get_chat_history(int(user_id), limit=1)
            if not chat_history:
                await query.answer("No messages", show_alert=True)
                return
            
            last_msg = chat_history[0]
            msg_content = last_msg.get('content', '')
            msg_type = last_msg.get('msg_type', 'text')
            file_id = last_msg.get('file_id')
            
            all_users = await database.get_inbox_list()
            if not all_users:
                await query.answer("No users", show_alert=True)
                return
            
            keyboard = [
                [InlineKeyboardButton(f"Confirm send to {len(all_users)} users", callback_data=f"confirm_batch_send:{int(user_id)}:{msg_type}:{file_id or 'none'}")],
                [InlineKeyboardButton("Cancel", callback_data=f"back_to_main:{user_id}")]
            ]
            
            preview = (msg_content[:50] + "...") if len(msg_content) > 50 else msg_content
            await query.edit_message_text(
                f"Batch Send Confirm\n\nPreview: {preview}\nTarget: {len(all_users)} users",
                parse_mode="HTML",
                reply_markup=InlineKeyboardMarkup(keyboard)
            )
        except Exception as e:
            logger.error(f"Batch send prep failed: {e}")
            await query.answer(f"Error: {e}", show_alert=True)
    
    elif action == "confirm_batch_send":
        try:
            source_user_id = int(parts[1])
            msg_type = parts[2] if len(parts) > 2 else 'text'
            file_id = parts[3] if len(parts) > 3 and parts[3] != 'none' else None
            
            chat_history = await database.get_chat_history(source_user_id, limit=1)
            if not chat_history:
                await query.answer("Message deleted", show_alert=True)
                return
            
            source_msg = chat_history[0]
            content = source_msg.get('content', '')
            
            all_users = await database.get_inbox_list()
            target_users = [u['from_userid'] for u in all_users if u['from_userid'] != source_user_id]
            
            if not target_users:
                await query.answer("No other users", show_alert=True)
                return
            
            admin_info = await database.get_admin_info(admin_id)
            admin_display_name = admin_tg_name
            admin_worker_no = admin_info['worker_no'] if admin_info else "CS001"
            admin_tag = format_admin_tag(admin_id, admin_display_name, admin_worker_no)
            
            success_count = 0
            failed_count = 0
            
            for target_id in target_users:
                try:
                    if msg_type == 'text':
                        await safe_send_message(
                            context=context,
                            chat_id=target_id,
                            text=f"{admin_tag}\n{content}",
                            parse_mode="HTML"
                        )
                    else:
                        processor = get_media_processor(context.bot)
                        success, processed_path, thumb_path, _ = await processor.process_media(file_id, msg_type)
                        
                        caption_text = admin_tag
                        if content:
                            caption_text += f"\n{escape(content)}"
                        
                        if msg_type in ['sticker', 'video_note']:
                            await safe_send_media(
                                context, target_id, msg_type,
                                file_id=file_id if not processed_path else None,
                                local_path=processed_path, thumb_path=thumb_path
                            )
                        else:
                            await safe_send_media(
                                context, target_id, msg_type,
                                file_id=file_id if not processed_path else None,
                                local_path=processed_path, thumb_path=thumb_path,
                                caption=caption_text
                            )
                        
                        if processed_path:
                            processor.cleanup(processed_path, thumb_path)
                    
                    success_count += 1
                except Exception as e:
                    logger.warning(f"Send to {target_id} failed: {e}")
                    failed_count += 1
            
            await database.save_message(
                from_userid=admin_id,
                to_userid=0,
                content=f"[Batch] {content}",
                msg_type=msg_type,
                file_id=file_id,
                is_read=True,
                username="Admin",
                status='sent',
                admin_userid=str(config.ADMIN_ID)
            )
            
            await query.edit_message_text(
                f"Batch Send Complete\n\nSuccess: {success_count}\nFailed: {failed_count}",
                parse_mode="HTML"
            )
        except Exception as e:
            logger.error(f"Batch send failed: {e}")
            await query.answer(f"Error: {e}", show_alert=True)

    elif action == "select_other_user":
        msg_uuid = parts[1] if len(parts) > 1 else None
        if not msg_uuid:
            await query.answer("❌ 索引丢失", show_alert=True)
            return

        # 列出最近 5 个活跃用户
        users = await database.get_inbox_list()
        if not users:
            await query.answer("暂无活跃用户", show_alert=True)
            return
            
        keyboard = []
        for u in users[:5]:
            keyboard.append([InlineKeyboardButton(f"👤 {u['from_username']} (ID: {u['from_userid']})", callback_data=f"quick_send:{u['from_userid']}:{msg_uuid}")])
        keyboard.append([InlineKeyboardButton("❌ 取消", callback_data=f"cancel_send:{msg_uuid}")])
        
        await query.edit_message_text("请选择要补发的目标用户：", reply_markup=InlineKeyboardMarkup(keyboard))

    elif action == "cancel_send":
        msg_uuid = parts[1] if len(parts) > 1 else None
        if msg_uuid:
            await database.update_pending_admin_msg_status(msg_uuid, 'expired')
        await query.edit_message_text("🚫 已取消发送。提示：回复用户请尽量使用“左滑引用”。")

async def back_to_main(query, user_id):
    """恢复消息下的主管理按钮"""
    from telegram import InlineKeyboardButton, InlineKeyboardMarkup
    keyboard = [
        [
            InlineKeyboardButton("✅ 标记已读", callback_data=f"mark_read:{user_id}"),
            InlineKeyboardButton("🚀 快速回复", callback_data=f"quick_reply:{user_id}")
        ],
        [
            InlineKeyboardButton("👤 用户详情", callback_data=f"view_user:{user_id}"),
            InlineKeyboardButton("📂 结束会话", callback_data=f"close_session:{user_id}")
        ],
        [
            InlineKeyboardButton("📢 一键群发", callback_data=f"batch_send:{user_id}")
        ]
    ]
    await query.edit_message_reply_markup(reply_markup=InlineKeyboardMarkup(keyboard))

async def process_security_check(update: Update, context: ContextTypes.DEFAULT_TYPE, text: str, is_admin: bool = False, target_id: Optional[int] = None, do_translate: bool = False):
    """
    通用安全校验逻辑：拦截地址并处理双向通知
    返回: (is_passed, final_text, error_msg, is_ocr_verified, ocr_tag)
    """
    user_message = update.message
    if not user_message:
        return True, text, None, False, ""
    
    # 获取图片字节：相册图片 或 以文件形式发送的图片（常见为截图）
    image_bytes = None
    if user_message.photo:
        photo_file = await user_message.photo[-1].get_file()
        image_bytes = await photo_file.download_as_bytearray()
    elif user_message.document and user_message.document.mime_type and user_message.document.mime_type.startswith("image/"):
        if user_message.document.file_size and user_message.document.file_size > 20 * 1024 * 1024:
            logger.warning("跳过过大的图片文档 OCR: %s bytes", user_message.document.file_size)
        else:
            doc_file = await user_message.document.get_file()
            image_bytes = await doc_file.download_as_bytearray()
    
    # 如果是回复地址截图核验，需要取回之前暂存的地址
    pending_addr = (context.user_data or {}).get('pending_verification_addr')
    check_text = text
    if pending_addr and image_bytes:
        check_text = str(pending_addr)
        
    # 获取目标语言 (仅用于翻译)
    target_lang = 'en'
    if do_translate and is_admin:
        # 2026 修复：增加中-中对话语种一致性检查 (Smart Language Bypass)
        from services.translator import is_chinese
        
        # 判定管理员回复内容是否为中文
        admin_is_cn = is_chinese(text)
        
        # 获取被引用消息（客户原话）的语种
        customer_is_cn = False
        try:
            quoted = user_message.reply_to_message
            if quoted:
                q_text = quoted.text or quoted.caption or ""
                if q_text:
                    # 优先检测客户原话是否包含中文
                    customer_is_cn = is_chinese(q_text)
                    if not customer_is_cn:
                        target_lang = detect(q_text)
        except:
            target_lang = 'en'
            
        # 如果客户说中文且管理员回中文，则熔断翻译流程
        if admin_is_cn and customer_is_cn:
            logger.info(f"Smart Bypass: Both admin and customer use Chinese. Skipping translation for user {target_id}")
            do_translate = False

    # 调用全局统一安全流程
    security_check_flow_result = await security_check_flow(
        check_text, 
        image_bytes=image_bytes, 
        do_translate=do_translate,
        target_lang='zh-CN' if not is_admin else target_lang
    )
    is_passed, final_text, error_msg, is_ocr_verified, ocr_tag = security_check_flow_result
    
    if not is_passed:
        # --- 校验失败：执行拦截逻辑 ---
        # 1. 向发送方显示警告
        await user_message.reply_text(error_msg or "安全校验未通过", parse_mode="HTML")
        
        # 2. 通知接收方 (通知对方)
        receiver_id = target_id or (int(config.ADMIN_ID) if not is_admin else 0)
        if receiver_id:
            notice = f"⚠️ <b>拦截提示：</b>对方试图发送一个未经过安全校验的加密地址，已自动拦截，请提醒对方重新发送图片验证。{ocr_tag}"
            try:
                await safe_send_message(context, receiver_id, notice, parse_mode="HTML")
            except Exception as e:
                logger.warning(f"Failed to send intercept notice to {receiver_id}: {e}")
        
        # 3. 如果是因为缺图片被拦截，记录状态
        if error_msg and "配上高清截图" in error_msg and context.user_data is not None:
            context.user_data['pending_verification_addr'] = text
            
        return False, text, error_msg, is_ocr_verified, ocr_tag
        
    # 校验通过
    if pending_addr and image_bytes and context.user_data is not None:
        context.user_data.pop('pending_verification_addr', None)
        
    # 2026 规格：如果是给管理员看的消息，才附加 OCR 标签
    # 修复：process_security_check 现在返回 5 个值，解构时需要对应
    return True, final_text, error_msg, is_ocr_verified, ocr_tag

async def handle_user_message(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """全局拦截用户发送的所有消息 (带地址 + 图片强核验逻辑)"""
    user = update.effective_user
    user_message = update.message
    if not user or not user_message: return
    
    user_id = user.id
    admin_id = int(config.ADMIN_ID)
    
    # 忽略来自管理员的消息 (管理员由 handle_admin_reply 处理)
    if int(user_id) in config.ADMIN_TG_IDS:
        return
        
    original_text = user_message.text or user_message.caption or ""
    
    # --- 安全校验：地址防火墙 ---
    security_check_passed, verified_text, _, ocr_verified_flag, ocr_tag_str = await process_security_check(update, context, original_text, is_admin=False)
    if not security_check_passed:
        return
    
    # 更新经过校验后的文本（可能包含 OCR 通过标记）
    original_text = verified_text

    # 解析多媒体内容
    msg_type = 'text'
    file_id = None
    if user_message.photo:
        msg_type = 'photo'
        file_id = user_message.photo[-1].file_id
    elif user_message.sticker:
        msg_type = 'sticker'
        file_id = user_message.sticker.file_id
    elif user_message.animation:
        msg_type = 'animation'
        file_id = user_message.animation.file_id
    elif user_message.video:
        msg_type = 'video'
        file_id = user_message.video.file_id
    elif user_message.video_note:
        msg_type = 'video_note'
        file_id = user_message.video_note.file_id
    elif user_message.voice:
        msg_type = 'voice'
        file_id = user_message.voice.file_id
    elif user_message.audio:
        msg_type = 'audio'
        file_id = user_message.audio.file_id
    elif user_message.document:
        msg_type = 'document'
        file_id = user_message.document.file_id
    
    # 1. 调用翻译 (内部已集成地址保护)
    translated_text = None
    if original_text and not ocr_verified_flag: # 只有 OCR 未通过或不是纯地址截图时才尝试翻译
        try:
            # 2026 修复：如果文本和翻译后的文本完全一致（例如纯地址），则不存储 translated_text
            # 避免 Web 端出现重复显示的“译文”框
            potential_translation = await translator.translate_to_admin(original_text)
            
            # 只有当译文确实有意义（且不是简单的复读原文）时才赋值
            if potential_translation and potential_translation.strip() != original_text.strip():
                print(f"[DEBUG - 准备翻译] 正在为用户 {user_id} 调用翻译...")
                translated_text = potential_translation
                print(f"[DEBUG - 翻译成功] 获得译文: {translated_text}")
            else:
                logger.info(f"Skipping redundant translation for user {user_id} (Same as original or Pure address)")
        except Exception as e:
            print(f"[ERROR - 翻译失败] {e}")
            translated_text = f"⚠️ 翻译组件异常: {str(e)}"

    # 2. 拼接管理端展示文本（简洁版，统一身份字段）
    user_real_name = user.full_name or user.username or f"User {user_id}"
    user_tg_username = user.username
    identity_block = format_user_identity(user_id, user_real_name, user_tg_username)
    admin_header = f"新咨询\n{identity_block}\n\n"
    
    # --- 2026 规格：自动抓取用户头像 (Module 6) ---
    avatar_file_id = None
    try:
        # 尝试获取用户头像列表
        photos = await context.bot.get_user_profile_photos(user_id, limit=1)
        if photos and photos.total_count > 0:
            # 取最大尺寸的头像 file_id
            avatar_file_id = photos.photos[0][-1].file_id
            logger.info(f"✅ 已抓取用户 {user_id} 的头像: {avatar_file_id}")
    except Exception as e:
        logger.warning(f"无法抓取用户 {user_id} 头像: {e}")

    if translated_text:
        admin_text = (
            f"{admin_header}"
            f"原文: {escape(original_text)}\n"
            f"译文: {escape(translated_text)}"
        )
    else:
        admin_text = f"{admin_header}内容: {escape(original_text)}"

    # 3. 数据库持久化
    try:
        await database.save_message(
            from_userid=user_id,
            to_userid=admin_id,
            content=original_text,
            msg_type=msg_type,
            file_id=file_id,
            is_read=False,
            username=user_real_name,
            tg_msg_id=user_message.message_id,
            translated_text=translated_text,
            original_text=original_text if translated_text else None,
            avatar_file_id=avatar_file_id
        )
    except Exception as e:
        print(f"[ERROR - DB写入失败] {e}")

    # 4. 推送给管理员
    try:
        from telegram import InlineKeyboardButton, InlineKeyboardMarkup
        keyboard = [
            [InlineKeyboardButton("✅ 标记已读", callback_data=f"mark_read:{user_id}"),
             InlineKeyboardButton("🚀 快速回复", callback_data=f"quick_reply:{user_id}")],
            [InlineKeyboardButton("📂 结束会话", callback_data=f"close_session:{user_id}")]
        ]
        
        # 如果有 OCR 标签，附加在文本最后
        if ocr_tag_str:
            admin_text = f"{admin_text}\n{ocr_tag_str}"

        sent_msg = await safe_send_message(
            context=context,
            chat_id=admin_id,
            text=admin_text,
            parse_mode="HTML",
            reply_markup=InlineKeyboardMarkup(keyboard)
        )
        if sent_msg:
            await database.save_admin_mapping(sent_msg.message_id, user_id, user_message.message_id)
        
        if file_id:
            # 2026 规格：转发给管理员的消息也执行基本媒体处理 (主要用于 Web 端展示兼容)
            processor = get_media_processor(context.bot)
            success, processed_path, thumb_path, _ = await processor.process_media(file_id, msg_type)
            
            try:
                media_msg = await safe_send_media(
                    context, admin_id, msg_type,
                    file_id=file_id if not processed_path else None,
                    local_path=processed_path, thumb_path=thumb_path,
                    caption=(
                        f"来自: {escape(str(user_real_name))}\n"
                        f"用户名: {escape('@' + user_tg_username) if user_tg_username else '-'}\n"
                        f"UserID: {user_id}"
                    ) if msg_type not in ['sticker', 'video_note'] else None
                )
                if media_msg:
                    await database.save_admin_mapping(media_msg.message_id, user_id, user_message.message_id)
            finally:
                if processed_path: processor.cleanup(processed_path, thumb_path)
            
        import web_admin
        await web_admin.notify_web_clients("new_message", {"user_id": user_id})
    except Exception as e:
        print(f"[ERROR - 推送管理员失败] {e}")


async def handle_admin_reply(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """处理回复消息（仅限指定的管理员 对转发消息的回复）"""
    user = update.effective_user
    user_message = update.message
    if not user or not user_message:
        return
    
    user_id = user.id
    
    # 权限检查：仅限指定的管理员
    if int(user_id) not in config.ADMIN_TG_IDS:
        return

    # 检查是否是回复某条消息
    reply_to_message = user_message.reply_to_message
    if not reply_to_message:
        return

    target_user_id, original_tg_msg_id = await extract_target_user_info(reply_to_message)

    # 提取管理员实时 TG 姓名/用户名并同步数据库并直接用于回复
    admin_tg_name = user.first_name or user.username or "Administrator"
    await database.update_admin_info(user_id, admin_tg_name)

    if not target_user_id:
        if (user_message.text or "").lstrip().startswith("/"):
            return
        # 如果是群聊中引用的消息，静默忽略（不是Bot转发的通知消息）
        chat = user_message.chat
        if chat and chat.type in ('group', 'supergroup'):
            return
        await user_message.reply_text("❌ 无法识别目标用户 ID。请确保你回复的是机器人转发给你的通知或消息。", parse_mode="HTML")
        return

    original_text_input = user_message.text or user_message.caption or ""

    # 2026 修复：强制对管理员回复执行翻译 (即便全局开关关闭，只要是回复客户，也应优先翻译)
    is_passed, verified_text, _, _, _ = await process_security_check(
        update, context, original_text_input,
        is_admin=True, target_id=target_user_id,
        do_translate=True  # 强制开启翻译流程，内部会判断 target_lang
    )
    if not is_passed:
        return

    msg_type, content, file_id = classify_message_content(user_message, verified_text)

    trace_id = str(uuid.uuid4())
    
    try:
        quoted_info = extract_quoted_message_info(reply_to_message)
        quoted_display_text = quoted_info['display_text']
        quoted_type = quoted_info['type']
        quoted_file_id = quoted_info['file_id']

        final_send_content = verified_text
        translated_text, original_text = prepare_translation_feedback(original_text_input, verified_text)

        # 2026 规格：获取当前处理管理员的详细信息
        admin_info = await database.get_admin_info(user_id)
        admin_userid_str = str(user_id)
        admin_display_name = admin_tg_name # 强制实时实名覆盖数据库旧值
        admin_worker_no = admin_info['worker_no'] if admin_info else "CS001"

        msg_db_id = await database.save_message(
            from_userid=user_id,
            to_userid=target_user_id,
            content=final_send_content,
            msg_type=msg_type,
            file_id=file_id,
            is_read=True,
            username="Admin",
            quoted_text=quoted_display_text or (f"[{quoted_type}]" if quoted_type != 'text' else None),
            quoted_msg_id=original_tg_msg_id or reply_to_message.message_id,
            quoted_file_id=quoted_file_id,
            quoted_type=quoted_type,
            status='sending',
            trace_id=trace_id,
            translated_text=translated_text,
            original_text=original_text,
            admin_userid=admin_userid_str # 2026 规格：记录当前处理人
        )
        
        import web_admin
        await web_admin.notify_web_clients("new_message", {"user_id": target_user_id})
        
        # 发送给用户
        # 2026 规格：在消息头部注入管理员身份标签与工号信息
        # 优化文案：移除硬编码的“管理员回复”，改为更专业的引导文案
        admin_tag = format_admin_tag(user_id, admin_display_name, admin_worker_no)
        
        sent_msg = None
        reply_to_id = original_tg_msg_id
        
        if msg_type == 'text':
            sent_msg = await safe_send_message(
                context=context,
                chat_id=target_user_id,
                text=f"{admin_tag}\n{final_send_content}",
                parse_mode="HTML",
                reply_to_message_id=reply_to_id
            )
        else:
            # 2026 规格：针对富媒体消息执行下载、校验、转码处理
            processor = get_media_processor(context.bot)
            success, processed_path, thumb_path, error_msg = await processor.process_media(file_id, msg_type)
            
            if not success:
                logger.warning(f"Media processing failed, falling back to original file: {error_msg}")
                # 校验失败（如超过限制），向管理员反馈
                if "限制" in error_msg:
                    await user_message.reply_text(f"⚠️ <b>发送被拦截：</b>{error_msg}", parse_mode="HTML")
                    return
                # 其他处理错误，尝试直接转发原文件 (兜底)
                processed_path = None
                thumb_path = None

            try:
                # 构造带有身份标签的 Caption（对外仅发送最终内容：译文或已核验文本）
                caption_text = admin_tag
                if final_send_content and not str(final_send_content).startswith("["):
                    caption_text += f"\n{escape(final_send_content)}"

                # 特殊处理：Sticker 和 Video Note 不支持 Caption
                if msg_type in ['sticker', 'video_note']:
                    # 先发送媒体
                    sent_msg = await safe_send_media(
                        context, target_user_id, msg_type, 
                        file_id=file_id if not processed_path else None,
                        local_path=processed_path,
                        thumb_path=thumb_path,
                        reply_to_message_id=reply_to_id
                    )
                    # 再补发身份标签，防止客户收到“空白”信息（其实是只有媒体没有文字）
                    await safe_send_message(
                        context=context, chat_id=target_user_id, 
                        text=admin_tag, parse_mode="HTML", 
                        reply_to_message_id=sent_msg.message_id
                    )
                else:
                    # 其他媒体支持 Caption
                    sent_msg = await safe_send_media(
                        context, target_user_id, msg_type,
                        file_id=file_id if not processed_path else None,
                        local_path=processed_path,
                        thumb_path=thumb_path,
                        caption=caption_text,
                        reply_to_message_id=reply_to_id
                    )
            finally:
                # 清理临时文件
                if processed_path: processor.cleanup(processed_path, thumb_path)
        
        if sent_msg:
            await database.update_message_status(msg_db_id, 'sent', sent_msg.message_id)
        await web_admin.notify_web_clients("status_update", {"user_id": target_user_id})
        
        # 获取目标用户信息以显示在回执中
        target_identity = format_user_identity(target_user_id)

        admin_feedback = build_admin_feedback(
            target_identity,
            msg_type,
            verified_text,
            content,
            translated_text,
            original_text,
        )

        await user_message.reply_text(admin_feedback, parse_mode="HTML")
        logger.info(f"[TraceID: {trace_id}] 管理员回复用户 {target_user_id} 成功")

    except Exception as error:
        if 'msg_db_id' in locals():
            await database.update_message_status(msg_db_id, 'failed')
            import web_admin
            await web_admin.notify_web_clients("status_update", {"user_id": target_user_id})
        logger.error(f"[TraceID: {trace_id}] 回复失败: {error}")
        await user_message.reply_text(f"❌ 回复失败：{error}")

async def handle_admin_non_reply(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """处理管理员非回复消息：防止消息失踪并提供智能路由 (并发感知的自动/手动确认机制)"""
    user_message = update.message
    if not user_message: return
    effective_user = update.effective_user
    if not effective_user: return

    # 权限检查
    if int(effective_user.id) not in config.ADMIN_TG_IDS:
        return

    msg_text = user_message.text or user_message.caption or ""

    # 隐私功能：在机器人菜单录入 API/Token 时，不进入待补发流程，不给出“选择发送”按钮
    if is_private_admin_payload(user_message):
        await user_message.reply_text("🔒 已识别为隐私输入（API/Token），仅用于配置，不会推送给任何用户。")
        return
    
    # 2026 规格：全局唯一消息 ID 生成策略 (UUID v4 截断)
    import uuid
    msg_uuid = str(uuid.uuid4())[:8]
    # 正确识别消息类型（含 sticker 和 animation）
    if user_message.text:
        msg_type = 'text'
        file_id: Optional[str] = None
    elif user_message.photo:
        msg_type = 'photo'
        file_id = user_message.photo[-1].file_id
    elif user_message.sticker:
        msg_type = 'sticker'
        file_id = user_message.sticker.file_id
    elif user_message.animation:
        msg_type = 'animation'
        file_id = user_message.animation.file_id
    elif user_message.video:
        msg_type = 'video'
        file_id = user_message.video.file_id
    elif user_message.document:
        msg_type = 'document'
        file_id = user_message.document.file_id
    else:
        msg_type = 'other'
        file_id = None

    # 持久化存储待处理消息 (防丢失/跨重启)
    await database.save_pending_admin_msg(
        msg_uuid=msg_uuid,
        admin_id=admin_id,
        content=msg_text,
        msg_type=msg_type,
        file_id=file_id or ""
    )

    # 1. 核心并发检测算法：检查最近 5 分钟内的活跃咨询用户数
    concurrency_threshold = int(await database.get_admin_setting("concurrency_threshold", "2") or "2")
    active_count = await database.get_recent_active_count(minutes=5)
    is_high_concurrency = active_count >= concurrency_threshold

    # 获取最近活跃用户作为候选目标
    latest_user = await database.get_latest_active_user()
    
    # 统计该管理员当前所有待处理的消息数
    async with database.aiosqlite.connect(database.DB_NAME) as db:
        async with db.execute("SELECT COUNT(*) FROM pending_admin_messages WHERE admin_id = ? AND status = 'pending'", (admin_id,)) as cursor:
            row = await cursor.fetchone()
            total_pending = row[0] if row else 0

    if not latest_user:
        await user_message.reply_text(
            "❌ <b>无法直接发送：您未引用任何消息。</b>\n\n"
            "目前数据库中暂无活跃用户。请先在下方用户列表中找到目标消息并“左滑引用”进行回复。",
            parse_mode="HTML"
        )
        return

    target_id = latest_user['from_userid']
    target_name = latest_user['from_username']
    target_identity = format_user_identity(target_id, target_name)

    # 2. 智能决策流
    if not is_high_concurrency:
        # 并发量低 (≤阈值)：仍要求手动确认，但只显示一个确认按钮（不弹多选）
        # 2026 修复：禁用自动静默发送，防止内容为空时大量发送只有标签的空消息
        logger.info(f"⚖️ [Low-Concurrency Confirm] Routing to: {target_id}, msg_type={msg_type}")
        
        # 内容预览
        content_preview: str
        if msg_type == 'text':
            content_preview = f"📝 <i>{escape(msg_text[:100])}{'...' if len(msg_text) > 100 else ''}</i>"
        elif msg_type == 'photo':
            content_preview = f"🖼️ <i>[图片]{(' - ' + escape(msg_text[:60])) if msg_text else ''}</i>"
        elif msg_type == 'sticker':
            content_preview = "🎭 <i>[表情包]</i>"
        elif msg_type == 'animation':
            content_preview = "🎞️ <i>[动图]</i>"
        elif msg_type == 'video':
            content_preview = f"🎬 <i>[视频]{(' - ' + escape(msg_text[:60])) if msg_text else ''}</i>"
        elif msg_type == 'document':
            content_preview = f"📎 <i>[文件]{(' - ' + escape(msg_text[:60])) if msg_text else ''}</i>"
        else:
            content_preview = f"📦 <i>[{msg_type}]</i>"
        
        text_low = (
            f"💡 <b>请确认：将以下内容发送给</b>\n"
            f"{target_identity}\n\n"
            f"{content_preview}\n\n"
            f"⚠️ <b>提示：</b>建议左滑引用用户消息后再回复，以避免误发。\n⏰ 超过30分钟未确认会自动失效。"
        )
        
        from telegram import InlineKeyboardButton, InlineKeyboardMarkup
        keyboard_low = [
            [InlineKeyboardButton(f"✅ 确认发送给 {target_name}", callback_data=f"quick_send:{target_id}:{msg_uuid}")],
            [InlineKeyboardButton("🔍 选择其他用户", callback_data=f"select_other_user:{msg_uuid}")],
            [InlineKeyboardButton("❌ 取消", callback_data=f"cancel_send:{msg_uuid}")]
        ]
        await user_message.reply_text(text_low, parse_mode="HTML", reply_markup=InlineKeyboardMarkup(keyboard_low))
        return
        
    # 自动发送失败时的兜底（此分支现已不再使用，但保留以防未来需要）
    if False:
        pass

    # 并发量高 (>阈值) 或自动发送失败：弹出【手动确认】模式
    logger.info(f"⚖️ [Manual-Confirm] Concurrency high ({active_count}/{concurrency_threshold}) or error occurred.")
    
    text = (
        f"💡 <b>检测到您未引用对话。</b>\n\n"
        f"当前咨询并发量较高 ({active_count} 位活跃客户)，为防止误发，请手动确认接收方：\n"
        f"{target_identity}\n\n"
        f"📦 <b>待发送消息队列:</b> {total_pending} 条\n"
        f"📝 <b>当前内容预览:</b>\n<i>{escape(msg_text[:100])}{'...' if len(msg_text) > 100 else ''}</i>\n⏰ 超过30分钟未确认会自动失效。"
    )

    from telegram import InlineKeyboardButton, InlineKeyboardMarkup
    keyboard = [
        [InlineKeyboardButton(f"🚀 确认发送给 {target_name}", callback_data=f"quick_send:{target_id}:{msg_uuid}")],
        [InlineKeyboardButton("🔍 选择其他活跃用户", callback_data=f"select_other_user:{msg_uuid}")],
        [InlineKeyboardButton("❌ 取消发送", callback_data=f"cancel_send:{msg_uuid}")]
    ]
    
    await user_message.reply_text(text, parse_mode="HTML", reply_markup=InlineKeyboardMarkup(keyboard))

async def select_other_user_callback(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """处理"选择其他用户"回调 - 显示用户列表供管理员选择"""
    query = update.callback_query
    if not query:
        return
    
    user = query.from_user
    if not user or int(user.id) not in config.ADMIN_TG_IDS:
        await query.answer("❌ 仅管理员可使用此功能", show_alert=True)
        return
    
    await query.answer()
    
    # 提取 msg_uuid
    callback_data = query.data
    if not callback_data:
        return
        
    msg_uuid = callback_data.split(":", 1)[1] if ":" in callback_data else None
    
    if not msg_uuid:
        await query.edit_message_text("❌ 无效的请求")
        return

    pending_msg = await database.get_pending_admin_msg(msg_uuid)
    if not pending_msg:
        await query.edit_message_text("⚠️ 消息已处理或不存在")
        return
    if is_pending_message_expired(pending_msg):
        await database.update_pending_admin_msg_status(msg_uuid, 'expired')
        await database.delete_pending_admin_msg(msg_uuid)
        await query.edit_message_text("⏰ 这条待发送消息已超过30分钟，默认失效。")
        return
    
    # 获取所有活跃用户列表（最近 7 天有消息）
    try:
        active_users = await database.get_active_users_for_selection(days=7, limit=50)
    except Exception as e:
        logger.error(f"获取活跃用户列表失败: {e}")
        await query.edit_message_text(f"❌ 获取用户列表失败: {e}")
        return
    
    if not active_users:
        await query.edit_message_text("❌ 暂无活跃用户")
        return
    
    # 显示用户列表（分页，每页 5 个）
    page = 0
    if context.user_data is not None:
        context.user_data['select_user_page'] = page
        context.user_data['select_user_msg_uuid'] = msg_uuid
        context.user_data['select_user_list'] = active_users
        context.user_data['select_user_search'] = None
    
    await _show_user_selection_page(query, active_users, page, msg_uuid)


async def _show_user_selection_page(query, users: list, page: int, msg_uuid: str) -> None:
    """显示用户选择页面（分页）"""
    if not query:
        return
        
    page_size = 5
    total_pages = (len(users) + page_size - 1) // page_size
    
    if page < 0 or page >= total_pages:
        page = 0
    
    start_idx = page * page_size
    end_idx = start_idx + page_size
    page_users = users[start_idx:end_idx]
    
    # 构建用户列表文本
    text = "👥 <b>选择目标用户</b>\n\n"
    
    for i, user_info in enumerate(page_users, 1):
        user_id = user_info['from_userid']
        username = user_info['from_username'] or f"User {user_id}"
        last_active = user_info.get('last_msg_time', 'N/A')
        unread_count = user_info.get('unread_count', 0)
        
        # 格式化最后活跃时间
        if last_active and last_active != 'N/A':
            try:
                from datetime import datetime
                dt = datetime.fromisoformat(str(last_active))
                time_str = dt.strftime("%m-%d %H:%M")
            except:
                time_str = str(last_active)[:10]
        else:
            time_str = "N/A"
        
        unread_badge = f" 🔴 {unread_count}" if unread_count > 0 else ""
        text += f"{i}. <b>{escape(str(username))}</b> (ID: {user_id})\n"
        text += f"   ⏰ {time_str}{unread_badge}\n"
    
    text += f"\n📄 第 {page + 1}/{total_pages} 页"
    
    # 构建键盘
    keyboard = []
    for i, user_info in enumerate(page_users, 1):
        user_id = user_info['from_userid']
        username = user_info['from_username'] or f"User {user_id}"
        display_name = (str(username)[:15] + "...") if len(str(username)) > 15 else str(username)
        keyboard.append([
            InlineKeyboardButton(
                f"{i}. {display_name}",
                callback_data=f"confirm_send_to:{user_id}:{msg_uuid}"
            )
        ])
    
    # 分页按钮
    nav_buttons = []
    if page > 0:
        nav_buttons.append(InlineKeyboardButton("⬅️ 上一页", callback_data=f"user_page:{page-1}:{msg_uuid}"))
    if page < total_pages - 1:
        nav_buttons.append(InlineKeyboardButton("下一页 ➡️", callback_data=f"user_page:{page+1}:{msg_uuid}"))
    
    if nav_buttons:
        keyboard.append(nav_buttons)
    
    # 返回按钮
    keyboard.append([InlineKeyboardButton("❌ 取消", callback_data=f"cancel_send:{msg_uuid}")])
    
    await query.edit_message_text(
        text,
        parse_mode="HTML",
        reply_markup=InlineKeyboardMarkup(keyboard)
    )


async def user_page_callback(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """处理用户列表分页"""
    query = update.callback_query
    if not query:
        return
        
    user = query.from_user
    if not user or int(user.id) not in config.ADMIN_TG_IDS:
        await query.answer("❌ 仅管理员可使用此功能", show_alert=True)
        return
    
    await query.answer()
    
    # 解析回调数据
    callback_data = query.data
    if not callback_data:
        return
        
    parts = callback_data.split(":")
    page = int(parts[1])
    msg_uuid = parts[2] if len(parts) > 2 else None

    if msg_uuid:
        pending_msg = await database.get_pending_admin_msg(msg_uuid)
        if (not pending_msg) or is_pending_message_expired(pending_msg):
            if pending_msg:
                await database.update_pending_admin_msg_status(msg_uuid, 'expired')
                await database.delete_pending_admin_msg(msg_uuid)
            await query.edit_message_text("⏰ 这条待发送消息已超过30分钟，默认失效。")
            return
    
    # 获取缓存的用户列表
    users = context.user_data.get('select_user_list', []) if context.user_data else []
    
    if not users:
        await query.edit_message_text("❌ 用户列表已过期，请重新选择")
        return
    
    await _show_user_selection_page(query, users, page, msg_uuid or "")


async def confirm_send_to_callback(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """确认发送给选定的用户"""
    query = update.callback_query
    if not query:
        return
        
    user = query.from_user
    if not user or int(user.id) not in config.ADMIN_TG_IDS:
        await query.answer("❌ 仅管理员可使用此功能", show_alert=True)
        return
    
    # 解析回调数据
    callback_data = query.data
    if not callback_data:
        return
        
    parts = callback_data.split(":")
    target_user_id = int(parts[1])
    msg_uuid = parts[2] if len(parts) > 2 else None
    
    if not msg_uuid:
        await query.answer("❌ 无效的请求", show_alert=True)
        return
    
    # 获取待发送消息
    try:
        pending_msg = await database.get_pending_admin_msg(msg_uuid)
    except Exception as e:
        logger.error(f"获取待发送消息失败: {e}")
        await query.answer(f"❌ 获取消息失败: {e}", show_alert=True)
        return
    
    if not pending_msg:
        await query.answer("❌ 消息已过期或不存在", show_alert=True)
        return
    if is_pending_message_expired(pending_msg):
        await database.update_pending_admin_msg_status(msg_uuid, 'expired')
        await database.delete_pending_admin_msg(msg_uuid)
        await query.edit_message_text("⏰ 这条待发送消息已超过30分钟，默认失效。")
        return
    
    msg_text = pending_msg.get('content')
    msg_type = str(pending_msg.get('msg_type', 'text'))
    file_id = pending_msg.get('file_id')
    
    # 获取目标用户信息
    try:
        target_user_row = await database.get_user(target_user_id)
        target_name = target_user_row['from_username'] if target_user_row and 'from_username' in target_user_row else f"User {target_user_id}"
    except:
        target_name = f"User {target_user_id}"
    
    # 执行发送
    await query.answer()
    
    try:
        # 获取管理员信息
        admin_tg_name = user.first_name or user.username or "Administrator"
        await database.update_admin_info(user.id, admin_tg_name)
        
        admin_info = await database.get_admin_info(user.id)
        admin_display_name = admin_tg_name
        admin_worker_no = admin_info['worker_no'] if admin_info else "CS001"
        admin_tag = format_admin_tag(user.id, admin_display_name, admin_worker_no)
        
        # 保存消息到数据库
        msg_db_id = await database.save_message(
            from_userid=user.id,
            to_userid=target_user_id,
            content=msg_text,
            msg_type=msg_type,
            file_id=file_id,
            is_read=True,
            username="Admin",
            status='sending',
            admin_userid=str(user.id)
        )
        
        # 发送消息
        sent_msg = None
        if msg_type == 'text':
            sent_msg = await safe_send_message(
                context=context,
                chat_id=target_user_id,
                text=f"{admin_tag}\n{msg_text or ''}",
                parse_mode="HTML"
            )
        else:
            # 处理媒体消息
            processor = get_media_processor(context.bot)
            success, processed_path, thumb_path, error_msg = await processor.process_media(file_id or "", msg_type)
            
            if not success:
                logger.warning(f"Media processing failed: {error_msg}")
                processed_path = None
                thumb_path = None
            
            try:
                caption_text = admin_tag
                if msg_text:
                    caption_text += f"\n{escape(msg_text)}"
                
                if msg_type in ['sticker', 'video_note']:
                    sent_msg = await safe_send_media(
                        context, target_user_id, msg_type,
                        file_id=file_id if not processed_path else None,
                        local_path=processed_path, thumb_path=thumb_path
                    )
                else:
                    sent_msg = await safe_send_media(
                        context, target_user_id, msg_type,
                        file_id=file_id if not processed_path else None,
                        local_path=processed_path, thumb_path=thumb_path,
                        caption=caption_text
                    )
            finally:
                if processed_path:
                    processor.cleanup(processed_path, thumb_path)
        
        if sent_msg:
            await database.update_message_status(msg_db_id, 'sent', sent_msg.message_id)
            await query.edit_message_text(
                f"✅ <b>已成功发送给</b>\n"
                f"👤 <b>{escape(str(target_name))}</b> (ID: {target_user_id})",
                parse_mode="HTML"
            )
        else:
            await database.update_message_status(msg_db_id, 'failed')
            await query.edit_message_text(f"❌ 发送失败，请重试")
        
        # 清理待发送消息
        await database.delete_pending_admin_msg(msg_uuid)
        
        # 通知 Web 端
        import web_admin
        await web_admin.notify_web_clients("new_message", {"user_id": target_user_id})
        
    except Exception as e:
        logger.error(f"发送消息失败: {e}")
        await query.edit_message_text(f"❌ 发送失败: {e}")




async def admin_reply_command(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """管理员主动指定用户 ID 进行回复: /reply <user_id> <内容>"""
    effective_user = update.effective_user
    user_message = update.message
    if not effective_user or not user_message:
        return
    user_id = effective_user.id
    
    # 权限检查
    if int(user_id) not in config.ADMIN_TG_IDS:
        return

    if not context.args or len(context.args) < 2:
        await user_message.reply_text("❌ 使用方法：<code>/reply 用户ID 回复内容</code>", parse_mode="HTML")
        return

    try:
        target_user_id = int(context.args[0])
        reply_text = " ".join(context.args[1:])
        
        # 提取管理员实时 TG 姓名/用户名并同步数据库
        admin_tg_name = effective_user.first_name or effective_user.username or "Administrator"
        await database.update_admin_info(user_id, admin_tg_name)

        # 存入数据库
        msg_db_id = await database.save_message(
            from_userid=admin_id,
            to_userid=target_user_id,
            content=reply_text,
            msg_type='text',
            is_read=True,
            status='sending'
        )
        
        # 发送给指定用户
        # 2026 规格：注入身份标签
        admin_info = await database.get_admin_info(user_id)
        admin_display_name = admin_tg_name # 强制实时实名
        admin_worker_no = admin_info['worker_no'] if admin_info else "CS001"
        admin_tag = format_admin_tag(user_id, admin_display_name, admin_worker_no)

        sent_msg = await context.bot.send_message(
            chat_id=target_user_id,
            text=f"{admin_tag}\n{reply_text}",
            parse_mode="HTML"
        )
        await database.update_message_status(msg_db_id, 'sent', sent_msg.message_id)
        target_identity = format_user_identity(target_user_id)
        await user_message.reply_text(f"✅ 已成功发送给用户\n{target_identity}", parse_mode="HTML")
    except ValueError:
        await user_message.reply_text("❌ 无效的用户 ID。")
    except Exception as e:
        if 'msg_db_id' in locals():
            await database.update_message_status(msg_db_id, 'failed')
        logger.error(f"主动回复用户 {target_user_id} 失败: {e}")
        await user_message.reply_text(f"❌ 发送失败：{e}")
