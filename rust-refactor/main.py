# -*- coding: utf-8 -*-
"""
Telegram Bot 主入口 - 多商户版本
"""
import os
import logging
import asyncio
from telegram.ext import Application, CommandHandler, MessageHandler, CallbackQueryHandler, filters
from telegram.error import TelegramError

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

# 从环境变量获取商户配置
# BOT_TOKENS = '{"merchant1": "token1", "merchant2": "token2"}'
BOT_TOKENS = os.getenv("BOT_TOKENS", "{}")
MERCHANT_TOKENS = {}

try:
    import json
    MERCHANT_TOKENS = json.loads(BOT_TOKENS)
except:
    logger.warning("BOT_TOKENS解析失败，使用单商户模式")
    MERCHANT_TOKENS = {"default": os.getenv("BOT_TOKEN", "")}


class TGBotMultiMerchant:
    """多商户Telegram Bot管理器"""

    def __init__(self):
        self.apps = {}  # merchant_id -> Application
        self.handlers = {}

    def register_handlers(self, merchant_id: str, application: Application):
        """注册商户Bot的处理器"""

        # 基础命令
        application.add_handler(CommandHandler("start", self.start_command))
        application.add_handler(CommandHandler("help", self.help_command))
        application.add_handler(CommandHandler("admin", self.admin_command))

        # 消息处理
        application.add_handler(
            MessageHandler(filters.TEXT & ~filters.COMMAND, self.handle_text)
        )
        application.add_handler(
            MessageHandler(filters.PHOTO, self.handle_photo)
        )
        application.add_handler(
            MessageHandler(filters.VOICE | filters.AUDIO | filters.VIDEO, self.handle_media)
        )

        # 回调处理
        application.add_handler(CallbackQueryHandler(self.handle_callback))

        self.apps[merchant_id] = application
        logger.info(f"[{merchant_id}] Bot处理器注册完成")

    async def start_command(self, update, context):
        """处理 /start 命令"""
        await update.message.reply_text(
            "欢迎使用 TGBot Pro！\n\n发送 /help 查看帮助"
        )

    async def help_command(self, update, context):
        """处理 /help 命令"""
        await update.message.reply_text(
            "📖 **帮助文档**\n\n"
            "/start - 开始使用\n"
            "/help - 查看帮助\n"
            "/admin - 管理面板\n\n"
            "直接发送消息开始对话",
            parse_mode="Markdown"
        )

    async def admin_command(self, update, context):
        """处理 /admin 命令"""
        # 转发给Web Admin处理
        # TODO: 调用后端API
        await update.message.reply_text(
            "🔧 管理面板请访问Web端"
        )

    async def handle_text(self, update, context):
        """处理文本消息"""
        text = update.message.text
        user = update.effective_user

        logger.info(f"消息 from {user.id}: {text[:50]}")

        # TODO: 发送到后端AI处理 + 广告检测
        # 1. 保存消息
        # 2. AI响应
        # 3. 广告检测
        # 4. 翻译

    async def handle_photo(self, update, context):
        """处理图片消息"""
        # OCR检测 + 广告检测
        photo = update.message.photo[-1]
        await update.message.reply_text("图片已收到，正在识别...")

    async def handle_media(self, update, context):
        """处理媒体消息"""
        await update.message.reply_text("媒体消息已收到")

    async def handle_callback(self, update, context):
        """处理回调按钮"""
        query = update.callback_query
        await query.answer()

        data = query.data
        logger.info(f"Callback: {data}")

        # 处理各种按钮回调
        # mark_read:123456
        # quick_reply:123456
        # do_quick:123456:1

    async def start_all(self):
        """启动所有商户Bot"""
        for merchant_id, token in MERCHANT_TOKENS.items():
            if not token:
                continue

            logger.info(f"启动商户Bot: {merchant_id}")
            app = Application.builder().token(token).build()

            self.register_handlers(merchant_id, app)

            try:
                await app.run_polling(drop_pending_updates=True)
            except TelegramError as e:
                logger.error(f"[{merchant_id}] Bot启动失败: {e}")


async def main():
    """主入口"""
    bot_manager = TGBotMultiMerchant()
    await bot_manager.start_all()


if __name__ == "__main__":
    asyncio.run(main())