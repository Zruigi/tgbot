# Telegram Bot 服务 - 多商户支持
"""
Bot Handler - 处理多商户Bot
"""
import logging
from typing import Optional, Dict, Any
from telegram import Update, Bot
from telegram.ext import Application, ContextTypes, MessageHandler, CallbackQueryHandler, CommandHandler
from telegram.ext import filters

logger = logging.getLogger(__name__)


class MerchantBotHandler:
    """商户Bot处理器"""

    def __init__(self, merchant_id: str, bot_token: str, bot: Bot):
        self.merchant_id = merchant_id
        self.bot_token = bot_token
        self.bot = bot
        self.app: Optional[Application] = None

    async def handle_message(self, update: Update, context: ContextTypes.DEFAULT_TYPE):
        """处理接收到的消息"""
        if not update.message:
            return

        user = update.effective_user
        message = update.message

        logger.info(f"[{self.merchant_id}] Message from {user.id}: {message.text[:50]}...")

        # TODO: 转发到后端API处理
        # 1. 保存消息到数据库
        # 2. AI广告检测
        # 3. 翻译处理
        # 4. 回复用户

    async def handle_callback(self, update: Update, context: ContextTypes.DEFAULT_TYPE):
        """处理回调按钮"""
        query = update.callback_query
        if not query:
            return

        data = query.data
        user_id = query.from_user.id

        logger.info(f"[{self.merchant_id}] Callback from {user_id}: {data}")

        # 解析callback_data并处理
        # 格式: action:target_id:extra

    async def start(self):
        """启动Bot"""
        self.app = Application.builder().token(self.bot_token).build()

        # 注册处理器
        self.app.add_handler(MessageHandler(filters.ALL, self.handle_message))
        self.app.add_handler(CallbackQueryHandler(self.handle_callback))

        # 启动polling
        await self.app.run_polling(drop_pending_updates=True)


class BotManager:
    """Bot管理器 - 管理多商户Bot"""

    def __init__(self):
        self.bots: Dict[str, MerchantBotHandler] = {}

    def register_bot(self, merchant_id: str, bot_token: str) -> MerchantBotHandler:
        """注册商户Bot"""
        from telegram import Bot as BotClass
        bot = BotClass(token=bot_token)
        handler = MerchantBotHandler(merchant_id, bot_token, bot)
        self.bots[merchant_id] = handler
        return handler

    def get_bot(self, merchant_id: str) -> Optional[MerchantBotHandler]:
        """获取商户Bot"""
        return self.bots.get(merchant_id)

    async def start_all(self):
        """启动所有Bot"""
        for handler in self.bots.values():
            try:
                await handler.start()
            except Exception as e:
                logger.error(f"启动Bot失败: {e}")


# 全局Bot管理器
bot_manager = BotManager()