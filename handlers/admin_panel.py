# -*- coding: utf-8 -*-
"""
管理员配置面板 - 2026 高性能规格
1. Zero-Lag 响应规范
2. 2026 旗舰模型矩阵
3. 自定义模型名称中枢
"""
import logging
import asyncio
from telegram import Update, InlineKeyboardButton, InlineKeyboardMarkup
from telegram.ext import ContextTypes, ConversationHandler, CommandHandler, MessageHandler, CallbackQueryHandler, filters
import config
import database
from services import translator

logger = logging.getLogger(__name__)

# 对话状态
WAITING_CUSTOM_MODEL = 1
WAITING_TOKEN = 2
WAITING_SYSTEM_PROMPT = 3

# 2026 旗舰模型矩阵
MODEL_MATRIX = {
    "DeepSeek": {
        "base_url": "https://api.deepseek.com/v1",
        "models": ["deepseek-chat", "deepseek-reasoner"]
    },
    "Gemini": {
        "base_url": "https://generativelanguage.googleapis.com/v1beta/openai/",
        "models": ["gemini-3.1-pro", "gemini-3.1-flash"]
    },
    "OpenAI": {
        "base_url": "https://api.openai.com/v1",
        "models": ["gpt-5.3-pro", "gpt-5.3-mini"]
    }
}

async def admin_panel(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """显示管理员主配置面板 (Zero-Lag UI)"""
    # 立即响应
    query = update.callback_query
    if query:
        await query.answer()
        
    user = update.effective_user
    if not user or int(user.id) not in config.ADMIN_TG_IDS:
        if update.message:
            await update.message.reply_text("⛔ 仅管理员可使用此命令。")
        return ConversationHandler.END

    # 并发读取数据库配置
    tasks = [
        database.get_admin_setting("translation_engine", "llm"),
        database.get_admin_setting("llm_model", config.LLM_MODEL_NAME),
        database.get_admin_setting("ocr_force_enabled", "true")
    ]
    engine, model, ocr_str = await asyncio.gather(*tasks)
    ocr_enabled = (ocr_str or "true").lower() == "true"
    
    # 极速一键切换按钮
    engine_text = "🤖 大模型" if engine == 'llm' else "🌍 谷歌翻译"
    toggle_text = "🔄 切换为: " + ("🌍 谷歌" if engine == 'llm' else "🤖 大模型")
    
    text = (
        "🚀 <b>Admin Control Center 2026</b>\n\n"
        f"🔸 <b>当前引擎</b>: <code>{engine_text}</code>\n"
        f"🔸 <b>当前模型</b>: <code>{model}</code>\n"
        f"🔸 <b>安全核验</b>: <code>{'开启 ✅' if ocr_enabled else '关闭 ❌'}</code>\n\n"
        "请选择操作："
    )

    keyboard = [
        [InlineKeyboardButton(toggle_text, callback_data="adm:toggle_eng")],
        [InlineKeyboardButton("🧠 2026 模型矩阵", callback_data="adm:providers")],
        [InlineKeyboardButton("⌨️ 手动输入模型名称", callback_data="adm:custom_mod")],
        [InlineKeyboardButton("📝 翻译提示词", callback_data="adm:prompt")],
        [InlineKeyboardButton("🔄 刷新潜在用户", callback_data="adm:sync_potential")],
        [InlineKeyboardButton(f"{'🛡️' if ocr_enabled else '🔓'} OCR 强核验开关", callback_data="adm:ocr_toggle")],
        [InlineKeyboardButton("🧹 清理缓存", callback_data="adm:clear_cache"), InlineKeyboardButton("🔙 返回", callback_data="back_to_start")]
    ]
    
    reply_markup = InlineKeyboardMarkup(keyboard)
    if query:
        try:
            await query.edit_message_text(text, reply_markup=reply_markup, parse_mode="HTML")
        except Exception as e:
            if "Message is not modified" not in str(e):
                raise
    elif update.message:
        await update.message.reply_html(text, reply_markup=reply_markup)
    return ConversationHandler.END

async def handle_admin_callback(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """处理面板回调 (局部刷新逻辑)"""
    query = update.callback_query
    if not query:
        return None
    await query.answer() # 立即响应
    
    data = query.data
    if not data:
        return None
    
    if data == "adm:toggle_eng":
        current = await database.get_admin_setting("translation_engine", "llm")
        new_eng = "google" if current == "llm" else "llm"
        await database.save_admin_setting("translation_engine", new_eng)
        return await admin_panel(update, context)
        
    elif data == "adm:providers":
        keyboard = [
            [InlineKeyboardButton("DeepSeek", callback_data="adm:pv:DeepSeek"), InlineKeyboardButton("Gemini", callback_data="adm:pv:Gemini")],
            [InlineKeyboardButton("OpenAI", callback_data="adm:pv:OpenAI")],
            [InlineKeyboardButton("🔙 返回", callback_data="adm:main")]
        ]
        await query.edit_message_text("请选择 AI 厂商 (2026 规格)：", reply_markup=InlineKeyboardMarkup(keyboard))
        
    elif data.startswith("adm:pv:"):
        parts = data.split(":")
        if len(parts) < 3: return None
        provider = parts[2]
        if context.user_data is not None:
            context.user_data['pending_provider'] = provider
        models = MODEL_MATRIX.get(provider, {}).get("models", [])
        
        keyboard = [[InlineKeyboardButton(m, callback_data=f"adm:set_mod:{m}")] for m in models]
        keyboard.append([InlineKeyboardButton("🔙 返回", callback_data="adm:providers")])
        await query.edit_message_text(f"选择 {provider} 旗舰型号：", reply_markup=InlineKeyboardMarkup(keyboard))
        
    elif data.startswith("adm:set_mod:"):
        parts = data.split(":")
        if len(parts) < 3: return None
        model_code = parts[2]
        provider = (context.user_data or {}).get('pending_provider', 'OpenAI')
        base_url = MODEL_MATRIX.get(provider, {}).get("base_url", "https://api.openai.com/v1")
        
        await database.save_admin_setting("llm_model", model_code)
        await database.save_admin_setting("llm_base_url", base_url)
        if context.user_data is not None:
            context.user_data['pending_model'] = model_code
            context.user_data['pending_base_url'] = base_url
        
        await query.edit_message_text(
            f"✅ 已锁定: <code>{model_code}</code>\n"
            "请输入 <b>API Key</b> 进行验证 (输入 /skip 保持原样)：",
            parse_mode="HTML"
        )
        return WAITING_TOKEN

    elif data == "adm:custom_mod":
        await query.edit_message_text("⌨️ <b>自定义模型接入</b>\n\n请输入完整的模型名称 (例如: <code>gpt-6.0</code>)：", parse_mode="HTML")
        return WAITING_CUSTOM_MODEL
        
    elif data == "adm:prompt":
        curr = await database.get_admin_setting("system_prompt", "Professional Translator")
        await query.edit_message_text(
            f"📝 <b>翻译补充提示词</b>\n"
            f"当前: <code>{curr}</code>\n\n"
            "该段会<b>追加</b>在系统固定规则之后（含译文格式与术语注解），用于微调语气或业务偏好。\n"
            "请输入新内容：",
            parse_mode="HTML",
        )
        return WAITING_SYSTEM_PROMPT
        
    elif data == "adm:sync_potential":
        # 2026 规格：调用管理端同步接口并返回摘要
        try:
            import aiohttp
            # 注意：内部请求也需要 Token
            url = f"{config.WEB_BASE_URL}/api/potential_users/sync?token={config.WEB_ADMIN_TOKEN}"
            async with aiohttp.ClientSession() as session:
                async with session.post(url) as resp:
                    if resp.status == 200:
                        res = await resp.json()
                        summary = res.get("summary", "同步完成")
                        await query.answer(f"✅ {summary}", show_alert=True)
                    else:
                        await query.answer("❌ 同步接口响应异常", show_alert=True)
        except Exception as e:
            logger.error(f"Sync Potential Error: {e}")
            await query.answer(f"❌ 同步失败: {e}", show_alert=True)
        return await admin_panel(update, context)
        
    elif data == "adm:ocr_toggle":
        ocr_str = await database.get_admin_setting("ocr_force_enabled", "true")
        curr = (ocr_str or "true").lower() == "true"
        await database.save_admin_setting("ocr_force_enabled", str(not curr).lower())
        return await admin_panel(update, context)
        
    elif data == "adm:clear_cache":
        if context.user_data is not None:
            context.user_data.clear()
        await query.answer("🧹 缓存已清理")
        return await admin_panel(update, context)
        
    elif data == "adm:main":
        return await admin_panel(update, context)
        
    return None

async def save_custom_model(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """保存自定义模型名称"""
    if not update.message or not update.message.text:
        return WAITING_CUSTOM_MODEL
    model = update.message.text.strip()
    await database.save_admin_setting("llm_model", model)
    if context.user_data is not None:
        context.user_data['pending_model'] = model
    await update.message.reply_text(f"✅ 模型名称已更新为: <code>{model}</code>\n请输入 API Key 或发送 /skip：", parse_mode="HTML")
    return WAITING_TOKEN

async def save_token(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """异步验证并保存 Token"""
    if not update.message or not update.message.text:
        return WAITING_TOKEN
    token = update.message.text.strip()
    model = (context.user_data or {}).get('pending_model', 'Unknown')
    base_url = await database.get_admin_setting("llm_base_url", config.LLM_BASE_URL)
    
    try: await update.message.delete()
    except: pass

    if token == "/skip":
        await update.message.reply_text("✅ 配置已保持。")
    else:
        status_msg = await update.message.reply_text("⏳ 正在验证 2026 规格链路...")
        if await translator.verify_token_async(token, model, base_url):
            key_field = "llm_api_key_deepseek" if 'deepseek' in model.lower() else ("llm_api_key_gemini" if 'gemini' in model.lower() else "llm_api_key_generic")
            await database.save_admin_setting(key_field, token)
            await status_msg.edit_text(f"✅ 验证成功！{model} 已激活。")
        else:
            await status_msg.edit_text("❌ 验证失败！请检查 Token 或网络。")
        await asyncio.sleep(1.5)
        await status_msg.delete()
        
    if context.user_data is not None:
        context.user_data.pop('pending_model', None)
        context.user_data.pop('pending_base_url', None)
        context.user_data.pop('pending_provider', None)
    return await admin_panel(update, context)

async def save_prompt(update: Update, context: ContextTypes.DEFAULT_TYPE):
    if not update.message or not update.message.text:
        return WAITING_SYSTEM_PROMPT
    await database.save_admin_setting("system_prompt", update.message.text.strip())
    msg = await update.message.reply_text("✅ 提示词已更新。")
    await asyncio.sleep(1)
    if msg:
        await msg.delete()
    return await admin_panel(update, context)

async def cancel_settings(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """退出配置 (安全清理)，若为 /start 则继续执行对应命令"""
    cmd = (update.message.text or '').strip().lower() if update.message else ''
    if cmd.startswith('/start'):
        from handlers.start import start
        await start(update, context)
    elif update.message:
        await update.message.reply_text("已退出配置。")
    elif update.callback_query:
        await update.callback_query.edit_message_text("已退出配置。")
    return ConversationHandler.END

async def sync_potential_command(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """/sync_potential 指令：管理员立即同步潜在用户数据"""
    user = update.effective_user
    if not user or int(user.id) not in config.ADMIN_TG_IDS:
        return
        
    msg = await update.message.reply_text("🔄 正在触发云端同步...")
    try:
        import aiohttp
        url = f"{config.WEB_BASE_URL}/api/potential_users/sync?token={config.WEB_ADMIN_TOKEN}"
        async with aiohttp.ClientSession() as session:
            async with session.post(url) as resp:
                if resp.status == 200:
                    res = await resp.json()
                    summary = res.get("summary", "同步成功")
                    await msg.edit_text(f"✅ <b>同步完成</b>\n{summary}", parse_mode="HTML")
                else:
                    await msg.edit_text("❌ 同步接口异常")
    except Exception as e:
        await msg.edit_text(f"❌ 同步失败: {e}")

admin_config_handler = ConversationHandler(
    entry_points=[
        CommandHandler("admin", admin_panel),
        CallbackQueryHandler(handle_admin_callback, pattern="^adm:")
    ],
    states={
        WAITING_CUSTOM_MODEL: [MessageHandler(filters.TEXT & ~filters.COMMAND & ~filters.REPLY, save_custom_model)],
        WAITING_TOKEN: [MessageHandler(filters.TEXT & ~filters.COMMAND & ~filters.REPLY, save_token), CommandHandler("skip", save_token)],
        WAITING_SYSTEM_PROMPT: [MessageHandler(filters.TEXT & ~filters.COMMAND & ~filters.REPLY, save_prompt)]
    },
    fallbacks=[
        CommandHandler("cancel", cancel_settings),
        CommandHandler("start", cancel_settings),
    ],
    per_message=False
)
