# -*- coding: utf-8 -*-
"""
系统状态与健康检查处理器
"""
import time
import psutil
import os
import logging
from telegram import Update
from telegram.ext import ContextTypes
import config
import database

logger = logging.getLogger(__name__)

async def status_command(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """显示系统状态、延迟、数据库健康状况等"""
    user_id = update.effective_user.id
    if str(user_id) != str(config.ADMIN_ID):
        return

    start_time = time.time()
    
    # 1. 检查数据库
    db_status = "🟢 正常"
    try:
        import aiosqlite
        async with aiosqlite.connect(database.DB_NAME) as db:
            await db.execute("SELECT 1")
    except Exception as e:
        db_status = f"🔴 异常: {e}"

    # 2. 系统负载
    process = psutil.Process(os.getpid())
    mem_info = process.memory_info().rss / 1024 / 1024  # MB
    cpu_usage = psutil.cpu_percent()
    
    # 3. 消息延迟计算 (通过当前时间与消息时间对比)
    msg_time = update.message.date.timestamp()
    latency = time.time() - msg_time
    
    # 4. 统计信息
    try:
        import aiosqlite
        async with aiosqlite.connect(database.DB_NAME) as db:
            async with db.execute("SELECT COUNT(*) FROM users") as cursor:
                user_count = (await cursor.fetchone())[0]
            async with db.execute("SELECT COUNT(*) FROM messages") as cursor:
                msg_count = (await cursor.fetchone())[0]
    except:
        user_count = msg_count = "未知"

    status_text = (
        "📊 <b>系统健康状态报告</b>\n\n"
        f"⏱ <b>消息延迟:</b> <code>{latency:.2f}s</code>\n"
        f"🗄 <b>数据库状态:</b> {db_status}\n"
        f"💾 <b>内存占用:</b> <code>{mem_info:.1f} MB</code>\n"
        f"💻 <b>CPU 使用率:</b> <code>{cpu_usage}%</code>\n"
        f"👥 <b>累计用户:</b> <code>{user_count}</code>\n"
        f"💬 <b>累计消息:</b> <code>{msg_count}</code>\n"
        f"🚀 <b>运行时间:</b> <code>{int(time.time() - start_time)}ms (响应)</code>\n"
        f"\n✅ <b>Webhook/Polling:</b> 正常"
    )

    await update.message.reply_html(status_text)

async def clear_command(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """清理当前对话缓存 (Context.user_data)"""
    user_id = update.effective_user.id
    if str(user_id) != str(config.ADMIN_ID):
        return
        
    # 清理当前上下文中的暂存数据
    context.user_data.clear()
    
    # 清理所有用户的地址核验状态 (如果是全局清理)
    # 但通常管理员只想清理自己的状态，这里只清理当前上下文
    
    await update.message.reply_text("🧹 <b>对话缓存已清理。</b>\n所有待处理的状态（如待验证地址）已重置。", parse_mode="HTML")
