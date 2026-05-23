# -*- coding: utf-8 -*-
"""
群组监听扩展 HTTP 端点：已验证/未验证用户列表、频道关注提醒、删除提醒消息。

从 ``web_admin`` 拆出以降低单文件体量；Bot 实例仍通过 ``web_admin._bot_instance`` 获取（与主进程一致）。
"""
from __future__ import annotations

import logging
from datetime import datetime, timedelta

from aiohttp import web
from telegram import InlineKeyboardButton, InlineKeyboardMarkup

import database
from backend.app.utils.channel_utils import get_button_url, normalize_channel_link

logger = logging.getLogger(__name__)


def _get_bot():
    import web_admin

    return web_admin._bot_instance


async def get_verified_users_handler(request):
    """GET /api/group_listen/verified_users - 获取已验证用户列表"""
    try:
        group_id = request.rel_url.query.get('group_id')
        period = request.rel_url.query.get('period', 'all')

        logger.info(f"获取已验证用户: group_id={group_id}, period={period}")

        if period not in ('today', 'all'):
            period = 'all'

        if group_id:
            group_id = int(group_id)
            users = await database.get_verified_users(group_id, period)
            logger.info(f"特定群组查询: group_id={group_id}, 返回 {len(users)} 个用户")
        else:
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

        try:
            normalized_link = normalize_channel_link(channel_link)
        except ValueError as err:
            return web.json_response({'status': 'error', 'message': str(err)}, status=400)

        logger.info(
            f"发送频道提醒: group_id={group_id}, user_count={len(user_ids)}, "
            f"channel_link={channel_link} -> {normalized_link}"
        )

        bot = _get_bot()
        if not bot:
            return web.json_response({'status': 'error', 'message': '机器人未初始化'}, status=500)

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

                user_info = await database.get_group_member(group_id, user_id)
                username = None
                if user_info:
                    username = user_info.get('username')
                    logger.info(f"用户 {user_id} 的用户名: {username}")
                else:
                    logger.warning(f"未找到用户 {user_id} 的信息")

                mention = f"@{username}" if username else f"<a href=\"tg://user?id={user_id}\">用户</a>"

                text = f"""🔔 {mention}，请关注我们的频道以获得最新信息！

📢 频道链接: {normalized_link}
🤖 或点击下方按钮: 回补订阅解开禁言

⏰ 此消息将在12小时后自动删除"""

                buttons = [
                    [InlineKeyboardButton("📢 直接关注", url=get_button_url(normalized_link))],
                    [InlineKeyboardButton("✅ 回补订阅解开禁言", callback_data=f"subscribe_{group_id}_{channel_id_for_callback}")]
                ]
                keyboard = InlineKeyboardMarkup(buttons)

                logger.info(f"发送消息给用户 {username or user_id}...")
                msg = await bot.send_message(
                    chat_id=group_id,
                    text=text,
                    parse_mode='HTML',
                    reply_markup=keyboard
                )

                message_id = msg.message_id
                logger.info(f"✅ 消息发送成功，message_id={message_id}")

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

        bot = _get_bot()
        if not bot:
            return web.json_response({'status': 'error', 'message': '机器人未初始化'}, status=500)

        try:
            await bot.delete_message(chat_id=group_id, message_id=message_id)

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
