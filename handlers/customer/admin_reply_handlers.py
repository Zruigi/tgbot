# -*- coding: utf-8 -*-
"""
管理员回复处理器 - 从 handle_admin_reply() 拆分出的小函数
每个函数负责单一职责，便于测试和维护
"""
import logging
import re
from typing import Optional, Dict, Any, Tuple
from telegram import Update, Message
from telegram.ext import ContextTypes
import database
import config

logger = logging.getLogger(__name__)


async def extract_target_user_info(reply_to_message: Message) -> Tuple[Optional[int], Optional[int]]:
    """从被回复的消息中提取目标用户 ID 和原始消息 ID"""
    target_user_id = await database.get_user_id_by_admin_msg(reply_to_message.message_id)
    original_tg_msg_id = await database.get_original_msg_id_by_admin_msg(reply_to_message.message_id)

    if not target_user_id:
        message_text = str(reply_to_message.text or reply_to_message.caption or "")
        match = re.search(r"(?:UserID|用户\s*ID|用户ID)\s*[:：]\s*(?:<code>)?(-?\d+)(?:</code>)?", message_text, re.IGNORECASE)
        if match:
            target_user_id = int(match.group(1))

    return target_user_id, original_tg_msg_id


async def process_security_and_translation(
    update: Update, 
    context: ContextTypes.DEFAULT_TYPE, 
    original_text: str,
    target_user_id: int
) -> Tuple[bool, str]:
    """
    执行安全校验与自动翻译
    返回：(is_passed, verified_text)
    """
    # 延迟导入，避免与 customer_service 的循环依赖
    from handlers.customer_service import process_security_check

    is_passed, verified_text, _, _, _ = await process_security_check(
        update, context, original_text,
        is_admin=True, target_id=target_user_id,
        do_translate=True  # 强制开启翻译流程
    )
    return is_passed, verified_text


def classify_message_content(user_message: Message, verified_text: str) -> Tuple[str, str, Optional[str]]:
    """
    分类消息内容并提取文件 ID
    返回：(msg_type, content, file_id)
    """
    if user_message.text:
        return 'text', verified_text, None
    
    if user_message.photo:
        return 'photo', verified_text or "[图片]", user_message.photo[-1].file_id
    
    if user_message.sticker:
        return 'sticker', f"[表情包: {user_message.sticker.emoji or ''}]", user_message.sticker.file_id
    
    if user_message.animation:
        return 'animation', verified_text or "[动图]", user_message.animation.file_id
    
    if user_message.video:
        return 'video', verified_text or "[视频]", user_message.video.file_id
    
    if user_message.video_note:
        return 'video_note', "[视频消息]", user_message.video_note.file_id
    
    if user_message.voice:
        return 'voice', verified_text or "[语音]", user_message.voice.file_id
    
    if user_message.audio:
        return 'audio', verified_text or "[音频]", user_message.audio.file_id
    
    if user_message.document:
        return 'document', verified_text or f"[文件: {user_message.document.file_name}]", user_message.document.file_id

    return 'other', "[不支持的媒体类型]", None


def extract_quoted_message_info(reply_to_message: Message) -> Dict[str, Any]:
    """从被引用的消息中提取摘要信息"""
    quoted_info = {
        'text': None,
        'file_id': None,
        'type': 'text',
        'display_text': None
    }
    
    # 提取文字内容
    if reply_to_message.text or reply_to_message.caption:
        full_text = str(reply_to_message.text or reply_to_message.caption)
        
        if "原文：" in full_text:
            quoted_text = full_text.split("原文：", 1)[1].split("----------------", 1)[0].strip()
        elif "内容：" in full_text:
            quoted_text = full_text.split("内容：", 1)[1].strip()
        else:
            quoted_text = full_text.strip()
        
        quoted_info['text'] = quoted_text
        quoted_info['display_text'] = (quoted_text[:47] + "...") if quoted_text and len(quoted_text) > 50 else quoted_text
    
    # 提取媒体文件 ID
    if reply_to_message.photo:
        quoted_info['type'] = 'photo'
        quoted_info['file_id'] = reply_to_message.photo[-1].file_id
    elif reply_to_message.sticker:
        quoted_info['type'] = 'sticker'
        quoted_info['file_id'] = reply_to_message.sticker.file_id
    elif reply_to_message.document:
        quoted_info['type'] = 'document'
        quoted_info['file_id'] = reply_to_message.document.file_id
    elif reply_to_message.video:
        quoted_info['type'] = 'video'
        quoted_info['file_id'] = reply_to_message.video.file_id
    
    return quoted_info


def prepare_translation_feedback(
    original_text: str, 
    verified_text: str
) -> Tuple[Optional[str], Optional[str]]:
    """
    准备翻译反馈信息
    返回：(translated_text, original_text) 或 (None, None)
    """
    is_translated = (verified_text.strip() != original_text.strip())
    
    if is_translated:
        return verified_text, original_text
    return None, None


def build_admin_feedback(
    target_identity: str,
    msg_type: str,
    verified_text: str,
    content: str,
    translated_text: Optional[str],
    original_text: Optional[str],
) -> str:
    """构建发送给管理员的反馈消息（与 customer_service 历史文案一致）"""
    from html import escape

    if translated_text:
        return (
            f"发送成功\n"
            f"{target_identity}\n"
            f"原文: {escape(str(original_text))}\n"
            f"译文: {escape(str(translated_text))}"
        )
    return (
        f"发送成功 [{msg_type}]\n"
        f"{target_identity}\n"
        f"内容: {escape(str(verified_text or content))}"
    )


def should_ignore_non_reply_in_group(chat_type: str) -> bool:
    """判断是否应该忽略群聊中的非回复消息"""
    return chat_type in ('group', 'supergroup')
