# -*- coding: utf-8 -*-
"""
查价与价格提醒引导处理器
"""
import logging
import ccxt.async_support as ccxt
from telegram import Update, InlineKeyboardButton, InlineKeyboardMarkup
from telegram.ext import ContextTypes, ConversationHandler, CommandHandler, MessageHandler, CallbackQueryHandler, filters

logger = logging.getLogger(__name__)

# 定义状态
WAITING_SYMBOL = 1

async def start_price_query(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """开始查价流程"""
    query = update.callback_query
    await query.answer()
    
    # 记录用户是查现货还是合约
    context.user_data['price_type'] = 'spot' if query.data == 'spot_price' else 'futures'
    
    await query.edit_message_text(
        "🔍 <b>价格查询</b>\n\n请输入您要查询的币种（例如：<code>BTC/USDT</code> 或 <code>ETH/USDT</code>）：",
        parse_mode="HTML"
    )
    return WAITING_SYMBOL

async def handle_symbol_input(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """处理用户输入的币对"""
    symbol = update.message.text.strip().upper()
    price_type = context.user_data.get('price_type', 'spot')
    
    # 简单校验格式
    if '/' not in symbol:
        symbol = f"{symbol}/USDT"

    # 严格遵守免鉴权规则
    exchange = ccxt.binance({
        'enableRateLimit': True,
        'options': {'defaultType': price_type}
    })
    
    try:
        ticker = await exchange.fetch_ticker(symbol)
        last_price = ticker['last']
        change = ticker['percentage']
        
        msg = (
            f"📈 <b>{symbol} { '现货' if price_type == 'spot' else '合约'}行情</b>\n\n"
            f"最新价格: <code>{last_price}</code>\n"
            f"24h 涨跌: <code>{'+' if change > 0 else ''}{change:.2f}%</code>\n\n"
            f"💡 点击下方按钮可添加实时价格报警。"
        )
        
        keyboard = [
            [InlineKeyboardButton("🔔 添加价格提醒", callback_data="add_price_alert")],
            [InlineKeyboardButton("🔙 返回主菜单", callback_data="back_to_start")]
        ]
        
        await update.message.reply_html(msg, reply_markup=InlineKeyboardMarkup(keyboard))
    except Exception as e:
        logger.error(f"Price query failed for {symbol}: {e}")
        await update.message.reply_text(f"❌ 查询失败：未能找到交易对 {symbol}。请检查输入是否正确。")
    finally:
        await exchange.close()
        
    return ConversationHandler.END

async def cancel_price_query(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """取消查价"""
    await update.message.reply_text("已取消查询。")
    return ConversationHandler.END

crypto_price_handler = ConversationHandler(
    entry_points=[CallbackQueryHandler(start_price_query, pattern="^(spot_price|futures_price)$")],
    states={
        WAITING_SYMBOL: [MessageHandler(filters.TEXT & ~filters.COMMAND, handle_symbol_input)]
    },
    fallbacks=[CommandHandler("cancel", cancel_price_query)],
    per_message=False
)
