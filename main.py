# -*- coding: utf-8 -*-
"""
Bot 主入口文件
"""
import asyncio
import logging
import os
import sys
from datetime import datetime, timedelta
from pathlib import Path
from telegram import Update
from telegram.ext import Application, CommandHandler, CallbackQueryHandler, MessageHandler, filters, ContextTypes

import config
from handlers.start import start, button_callback, restart_bot_command
from handlers.wallet_panel import (
    my_wallets_command,
    wallet_my_list_callback,
    wallet_admin_all_callback,
    unwatch_callback,
)
from handlers.alert_setting import email_conv_handler
from handlers.crypto_price import crypto_price_handler
from handlers.wallet_monitor import wallet_monitor_handler
from handlers.customer_service import (
    handle_admin_reply,
    handle_admin_non_reply,
    admin_reply_command,
    handle_user_message,
    cs_callback_handler,
    select_other_user_callback,
    user_page_callback,
    confirm_send_to_callback,
)
from handlers.group_monitor import (
    group_track_handler,
    group_message_handler,
    channel_track_handler,
    new_member_handler,
    left_member_handler,
)
from handlers.join_verify import (
    new_member_verify_handler,
    verify_message_handler,
    verify_callback_handler,
    chat_member_verify_handler,
)
from handlers.status import status_command, clear_command
from handlers.admin_panel import admin_config_handler, sync_potential_command
from handlers.channel_verify import handle_subscribe_button, handle_start_command_with_channel_check
from handlers.reminder_cleanup import cleanup_expired_reminders
import database
from database import init_db
from web_admin import start_web_server
from services.price_monitor import start_price_monitor
from services.chain_monitor import start_chain_monitor

# 配置日志
for p in (config.LOG_DIR, config.LOG_APP_DIR, config.LOG_TUNNEL_DIR, config.LOG_RUNTIME_DIR):
    os.makedirs(p, exist_ok=True)
logging.basicConfig(
    format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
    level=logging.INFO,
    handlers=[
        logging.FileHandler(config.BOT_LOG_PATH, encoding="utf-8"),
        logging.StreamHandler()
    ]
)
# 针对 Windows 控制台的 GBK 编码问题，尝试设置标准输出编码
if hasattr(sys.stdout, 'reconfigure'):
    sys.stdout.reconfigure(encoding='utf-8')
logger = logging.getLogger(__name__)

async def migrate_legacy_log_files() -> None:
    """将历史根目录日志迁移到 logs 目录"""
    legacy_names = [
        "bot.log",
        "backend.log",
        "cloudflared.log",
        "cloudflared.stdout",
        "cloudflared-8080.log",
        "cloudflared-8080.stdout",
        "bot_runtime.log",
        "bot_stdout.log",
        "bot_stderr.log",
        "cf_frontend.log",
        "cloudflared_frontend.log",
    ]

    category_map = {
        "bot.log": config.LOG_APP_DIR,
        "backend.log": config.LOG_APP_DIR,
        "cloudflared.log": config.LOG_TUNNEL_DIR,
        "cloudflared.stdout": config.LOG_TUNNEL_DIR,
        "cloudflared-8080.log": config.LOG_TUNNEL_DIR,
        "cloudflared-8080.stdout": config.LOG_TUNNEL_DIR,
        "cloudflared_frontend.log": config.LOG_TUNNEL_DIR,
        "cf_frontend.log": config.LOG_TUNNEL_DIR,
        "bot_runtime.log": config.LOG_RUNTIME_DIR,
        "bot_stdout.log": config.LOG_RUNTIME_DIR,
        "bot_stderr.log": config.LOG_RUNTIME_DIR,
    }

    for name in legacy_names:
        src = Path(config.BASE_DIR) / name
        if not src.exists() or not src.is_file():
            continue

        dest_dir = Path(category_map.get(name, config.LOG_APP_DIR))
        dest_dir.mkdir(parents=True, exist_ok=True)

        if name.startswith("cloudflared-8080"):
            dest_name = name.replace("cloudflared-8080", "cloudflared")
        else:
            dest_name = name

        dest = dest_dir / dest_name
        if dest.exists():
            continue

        try:
            src.rename(dest)
            logger.info(f"[日志迁移] 已迁移: {name} -> {dest.relative_to(Path(config.BASE_DIR))}")
        except Exception as e:
            logger.warning(f"[日志迁移] 迁移失败 {name}: {e}")


async def cleanup_old_logs() -> None:
    """清理超过保留时长的日志文件"""
    try:
        retention = max(1, int(config.LOG_RETENTION_HOURS))
    except Exception:
        retention = 48

    cutoff = datetime.now() - timedelta(hours=retention)
    log_dir = Path(config.LOG_DIR)
    if not log_dir.exists():
        return

    for path in log_dir.rglob("*.log"):
        try:
            mtime = datetime.fromtimestamp(path.stat().st_mtime)
            if mtime < cutoff:
                path.unlink(missing_ok=True)
                logger.info(f"[日志清理] 已删除过期日志: {path.name}")
        except Exception as e:
            logger.warning(f"[日志清理] 删除日志失败 {path.name}: {e}")


async def log_cleanup_loop() -> None:
    """每小时执行一次日志清理"""
    while True:
        await cleanup_old_logs()
        await asyncio.sleep(3600)


async def revoke_sync_loop(bot) -> None:
    """定时对账：检查开启信息校对的群组，发现消息已被撤回则同步删除 web 记录
    优化：并发处理多群、只校对 48h 内消息、动态间隔、限速保护
    """
    logger.info("[信息校对] 对账循环已启动")

    async def check_group(group_id: int, messages: list, target: int) -> list:
        """校对单个群，返回已删除的 tg_msg_id 列表"""
        deleted_ids = []
        for i, msg in enumerate(messages):
            tg_msg_id = msg['tg_msg_id']
            try:
                fwd = await bot.forward_message(
                    chat_id=target,
                    from_chat_id=group_id,
                    message_id=tg_msg_id,
                    disable_notification=True,
                )
                # 转发成功，消息还在：删暂存副本 + 标记已校对（防止下次循环重复处理）
                try:
                    await bot.delete_message(chat_id=target, message_id=fwd.message_id)
                except Exception:
                    pass
                # 标记已校对，避免重复转发导致死循环
                await database.mark_group_message_revoke_checked(group_id, tg_msg_id)
                # 每处理 5 条暂停一下，避免触发 Telegram 限速
                if (i + 1) % 5 == 0:
                    await asyncio.sleep(0.3)
            except Exception as fwd_ex:
                err = str(fwd_ex).lower()
                if any(k in err for k in ('message to forward not found', 'message_id_invalid', 'invalid message', 'message not found')):
                    deleted = await database.delete_group_message(group_id, tg_msg_id)
                    if deleted:
                        deleted_ids.append(tg_msg_id)
                        logger.info(f"[信息校对] 群 {group_id} 消息 {tg_msg_id} 已撤回，同步删除")
                elif 'flood' in err or 'too many' in err:
                    # 被限速，停一下再继续
                    logger.warning(f"[信息校对] Telegram 限速，暂停 5s")
                    await asyncio.sleep(5)
        return deleted_ids

    while True:
        try:
            await asyncio.sleep(30)  # 基础间隔 30s，比之前快一倍
            staging_chat_id_str = await database.get_admin_setting('revoke_sync_chat_id')
            staging_chat_id = int(staging_chat_id_str) if staging_chat_id_str else None

            # 未配置暂存频道时跳过，防止 forward 回原群造成死循环
            if not staging_chat_id:
                logger.debug("[信息校对] 未配置 revoke_sync_chat_id，跳过本轮")
                continue

            groups = await database.get_groups_with_revoke_sync()
            if not groups:
                continue

            # 并发处理所有群，但最多同时 3 个，避免 API 过载
            sem = asyncio.Semaphore(3)

            async def run_group(g):
                async with sem:
                    gid = g['group_id']
                    target = staging_chat_id
                    return gid, await check_group(gid, g['messages'], target)

            results = await asyncio.gather(*[run_group(g) for g in groups], return_exceptions=True)

            # 批量推送 WebSocket 通知
            from web_admin import notify_web_clients
            for r in results:
                if isinstance(r, Exception):
                    logger.debug(f"[信息校对] 群处理异常: {r}")
                    continue
                group_id, deleted_ids = r
                for tid in deleted_ids:
                    try:
                        await notify_web_clients("group_message_deleted", {
                            "group_id": group_id,
                            "tg_msg_id": tid,
                        })
                    except Exception:
                        pass
        except Exception as e:
            logger.debug(f"revoke_sync_loop error: {e}")


async def post_init(application: Application) -> None:
    """Bot 启动后的初始化操作"""
    await init_db()
    await migrate_legacy_log_files()
    await cleanup_old_logs()
    
    # 初始化财务模块
    try:
        from finance_core import init_finance_db, init_default_chart_of_accounts
        await init_finance_db()
        await init_default_chart_of_accounts()
        logger.info("✅ 财务模块数据库初始化完成")
    except Exception as e:
        logger.warning(f"财务模块初始化失败: {e}")
    
    # 启动 Web 管理端
    await start_web_server(application.bot)
    
    # 1. 尝试抓取管理员真实姓名并同步数据库 (预热)
    try:
        admin_chat = await application.bot.get_chat(config.ADMIN_ID)
        admin_real_name = admin_chat.first_name or admin_chat.username or "Administrator"
        await database.update_admin_info(config.ADMIN_ID, admin_real_name)
        logger.info(f"✅ 已预热管理员实名: {admin_real_name}")
    except Exception as e:
        logger.warning(f"预热管理员实名失败: {e}")

    # 2. 为管理员设置专属菜单按钮 (set_my_commands)
    try:
        from telegram import BotCommand, BotCommandScopeChat
        admin_commands = [
            BotCommand("admin", "⚙️ 启动机器人控制面板"),
            BotCommand("status", "📊 查看监听与监控运行状态"),
            BotCommand("sync_potential", "🔄 同步群组潜在用户"),
            BotCommand("clear", "🧹 清理当前对话缓存"),
            BotCommand("restart_bot", "🔄 一键重启 Bot（测试）"),
        ]
        # 仅对管理员生效
        await application.bot.set_my_commands(
            admin_commands, 
            scope=BotCommandScopeChat(chat_id=config.ADMIN_ID)
        )
        logger.info(f"✅ 已为管理员 {config.ADMIN_ID} 设置专属菜单按钮")
    except Exception as e:
        logger.warning(f"设置管理员菜单失败: {e}")

    # 启动后台监控循环任务 (挂载到后台持续运行)
    asyncio.create_task(start_price_monitor(application))
    asyncio.create_task(start_chain_monitor(application))
    asyncio.create_task(revoke_sync_loop(application.bot))
    asyncio.create_task(cleanup_expired_reminders(application.bot))  # v5.4: 频道提醒清理任务
    asyncio.create_task(log_cleanup_loop())
    logger.info(f"✅ 日志自动清理已启用，保留时长: {config.LOG_RETENTION_HOURS} 小时")
    logger.info("✅ 价格监控与链上监控引擎已挂载后台运行")

async def debug_callback(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """调试用的回调处理器"""
    if update.callback_query:
        logger.info(f"收到回调查询: {update.callback_query.data} 来自用户: {update.effective_user.id if update.effective_user else 'Unknown'}")
    elif update.message:
        logger.info(f"收到消息: {update.message.text} 来自用户: {update.effective_user.id if update.effective_user else 'Unknown'}")

async def ping(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """测试 Bot 是否在线"""
    if update.message:
        await update.message.reply_text("pong")

def main() -> None:
    """启动 Bot"""
    # 创建 Application 实例
    application = Application.builder().token(config.BOT_TOKEN).post_init(post_init).build()

    # 注册测试指令
    application.add_handler(CommandHandler("ping", ping))
    application.add_handler(CommandHandler("status", status_command))
    application.add_handler(CommandHandler("clear", clear_command))

    # 注册调试处理器 (记录所有更新)
    from telegram.ext import TypeHandler
    application.add_handler(TypeHandler(Update, debug_callback), group=-1)

    # 注册 /start 命令处理器
    application.add_handler(CommandHandler("start", start))
    application.add_handler(CommandHandler("restart_bot", restart_bot_command))
    application.add_handler(CommandHandler("sync_potential", sync_potential_command))
    application.add_handler(CommandHandler("wallets", my_wallets_command))
    application.add_handler(CallbackQueryHandler(wallet_my_list_callback, pattern=r"^wallet_my_list$"))
    application.add_handler(CallbackQueryHandler(wallet_admin_all_callback, pattern=r"^wallet_admin_all$"))
    application.add_handler(CallbackQueryHandler(unwatch_callback, pattern=r"^uw_[ua]_\d+$"))
    
    # 注册邮箱设置对话处理器
    application.add_handler(email_conv_handler)
    
    
    # 群组监听处理器 (捕捉潜在用户)
    # 2026 修复：将追踪逻辑与关键词匹配分离到不同 group，防止拦截导致功能失效
    application.add_handler(group_track_handler, group=2) 
    application.add_handler(group_message_handler, group=3)
    application.add_handler(channel_track_handler, group=2)
    application.add_handler(new_member_handler, group=3)
    application.add_handler(left_member_handler, group=3)
    # 入群验证 handlers（优先级高于普通群消息）
    application.add_handler(new_member_verify_handler, group=1)
    application.add_handler(chat_member_verify_handler, group=1)
    application.add_handler(verify_callback_handler, group=1)
    application.add_handler(verify_message_handler, group=2)
    
# 注册查价与钱包监听对话处理器 (放在全局通配符之前)
    application.add_handler(crypto_price_handler)
    application.add_handler(wallet_monitor_handler)
    
    # 1. 核心业务处理器：双向客服回复 (最高优先级)
    # 只要是回复 (Reply) 消息，无论处于何种对话状态，都优先处理为客服回复
    application.add_handler(MessageHandler(filters.REPLY & filters.User(user_id=config.ADMIN_ID), handle_admin_reply), group=0)
    
    # 2026 优化：拦截非回复类管理员消息，防止信息失踪
    application.add_handler(MessageHandler(filters.User(user_id=config.ADMIN_ID) & ~filters.REPLY & ~filters.COMMAND & filters.ChatType.PRIVATE, handle_admin_non_reply), group=0)

    # 2. 管理员配置处理器 (ConversationHandler)
    application.add_handler(admin_config_handler, group=1)
    
    # 3. 其他命令处理器
    application.add_handler(CommandHandler("reply", admin_reply_command))
    
    # 3.1 选择其他用户的回调处理器
    application.add_handler(CallbackQueryHandler(select_other_user_callback, pattern=r"^select_other_user:"))
    application.add_handler(CallbackQueryHandler(user_page_callback, pattern=r"^user_page:"))
    application.add_handler(CallbackQueryHandler(confirm_send_to_callback, pattern=r"^confirm_send_to:"))
    
    # 3.2 其他客服回调（包含一键群发）
    application.add_handler(CallbackQueryHandler(cs_callback_handler, pattern="^(mark_read|quick_reply|do_quick|back_to_main|close_session|quick_send|cancel_send|batch_send|confirm_batch_send)"))
    
    # 注册其他未实现按钮的处理器
    application.add_handler(CallbackQueryHandler(
        button_callback,
        pattern="^(spot_price|futures_price|wallet_monitor|admin_dashboard|contact_admin|cs_mode_test|debug_id|back_to_start|restart_bot)$",
    ))
    
    # 频道验证处理器 (v5.4)
    application.add_handler(CallbackQueryHandler(handle_subscribe_button, pattern='^subscribe_'))
    application.add_handler(CommandHandler("start", handle_start_command_with_channel_check))

    # 全局拦截非管理员的消息，存入数据库并转发给管理员
    # 修复：只拦截私聊 (ChatType.PRIVATE) 中的消息，忽略群聊消息，防止群里别人说话也同步给客服
    application.add_handler(MessageHandler(
        filters.ALL & ~filters.COMMAND & ~filters.User(user_id=config.ADMIN_ID) & filters.ChatType.PRIVATE, 
        handle_user_message
    ))

    # 启动 Bot
    logger.info("Bot 正在启动...")
    application.run_polling(stop_signals=None, allowed_updates=Update.ALL_TYPES)

    if config.RESTART_REQUESTED:
        logger.info("正在按管理员请求重启进程 (subprocess.Popen)…")
        config.RESTART_REQUESTED = False
        import subprocess
        # 针对 Windows 使用 start 命令开启新窗口
        # 获取当前脚本的绝对路径
        script_path = os.path.abspath(__file__)
        subprocess.Popen(f'start "TG_BOT_CORE" "{sys.executable}" "{script_path}"', shell=True)
        os._exit(0)

if __name__ == "__main__":
    main()
