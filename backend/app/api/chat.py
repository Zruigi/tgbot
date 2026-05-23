# -*- coding: utf-8 -*-
"""
私聊 API - 用户列表、聊天历史、消息发送
"""
from aiohttp import web
import logging
from typing import Optional, Dict, Any, List
from backend.repositories.user_repo import (
    get_inbox_list,
    get_inbox_list_for_tenant,
    get_chat_history,
    mark_messages_as_read,
    rebuild_inbox_from_messages,
    is_user_belong_to_tenant,
)
from backend.repositories.message_repo import (
    save_message,
    delete_single_message,
    search_messages,
)
from backend.repositories.audit_repo import log_tenant_isolation_event

logger = logging.getLogger(__name__)


async def api_get_users(request: web.Request) -> web.Response:
    """获取来访用户列表"""
    try:
        tenant_id = request.get('tenant_id', '')
        role = request.get('role', 'merchant_owner')
        merchant_id = request.get('merchant_id')
        
        if role == 'merchant_owner' and tenant_id and merchant_id:
            users = await get_inbox_list_for_tenant(tenant_id, merchant_id)
        else:
            users = await get_inbox_list()
        logger.info(f"获取用户列表：返回 {len(users)} 个用户")
        return web.json_response({
            'status': 'ok',
            'data': users
        })
    except Exception as e:
        logger.error(f"获取用户列表失败：{e}")
        import traceback
        logger.error(traceback.format_exc())
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_get_messages(request: web.Request) -> web.Response:
    """获取聊天历史"""
    try:
        user_id = int(request.match_info['user_id'])
        page = int(request.query.get('page', 1))
        limit = 50
        offset = (page - 1) * limit
        
        tenant_id = request.get('tenant_id', '')
        role = request.get('role', 'merchant_owner')
        merchant_id = request.get('merchant_id')
        
        if role == 'merchant_owner' and tenant_id and merchant_id:
            if not await is_user_belong_to_tenant(user_id, tenant_id, merchant_id):
                await log_tenant_isolation_event(
                    tenant_id=tenant_id,
                    action='cross_tenant_access_denied',
                    resource_type='message',
                    resource_id=str(user_id),
                    operator_role=role,
                    request_path=request.path,
                )
                return web.json_response(
                    {'status': 'error', 'message': '无权访问该用户消息'},
                    status=403
                )
        
        messages = await get_chat_history(user_id, limit=limit, offset=offset)
        
        return web.json_response({
            'status': 'ok',
            'data': messages,
            'total': len(messages)
        })
    except ValueError as e:
        return web.json_response(
            {'status': 'error', 'message': '无效的用户 ID'},
            status=400
        )
    except Exception as e:
        logger.error(f"获取聊天历史失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_send_message(request: web.Request) -> web.Response:
    """发送消息给用户（支持文本和图片）"""
    try:
        user_id = int(request.match_info['user_id'])
        
        # 检测请求类型：FormData 或 JSON
        content_type = request.content_type.lower()
        
        if 'multipart/form-data' in content_type:
            # 处理文件上传（FormData）
            reader = await request.multipart()
            
            image_file = None
            content = ''
            reply_to_msg_id = None
            
            async for field in reader:
                name = field.name
                if name == 'image':
                    # 读取图片文件
                    image_file = await field.read(decode=False)
                    # 获取文件名（如果有）
                    filename = field.filename or 'image.jpg'
                elif name == 'content':
                    content = (await field.read(decode=True)).strip()
                elif name == 'reply_to_msg_id':
                    reply_to_msg_id = int((await field.read(decode=True)).strip())
            
            if not image_file:
                return web.json_response(
                    {'status': 'error', 'message': '未找到图片文件'},
                    status=400
                )
            
            # 通过 Bot 引擎发送图片
            from backend.services.bot_engine import get_bot
            bot = get_bot()
            
            try:
                # 使用 Telegram Bot API 发送图片
                from aiogram import types
                photo_file = types.InputFile(image_file, filename=filename)
                
                # 构建 caption（如果有文本内容）
                caption = content if content else None
                
                # 处理回复引用
                reply_params = None
                if reply_to_msg_id:
                    from aiogram.types import ReplyParameters
                    reply_params = ReplyParameters(message_id=reply_to_msg_id)
                
                # 发送到 Telegram
                sent_msg = await bot.send_photo(
                    chat_id=user_id,
                    photo=photo_file,
                    caption=caption,
                    reply_parameters=reply_params
                )
                
                # 保存消息到数据库
                msg_db_id = await save_message(
                    from_userid=int(request.headers.get('X-Admin-User-Id', 0)),
                    to_userid=user_id,
                    content=content or '[图片]',
                    msg_type='photo',
                    file_id=sent_msg.photo[-1].file_id if sent_msg.photo else None,
                    is_read=True,
                    tg_msg_id=sent_msg.message_id,
                )
                
                logger.info(f"成功发送图片到用户 {user_id}, TG 消息 ID: {sent_msg.message_id}")
                
                return web.json_response({
                    'status': 'ok',
                    'data': {
                        'msg_id': msg_db_id,
                        'tg_msg_id': sent_msg.message_id,
                        'file_id': sent_msg.photo[-1].file_id if sent_msg.photo else None
                    }
                })
                
            except Exception as send_err:
                logger.error(f"通过 Telegram 发送图片失败：{send_err}")
                return web.json_response(
                    {'status': 'error', 'message': f'发送到 Telegram 失败：{str(send_err)}'},
                    status=500
                )
        
        else:
            # 处理 JSON 请求（纯文本）
            data = await request.json()
            content = data.get('content', '')
            reply_to_msg_id = data.get('reply_to_msg_id')
            file_id = data.get('file_id')
            
            if not content and not file_id:
                return web.json_response(
                    {'status': 'error', 'message': '消息内容不能为空'},
                    status=400
                )
            
            # 通过 Bot 引擎发送文本消息
            from backend.services.bot_engine import get_bot
            bot = get_bot()
            
            try:
                # 处理回复引用
                reply_params = None
                if reply_to_msg_id:
                    from aiogram.types import ReplyParameters
                    reply_params = ReplyParameters(message_id=reply_to_msg_id)
                
                # 发送到 Telegram
                sent_msg = await bot.send_message(
                    chat_id=user_id,
                    text=content,
                    reply_parameters=reply_params
                )
                
                # 保存消息到数据库
                msg_db_id = await save_message(
                    from_userid=int(request.headers.get('X-Admin-User-Id', 0)),
                    to_userid=user_id,
                    content=content,
                    msg_type='text',
                    file_id=None,
                    is_read=True,
                    tg_msg_id=sent_msg.message_id,
                )
                
                logger.info(f"成功发送消息到用户 {user_id}, TG 消息 ID: {sent_msg.message_id}")
                
                return web.json_response({
                    'status': 'ok',
                    'data': {
                        'msg_id': msg_db_id,
                        'tg_msg_id': sent_msg.message_id
                    }
                })
                
            except Exception as send_err:
                logger.error(f"通过 Telegram 发送消息失败：{send_err}")
                return web.json_response(
                    {'status': 'error', 'message': f'发送到 Telegram 失败：{str(send_err)}'},
                    status=500
                )
                
    except ValueError as e:
        logger.error(f"参数错误：{e}")
        return web.json_response(
            {'status': 'error', 'message': '无效的用户 ID'},
            status=400
        )
    except Exception as e:
        logger.error(f"发送消息失败：{e}")
        import traceback
        logger.error(traceback.format_exc())
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_delete_single_message(request: web.Request) -> web.Response:
    """删除单条消息"""
    try:
        user_id = int(request.match_info['user_id'])
        data = await request.json()
        msg_id = int(data.get('msg_id', 0))
        
        if not msg_id:
            return web.json_response(
                {'status': 'error', 'message': '消息 ID 无效'},
                status=400
            )
        
        success = await delete_single_message(user_id, msg_id)
        
        if success:
            return web.json_response({'status': 'ok'})
        else:
            return web.json_response(
                {'status': 'error', 'message': '删除失败'},
                status=500
            )
    except ValueError as e:
        return web.json_response(
            {'status': 'error', 'message': '参数无效'},
            status=400
        )
    except Exception as e:
        logger.error(f"删除消息失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_mark_read(request: web.Request) -> web.Response:
    """标记为已读"""
    try:
        user_id = int(request.match_info['user_id'])
        await mark_messages_as_read(user_id)
        return web.json_response({'status': 'ok'})
    except ValueError as e:
        return web.json_response(
            {'status': 'error', 'message': '无效的用户 ID'},
            status=400
        )
    except Exception as e:
        logger.error(f"标记已读失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_rebuild_inbox(request: web.Request) -> web.Response:
    """手动重建 Inbox（调试用）"""
    try:
        await rebuild_inbox_from_messages()
        users = await get_inbox_list()
        return web.json_response({
            'status': 'ok',
            'message': f'已重建 {len(users)} 个用户的 Inbox',
            'data': users
        })
    except Exception as e:
        logger.error(f"重建 Inbox 失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_delete_chat(request: web.Request) -> web.Response:
    """清空会话（软删除）"""
    try:
        from backend.repositories.user_repo import soft_delete_chat
        user_id = int(request.match_info['user_id'])
        await soft_delete_chat(user_id)
        return web.json_response({'status': 'ok'})
    except ValueError as e:
        return web.json_response(
            {'status': 'error', 'message': '无效的用户 ID'},
            status=400
        )
    except Exception as e:
        logger.error(f"清空会话失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )


async def api_search(request: web.Request) -> web.Response:
    """全局搜索"""
    try:
        keyword = request.query.get('q', '')
        limit = int(request.query.get('limit', 30))
        group_id = request.query.get('group_id')
        user_id = request.query.get('user_id')
        
        if not keyword:
            return web.json_response(
                {'status': 'error', 'message': '请输入搜索关键词'},
                status=400
            )
        
        results = await search_messages(
            keyword,
            group_id=int(group_id) if group_id else None,
            user_id=int(user_id) if user_id else None,
            limit=limit,
            tenant_id=request.get('tenant_id', ''),
            role=request.get('role', 'merchant_owner'),
        )
        
        return web.json_response({
            'status': 'ok',
            'data': results,
            'total': len(results)
        })
    except ValueError as e:
        return web.json_response(
            {'status': 'error', 'message': '参数无效'},
            status=400
        )
    except Exception as e:
        logger.error(f"搜索失败：{e}")
        return web.json_response(
            {'status': 'error', 'message': str(e)},
            status=500
        )
