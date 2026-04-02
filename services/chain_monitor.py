# -*- coding: utf-8 -*-
"""
链上 USDT 钱包监听引擎 (高性能区块扫描版 v2.0)
架构：Event Log Filtering (O(1) API Consumption)
"""
import asyncio
import logging
import httpx
import database
import config
import time
from datetime import datetime
from services.secret_manager import SecretManager
from handlers.customer_service import safe_send_message

logger = logging.getLogger(__name__)

# USDT 合约地址
TRC20_USDT = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"
BEP20_USDT = "0x55d398326f99059fF775485246999027B3197955"
# Transfer 事件的 Topic0 (ERC20 标准)
TRANSFER_TOPIC = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"

async def _get_sync_state(network: str):
    async with database.aiosqlite.connect(database.DB_NAME) as db:
        db.row_factory = database.aiosqlite.Row
        cursor = await db.execute("SELECT * FROM chain_sync_state WHERE network = ?", (network,))
        row = await cursor.fetchone()
        if not row:
            await db.execute("INSERT INTO chain_sync_state (network, last_scanned_block, last_scanned_ts) VALUES (?, 0, ?)", 
                           (network, int(time.time() * 1000)))
            await db.commit()
            return 0, int(time.time() * 1000)
        return row['last_scanned_block'], row['last_scanned_ts']

async def _update_sync_state(network: str, block: int = None, ts: int = None):
    async with database.aiosqlite.connect(database.DB_NAME) as db:
        if block is not None:
            await db.execute("UPDATE chain_sync_state SET last_scanned_block = ? WHERE network = ?", (block, network))
        if ts is not None:
            await db.execute("UPDATE chain_sync_state SET last_scanned_ts = ? WHERE network = ?", (ts, network))
        await db.commit()

async def _detect_trc20_energy_risk(client: httpx.AsyncClient, address: str):
    # 保留原有的风险识别逻辑
    try:
        api_key = await SecretManager.get_config("TRONGRID_API_KEY", default=config.TRONGRID_API_KEY)
        headers = {"TRON-PRO-API-KEY": api_key}
        url = f"https://api.trongrid.io/v1/accounts/{address}/transactions?only_to=true&only_confirmed=true&limit=10"
        resp = await client.get(url, headers=headers)
        data = resp.json()
        txs = data.get('data') or []
        if not txs: return 'normal', None
        trx_incoming = sum(1 for tx in txs if (((tx.get('raw_data') or {}).get('contract') or [{}])[0] or {}).get('type') == 'TransferContract')
        ratio = trx_incoming / len(txs)
        if len(txs) >= 5 and ratio >= 0.8:
            return 'high', "该地址 TRX 原生转账占比过高，疑似能量租赁或钓鱼地址"
        return 'normal', None
    except: return 'normal', None

async def scan_trc20(client: httpx.AsyncClient, monitor_map: dict):
    _, last_ts = await _get_sync_state('trc20')
    # TronGrid V1 允许按合约拉取全局事件
    url = f"https://api.trongrid.io/v1/contracts/{TRC20_USDT}/events"
    params = {
        "event_name": "Transfer",
        "order_by": "block_timestamp,asc",
        "min_block_timestamp": last_ts + 1,
        "only_confirmed": "true",
        "limit": 200
    }
    headers = {"TRON-PRO-API-KEY": await SecretManager.get_config("TRONGRID_API_KEY", default=config.TRONGRID_API_KEY)}
    
    try:
        resp = await client.get(url, params=params, headers=headers)
        res = resp.json()
        if not res.get('success'): return

        events = res.get('data', [])
        max_ts = last_ts

        for ev in events:
            ts = ev['block_timestamp']
            max_ts = max(max_ts, ts)
            
            args = ev.get('result', {})
            from_addr = args.get('from')
            to_addr = args.get('to')
            value = float(args.get('value', 0)) / 1e6
            tx_id = ev['transaction_id']

            # 核心匹配逻辑：O(1) 查询内存哈希表
            hit_addr = None
            direction = None
            if to_addr in monitor_map: 
                hit_addr = to_addr; direction = 'in'
            elif from_addr in monitor_map:
                hit_addr = from_addr; direction = 'out'

            if hit_addr:
                monitor = monitor_map[hit_addr]
                await process_hit(client, 'TRC20', hit_addr, from_addr, to_addr, value, tx_id, ts, direction, monitor)

        if max_ts > last_ts:
            await _update_sync_state('trc20', ts=max_ts)
    except Exception as e:
        logger.error(f"TRC20 Scan Error: {e}")

async def scan_bep20(client: httpx.AsyncClient, monitor_map: dict):
    last_block, _ = await _get_sync_state('bsc')
    api_key = await SecretManager.get_config("BSCSCAN_API_KEY", default=config.BSCSCAN_API_KEY)
    # 使用 BscScan Proxy 接口获取最新块高
    url_info = f"https://api.bscscan.com/api?module=proxy&action=eth_blockNumber&apikey={api_key}"
    try:
        resp_info = await client.get(url_info)
        latest_block = int(resp_info.json()['result'], 16)
        if last_block == 0: 
            await _update_sync_state('bsc', block=latest_block - 10)
            return
        
        if latest_block <= last_block: return

        # 批量获取 USDT 合约的 Transfer 日志
        from_block = last_block + 1
        to_block = min(from_block + 100, latest_block) # 每次最多扫 100 块防止超时
        
        url_logs = (f"https://api.bscscan.com/api?module=logs&action=getLogs"
                    f"&fromBlock={from_block}&toBlock={to_block}"
                    f"&address={BEP20_USDT}&topic0={TRANSFER_TOPIC}&apikey={api_key}")
        
        resp_logs = await client.get(url_logs)
        logs = resp_logs.json().get('result', [])
        if not isinstance(logs, list): return

        for log in logs:
            # 解析 ERC20 Transfer Log Topic (topic1=from, topic2=to)
            from_addr = "0x" + log['topics'][1][-40:]
            to_addr = "0x" + log['topics'][2][-40:]
            value = int(log['data'], 16) / 1e18
            tx_hash = log['transactionHash']
            ts = int(log['timeStamp'], 16) * 1000

            hit_addr = None
            direction = None
            if to_addr.lower() in monitor_map: 
                hit_addr = to_addr.lower(); direction = 'in'
            elif from_addr.lower() in monitor_map:
                hit_addr = from_addr.lower(); direction = 'out'

            if hit_addr:
                monitor = monitor_map[hit_addr]
                await process_hit(client, 'BSC', hit_addr, from_addr, to_addr, value, tx_hash, ts, direction, monitor)

        await _update_sync_state('bsc', block=to_block)
    except Exception as e:
        logger.error(f"BSC Scan Error: {e}")

async def process_hit(client, network, hit_addr, from_addr, to_addr, value, tx_id, ts, direction, monitor):
    m_id = monitor['monitor_id']
    user_id = monitor['user_id']
    remark = (monitor.get('remark') or '').strip()
    counterparty = from_addr if direction == 'in' else to_addr

    # 只有转入且是 TRC20 时做风险检查
    risk_level, risk_note = 'normal', None
    if direction == 'in' and network == 'TRC20':
        risk_level, risk_note = await _detect_trc20_energy_risk(client, hit_addr)
        await database.update_monitor_risk(m_id, risk_level, risk_note)

    # 发送 TG 通知
    icon = "📥" if direction == 'in' else "📤"
    risk_block = f"\n\n⚠️ <b>高风险提示</b>\n{risk_note}" if risk_level == 'high' else ""
    msg = (
        f"🚨 <b>您的钱包发生 USDT 变动！</b>\n\n"
        f"网络: <code>{network}</code>\n"
        f"方向: {icon} {'转入' if direction == 'in' else '转出'}\n"
        f"金额: <code>{value:.2f} USDT</code>\n"
        f"对方: <code>{counterparty}</code>"
        f"{(f'\n备注: <code>{remark}</code>' if remark else '')}\n"
        f"哈希: <a href='{( 'https://tronscan.org/#/transaction/' if network=='TRC20' else 'https://bscscan.com/tx/')}{tx_id}'>{tx_id[:8]}...</a>"
        f"{risk_block}"
    )
    await safe_send_message(None, user_id, msg, parse_mode="HTML")

    # 存入数据库
    await database.save_monitor_transaction(
        tx_id=tx_id, monitor_id=m_id, user_id=user_id, address=hit_addr, 
        network=network, direction=direction, amount=value, token_symbol='USDT', 
        tx_timestamp=ts, counterparty=counterparty, risk_level=risk_level, risk_note=risk_note
    )

    # WS 推送
    try:
        from web_admin import notify_web_clients
        await notify_web_clients("monitor_tx", {
            "user_id": user_id, "address": hit_addr, "network": network, 
            "direction": direction, "value": f"{value:.2f}", "tx_id": tx_id,
            "remark": remark, "risk_level": risk_level, "risk_note": risk_note
        })
    except: pass

async def start_chain_monitor(app):
    logger.info("🚀 [ChainMonitor v2.0] 高性能事件扫描引擎启动...")
    
    while True:
        try:
            # 1. 预载入所有监控地址到内存 (加速匹配)
            monitors = await database.get_active_monitors()
            if not monitors:
                await asyncio.sleep(15)
                continue
            
            # 构建查找表：{address: monitor_data}
            monitor_map = {m['address'].lower(): m for m in monitors}

            async with httpx.AsyncClient(timeout=30.0) as client:
                # 并发扫描两个链
                await asyncio.gather(
                    scan_trc20(client, monitor_map),
                    scan_bep20(client, monitor_map)
                )

            await asyncio.sleep(10) # 扫描频率，建议 10-15s

        except Exception as e:
            logger.error(f"Monitor Loop Exception: {e}")
            await asyncio.sleep(30)
