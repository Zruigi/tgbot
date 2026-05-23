# -*- coding: utf-8 -*-
"""
币安价格监控引擎 - 状态机逻辑
"""
import asyncio
import logging
import ccxt.async_support as ccxt
import database
import config
from handlers.customer_service import safe_send_message

logger = logging.getLogger(__name__)

async def start_price_monitor(app):
    """币安价格通道监控异步死循环"""
    logger.info("🚀 [PriceMonitor] 币安行情监控引擎启动...")
    
    # 严格遵守免鉴权规则：禁止传入任何 API Key
    exchange = ccxt.binance({
        'enableRateLimit': True,
        'options': {'defaultType': 'spot'}
    })
    
    try:
        while True:
            try:
                # 1. 数据库查询活跃任务 (仅处理 channel 类型)
                alerts = await database.get_active_alerts()
                active_alerts = [a for a in alerts if a.get('alert_type') == 'channel']
                
                if not active_alerts:
                    await asyncio.sleep(10)
                    continue
                    
                # 2. 聚合 Symbol 减少 API 调用
                symbols = list(set([a['symbol'] for a in active_alerts]))
                
                for symbol in symbols:
                    try:
                        # 调用公开无鉴权接口获取最新价
                        ticker = await exchange.fetch_ticker(symbol)
                        current_price = ticker['last']
                        
                        # 3. 严格执行状态机逻辑
                        for alert in [a for a in active_alerts if a['symbol'] == symbol]:
                            alert_id = alert['alert_id']
                            user_id = alert['user_id']
                            upper = alert['upper_line']
                            lower = alert['lower_line']
                            state = alert.get('channel_state') or 'waiting_cross_upper'
                            
                            # --- 状态 A: 等待上穿上行线 ---
                            if state == 'waiting_cross_upper' and current_price >= upper:
                                # 仅更新状态，不发通知
                                await database.update_alert_state(alert_id, 'crossed_upper_waiting_lower')
                                logger.info(f"📈 [PriceMonitor] Alert {alert_id}: {symbol} 触及上轨 {upper} (当前 {current_price}), 切换状态至 crossed_upper_waiting_lower")
                                
                            # --- 状态 B: 已上穿, 等待下穿下行线 ---
                            elif state == 'crossed_upper_waiting_lower' and current_price <= lower:
                                # 触发报警并关闭任务
                                await database.update_alert_state(alert_id, 'triggered', is_active=False)
                                msg = (
                                    f"🚨 <b>币安价格通道报警触发</b>\n\n"
                                    f"交易对: <code>{symbol}</code>\n"
                                    f"触发动作: 价格已下穿上行线并触及下行线！\n"
                                    f"下轨价格: <code>{lower}</code>\n"
                                    f"当前价格: <code>{current_price}</code>\n"
                                    f"状态: 监控已自动关闭"
                                )
                                # 使用已有的安全发送函数推送给用户
                                await safe_send_message(None, user_id, msg, parse_mode="HTML")
                                logger.info(f"🔔 [PriceMonitor] Alert {alert_id}: {symbol} 触发报警并关闭 (触及下轨 {lower})")
                                
                    except Exception as e:
                        print(f"[ERROR - PriceMonitor] 获取 {symbol} 行情失败: {e}")
                        
                # 严格遵守 10 秒间隔
                await asyncio.sleep(10)
                
            except Exception as e:
                # 绝对禁止崩溃，打印错误并继续
                print(f"[ERROR - PriceMonitor] 循环异常: {e}")
                await asyncio.sleep(30)
    finally:
        # 确保 exchange 连接被正确关闭，防止 aiohttp ClientSession 泄漏
        await exchange.close()
        logger.info("[PriceMonitor] exchange 连接已关闭")
