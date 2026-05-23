# -*- coding: utf-8 -*-
"""
客服回调处理器 - 从 cs_callback_handler() 拆分出的小函数
每个函数负责单一职责，便于测试和维护
"""
import logging
from typing import Optional, List, Dict, Any
from telegram import Update, InlineKeyboardButton, InlineKeyboardMarkup, Message
from telegram.ext import ContextTypes
import database
import config

logger = logging.getLogger(__name__)


async def handle_mark_read(query, target_user_id: int) -> None:
    """处理标记已读操作"""
    await database.mark_messages_as_read(target_user_id)
    await query.answer(f"✅ 已标记用户 {target_user_id} 为已读")
    
    # 修改原消息中的按钮
    msg = query.message
    if msg and isinstance(msg, Message):
        suffix = "\n(已标记已读)"
        if msg.caption:
            await query.edit_message_caption(caption=msg.caption + suffix)
        elif msg.text:
            await query.edit_message_text(text=msg.text + suffix)


async def handle_quick_reply_menu(query, target_user_id: Optional[int]) -> None:
    """显示快捷回复菜单"""
    keyboard = [
        [InlineKeyboardButton("👋 你好，请问有什么可以帮您？", callback_data=f"do_quick:{target_user_id}:1")],
        [InlineKeyboardButton("⏳ 请稍等，正在为您查询...", callback_data=f"do_quick:{target_user_id}:2")],
        [InlineKeyboardButton("✅ 好的，没问题。", callback_data=f"do_quick:{target_user_id}:3")],
        [InlineKeyboardButton("🔙 返回", callback_data=f"back_to_main:{target_user_id}")]
    ]
    await query.edit_message_reply_markup(reply_markup=InlineKeyboardMarkup(keyboard))


async def handle_do_quick_reply(context, query, target_user_id: int, choice: str, admin_tg_name: str) -> None:
    """执行快捷回复"""
    replies = {
        "1": "你好，请问有什么可以帮您？",
        "2": "好的，请稍等，正在为您查询中，请保持关注。",
        "3": "好的，没问题。"
    }
    reply_text = replies.get(choice, "收到")
    
    try:
        from .customer_service import safe_send_message, format_admin_tag
        
        # 获取管理员信息
        admin_info = await database.get_admin_info(config.ADMIN_ID)
        admin_display_name = admin_tg_name
        admin_worker_no = admin_info['worker_no'] if admin_info else "CS001"
        admin_tag = format_admin_tag(config.ADMIN_ID, admin_display_name, admin_worker_no)
        
        # 发送消息
        sent_msg = await safe_send_message(
            context=context,
            chat_id=target_user_id,
            text=f"{admin_tag}\n{reply_text}",
            parse_mode="HTML"
        )
        
        # 存入数据库
        if sent_msg:
            await database.save_message(
                from_userid=config.ADMIN_ID,
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
        from .customer_service import back_to_main
        await back_to_main(query, target_user_id)
    except Exception as e:
        await query.answer(f"❌ 发送失败：{e}", show_alert=True)


async def handle_close_session(query, target_user_id: int) -> None:
    """关闭会话"""
    await database.soft_delete_chat(target_user_id)
    await query.answer(f"📂 已结束并归档与用户 {target_user_id} 的会话")
    
    msg = query.message
    if msg and isinstance(msg, Message):
        suffix = "\n(会话已关闭)"
        if msg.caption:
            await query.edit_message_caption(caption=msg.caption + suffix)
        elif msg.text:
            await query.edit_message_text(text=msg.text + suffix)


async def handle_quick_send_confirm(context, query, target_user_id: int, msg_uuid: str, admin_tg_name: str) -> None:
    """处理快速发送确认（基于 UUID 的幂等性）"""
    from .customer_service import (
        is_pending_message_expired, 
        safe_send_message, 
        safe_send_media,
        get_media_processor,
        format_admin_tag
    )
    
    pending_msg = await database.get_pending_admin_msg(msg_uuid)
    if not pending_msg:
        await query.answer("⚠️ 消息已处理或不存在", show_alert=True)
        return
    
    if is_pending_message_expired(pending_msg):
        await database.update_pending_admin_msg_status(msg_uuid, 'expired')
        await database.delete_pending_admin_msg(msg_uuid)
        await query.edit_message_text("⏰ 这条待发送消息已超过 30 分钟，默认失效。")
        return
    
    # 检索该管理员名下所有早于或等于当前 UUID 的待处理消息
    async with database.aiosqlite.connect(database.DB_NAME) as db:
        db.row_factory = database.aiosqlite.Row
        async with db.execute(
            "SELECT timestamp FROM pending_admin_messages WHERE msg_uuid = ?", 
            (msg_uuid,)
        ) as cursor:
            current_msg = await cursor.fetchone()
            if not current_msg:
                await query.answer("⚠️ 消息已处理或不存在", show_alert=True)
                return
            current_ts = current_msg['timestamp']
        
        async with db.execute(
            """SELECT * FROM pending_admin_messages 
               WHERE admin_id = ? AND status = 'pending' AND timestamp <= ? 
               ORDER BY timestamp ASC""",
            (config.ADMIN_ID, current_ts)
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
            
            admin_info = await database.get_admin_info(config.ADMIN_ID)
            admin_display_name = admin_tg_name
            admin_worker_no = admin_info['worker_no'] if admin_info else "CS001"
            admin_tag = format_admin_tag(config.ADMIN_ID, admin_display_name, admin_worker_no)
            
            sent_msg = None
            if msg_type == 'text':
                sent_msg = await safe_send_message(
                    context=context, 
                    chat_id=target_user_id,
                    text=f"{admin_tag}\n{reply_text}", 
                    parse_mode="HTML"
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
                        await safe_send_message(
                            context, target_user_id, admin_tag, 
                            reply_to_message_id=sent_msg.message_id
                        )
                    else:
                        sent_msg = await safe_send_media(
                            context, target_user_id, msg_type,
                            file_id=file_id if not processed_path else None,
                            local_path=processed_path, thumb_path=thumb_path,
                            caption=caption
                        )
                finally:
                    if processed_path: 
                        processor.cleanup(processed_path, thumb_path)
            else:
                sent_msg = await safe_send_message(
                    context=context, 
                    chat_id=target_user_id,
                    text=f"{admin_tag}\n{reply_text or '[媒体消息]'}", 
                    parse_mode="HTML"
                )
            
            if sent_msg:
                await database.save_message(
                    from_userid=config.ADMIN_ID, 
                    to_userid=target_user_id,
                    content=reply_text or f"[{msg_type}]", 
                    msg_type=msg_type,
                    file_id=file_id, 
                    status='sent', 
                    tg_msg_id=sent_msg.message_id
                )
                await database.update_pending_admin_msg_status(p_uuid, 'sent')
                sent_count += 1
        
        if sent_count > 0:
            import web_admin
            await web_admin.notify_web_clients("new_message", {"user_id": target_user_id})
            await query.edit_message_text(
                f"✅ <b>成功补发 {sent_count} 条消息</b>", 
                parse_mode="HTML"
            )
        else:
            await query.answer("❌ 发送失败，请检查 ID 是否有效", show_alert=True)
    except Exception as e:
        logger.error(f"Batch send failed: {e}")
        await query.answer(f"❌ 批次处理部分失败：{e}", show_alert=True)
