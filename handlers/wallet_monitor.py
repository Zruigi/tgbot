# -*- coding: utf-8 -*-
"""
钱包监听与即时查账处理器
"""
import logging
import httpx
import asyncio
from datetime import datetime
from telegram import Update, InlineKeyboardButton, InlineKeyboardMarkup
from telegram.ext import ContextTypes, ConversationHandler, CommandHandler, MessageHandler, CallbackQueryHandler, filters
import config
import database

logger = logging.getLogger(__name__)

# 定义状态
WAITING_ADDRESS = 1

async def start_wallet_monitor(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """开始钱包监听流程"""
    query = update.callback_query
    await query.answer()
    
    await query.edit_message_text(
        "👛 <b>USDT 钱包监听</b>\n\n"
        "请输入您要查询和监听的 USDT 地址：\n"
        "支持 <code>TRC20</code> (T开头) 或 <code>BSC</code> (0x开头)",
        parse_mode="HTML"
    )
    return WAITING_ADDRESS

async def handle_address_input(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """处理用户输入的钱包地址并即时查账"""
    address = update.message.text.strip()
    user_id = update.effective_user.id
    
    # 自动判断网络
    network = None
    if address.startswith('T'):
        network = 'trc20'
    elif address.startswith('0x'):
        network = 'bsc'
    
    if not network:
        await update.message.reply_text("❌ 无效的地址格式。请发送正确的 TRC20 或 BSC 地址。")
        return WAITING_ADDRESS

    processing_msg = await update.message.reply_text("🔍 正在查询链上数据，请稍候...")
    
    try:
        recent_txs = []
        latest_timestamp = None
        async with httpx.AsyncClient(timeout=20.0) as client:
            if network == 'trc20':
                url = f"https://api.trongrid.io/v1/accounts/{address}/transactions/trc20?limit=20&order_by=block_timestamp,desc"
                headers = {"TRON-PRO-API-KEY": config.TRONGRID_API_KEY}
                resp = await client.get(url, headers=headers)
                data = resp.json()
                if data.get('success') and 'data' in data:
                    usdt_txs = [tx for tx in data['data'] if tx.get('token_info', {}).get('address') == 'TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t']
                    if usdt_txs:
                        latest_timestamp = usdt_txs[0]['block_timestamp']
                    for tx in usdt_txs[:10]:
                        val = float(tx['value']) / (10**6)
                        direct = "📥 转入" if tx['to'] == address else "📤 转出"
                        dt = datetime.fromtimestamp(tx['block_timestamp']/1000).strftime('%Y-%m-%d %H:%M')
                        recent_txs.append(f"{direct} {val:.2f} USDT ({dt})")
            
            elif network == 'bsc':
                url = (
                    f"https://api.bscscan.com/api?module=account&action=tokentx"
                    f"&contractaddress=0x55d398326f99059fF775485246999027B3197955"
                    f"&address={address}&sort=desc&page=1&offset=20&apikey={config.BSCSCAN_API_KEY}"
                )
                resp = await client.get(url)
                data = resp.json()
                if data.get('status') == '1' and 'result' in data:
                    if data['result']:
                        latest_timestamp = int(data['result'][0]['timeStamp']) * 1000
                    for tx in data['result'][:10]:
                        val = float(tx['value']) / (10**18)
                        direct = "📥 转入" if tx['to'].lower() == address.lower() else "📤 转出"
                        dt = datetime.fromtimestamp(int(tx['timeStamp'])).strftime('%Y-%m-%d %H:%M')
                        recent_txs.append(f"{direct} {val:.2f} USDT ({dt})")

        # 构建回复内容
        tx_text = "\n".join([f"{i+1}. {tx}" for i, tx in enumerate(recent_txs)]) if recent_txs else "暂无近期 USDT 转账记录"
        
        msg = (
            f"📊 <b>地址近期 USDT 账单</b>\n"
            f"网络: <code>{network.upper()}</code>\n"
            f"地址: <code>{address}</code>\n\n"
            f"{tx_text}\n\n"
            f"✅ 该地址已自动加入 24/7 实时监听队列！有新账单将自动推送。"
        )
        
        # 外部链接按钮
        scan_url = f"https://tronscan.org/#/address/{address}" if network == 'trc20' else f"https://bscscan.com/address/{address}"
        keyboard = [
            [InlineKeyboardButton(f"🔗 前往 {'TronScan' if network=='trc20' else 'BscScan'} 查看完整账单", url=scan_url)],
            [InlineKeyboardButton("🔙 返回主菜单", callback_data="back_to_start")]
        ]
        
        await processing_msg.delete()
        await update.message.reply_html(msg, reply_markup=InlineKeyboardMarkup(keyboard))
        
        # 入库保存，设置初始时间戳，避免推送历史消息
        await database.save_wallet_monitor(user_id, address, network, initial_timestamp=latest_timestamp)
        logger.info(f"Wallet monitor saved: {address} for user {user_id} with ts {latest_timestamp}")

    except Exception as e:
        logger.error(f"Wallet query failed for {address}: {e}")
        await processing_msg.edit_text("❌ 查询失败：无法连接至链上节点。地址已尝试保存至监听列表。")
        await database.save_wallet_monitor(user_id, address, network)

    return ConversationHandler.END

async def cancel_wallet_monitor(update: Update, context: ContextTypes.DEFAULT_TYPE):
    """取消监听流程"""
    await update.message.reply_text("已取消设置。")
    return ConversationHandler.END

wallet_monitor_handler = ConversationHandler(
    entry_points=[CallbackQueryHandler(start_wallet_monitor, pattern="^wallet_monitor$")],
    states={
        WAITING_ADDRESS: [MessageHandler(filters.TEXT & ~filters.COMMAND, handle_address_input)]
    },
    fallbacks=[CommandHandler("cancel", cancel_wallet_monitor)],
    per_message=False
)
