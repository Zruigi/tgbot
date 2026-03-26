# -*- coding: utf-8 -*-
"""
链上 USDT 钱包监听引擎 (TRC20 & BSC)
"""
import asyncio
import logging
import httpx
import database
import config
from handlers.customer_service import safe_send_message

logger = logging.getLogger(__name__)


async def _detect_trc20_energy_risk(client: httpx.AsyncClient, address: str) -> tuple[str, str | None]:
    """
    风险识别规则：
    - TRC20 地址最近收款中，若 TRX 原生转账占比 >= 80%，视为高风险（疑似能量租赁地址）。
    """
    try:
        headers = {"TRON-PRO-API-KEY": config.TRONGRID_API_KEY}
        url = f"https://api.trongrid.io/v1/accounts/{address}/transactions?only_to=true&only_confirmed=true&limit=50"
        resp = await client.get(url, headers=headers)
        data = resp.json()
        txs = data.get('data') or []

        if not txs:
            return 'normal', None

        total_incoming = len(txs)
        trx_incoming = 0
        for tx in txs:
            ctype = (((tx.get('raw_data') or {}).get('contract') or [{}])[0] or {}).get('type')
            # TransferContract = TRX 原生转账
            if ctype == 'TransferContract':
                trx_incoming += 1

        ratio = trx_incoming / total_incoming if total_incoming else 0
        if total_incoming >= 10 and ratio >= 0.8:
            note = "该地址可能视为能量租赁地址，请付款时仔细斟酌，避免被钓鱼肉包子打狗一去不复返"
            return 'high', note

        return 'normal', None
    except Exception as e:
        logger.warning(f"TRC20 风险识别失败({address}): {e}")
        return 'normal', None


async def start_chain_monitor(app):
    """链上 USDT 钱包监听异步死循环"""
    logger.info("🚀 [ChainMonitor] 链上钱包监控引擎启动...")

    while True:
        try:
            monitors = await database.get_active_monitors()
            if not monitors:
                await asyncio.sleep(20)
                continue

            async with httpx.AsyncClient(timeout=20.0) as client:
                for monitor in monitors:
                    m_id = monitor['monitor_id']
                    user_id = monitor['user_id']
                    address = monitor['address']
                    network = monitor['network'].lower()
                    remark = (monitor.get('remark') or '').strip()
                    last_ts = monitor['last_tx_timestamp'] or 0

                    try:
                        if network == 'trc20':
                            url = f"https://api.trongrid.io/v1/accounts/{address}/transactions/trc20"
                            headers = {"TRON-PRO-API-KEY": config.TRONGRID_API_KEY}
                            resp = await client.get(url, headers=headers)
                            data = resp.json()

                            if data.get('success') and 'data' in data:
                                for tx in data['data']:
                                    if tx.get('token_info', {}).get('address') != 'TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t':
                                        continue

                                    ts = tx['block_timestamp']
                                    if ts <= last_ts:
                                        continue

                                    value = float(tx['value']) / (10**6)
                                    to_addr = tx['to']
                                    from_addr = tx['from']
                                    tx_id = tx['transaction_id']

                                    incoming = to_addr == address
                                    direction_label = "📥 转入" if incoming else "📤 转出"
                                    direction_code = 'in' if incoming else 'out'
                                    counterparty = from_addr if incoming else to_addr

                                    risk_level = monitor.get('risk_level') or 'normal'
                                    risk_note = monitor.get('risk_note')
                                    if incoming:
                                        risk_level, risk_note = await _detect_trc20_energy_risk(client, address)
                                        await database.update_monitor_risk(m_id, risk_level, risk_note)

                                    risk_block = ""
                                    if risk_level == 'high' and risk_note:
                                        risk_block = f"\n\n⚠️ <b>高风险提示</b>\n{risk_note}"

                                    remark_block = f"\n备注: <code>{remark}</code>" if remark else ""
                                    msg = (
                                        f"🚨 <b>您的钱包发生 USDT 变动！</b>\n\n"
                                        f"网络: <code>TRC20 (TRON)</code>\n"
                                        f"方向: {direction_label}\n"
                                        f"金额: <code>{value:.2f} USDT</code>\n"
                                        f"对方: <code>{counterparty}</code>"
                                        f"{remark_block}\n"
                                        f"哈希: <a href='https://tronscan.org/#/transaction/{tx_id}'>{tx_id[:8]}...</a>"
                                        f"{risk_block}"
                                    )
                                    await safe_send_message(None, user_id, msg, parse_mode="HTML")

                                    await database.save_monitor_transaction(
                                        tx_id=tx_id,
                                        monitor_id=m_id,
                                        user_id=user_id,
                                        address=address,
                                        network='TRC20',
                                        direction=direction_code,
                                        amount=value,
                                        token_symbol='USDT',
                                        tx_timestamp=ts,
                                        counterparty=counterparty,
                                        risk_level=risk_level,
                                        risk_note=risk_note,
                                    )

                                    from web_admin import notify_web_clients
                                    await notify_web_clients("monitor_tx", {
                                        "user_id": user_id,
                                        "address": address,
                                        "network": network.upper(),
                                        "direction": direction_label,
                                        "value": f"{value:.2f}",
                                        "tx_id": tx_id,
                                        "remark": remark,
                                        "risk_level": risk_level,
                                        "risk_note": risk_note,
                                    })

                                    await database.update_monitor_timestamp(m_id, ts)
                                    last_ts = ts
                                    logger.info(f"💰 [ChainMonitor] {network.upper()} 变动通知已发送: {user_id}")

                        elif network == 'bsc':
                            url = (
                                f"https://api.bscscan.com/api?module=account&action=tokentx"
                                f"&contractaddress=0x55d398326f99059fF775485246999027B3197955"
                                f"&address={address}&sort=desc&apikey={config.BSCSCAN_API_KEY}"
                            )
                            resp = await client.get(url)
                            data = resp.json()

                            if data.get('status') == '1' and 'result' in data:
                                for tx in data['result']:
                                    ts = int(tx['timeStamp']) * 1000
                                    if ts <= last_ts:
                                        continue

                                    value = float(tx['value']) / (10**18)
                                    to_addr = tx['to']
                                    from_addr = tx['from']
                                    tx_hash = tx['hash']

                                    incoming = to_addr.lower() == address.lower()
                                    direction_label = "📥 转入" if incoming else "📤 转出"
                                    direction_code = 'in' if incoming else 'out'
                                    counterparty = from_addr if incoming else to_addr
                                    remark_block = f"\n备注: <code>{remark}</code>" if remark else ""

                                    msg = (
                                        f"🚨 <b>您的钱包发生 USDT 变动！</b>\n\n"
                                        f"网络: <code>BEP20 (BSC)</code>\n"
                                        f"方向: {direction_label}\n"
                                        f"金额: <code>{value:.2f} USDT</code>\n"
                                        f"对方: <code>{counterparty}</code>"
                                        f"{remark_block}\n"
                                        f"哈希: <a href='https://bscscan.com/tx/{tx_hash}'>{tx_hash[:8]}...</a>"
                                    )
                                    await safe_send_message(None, user_id, msg, parse_mode="HTML")

                                    await database.save_monitor_transaction(
                                        tx_id=tx_hash,
                                        monitor_id=m_id,
                                        user_id=user_id,
                                        address=address,
                                        network='BSC',
                                        direction=direction_code,
                                        amount=value,
                                        token_symbol='USDT',
                                        tx_timestamp=ts,
                                        counterparty=counterparty,
                                        risk_level='normal',
                                        risk_note=None,
                                    )

                                    from web_admin import notify_web_clients
                                    await notify_web_clients("monitor_tx", {
                                        "user_id": user_id,
                                        "address": address,
                                        "network": network.upper(),
                                        "direction": direction_label,
                                        "value": f"{value:.2f}",
                                        "tx_id": tx_hash,
                                        "remark": remark,
                                        "risk_level": 'normal',
                                        "risk_note": None,
                                    })

                                    await database.update_monitor_timestamp(m_id, ts)
                                    last_ts = ts
                                    logger.info(f"💰 [ChainMonitor] {network.upper()} 变动通知已发送: {user_id}")

                    except Exception as e:
                        print(f"[ERROR - ChainMonitor] 监控地址 {address} ({network}) 失败: {e}")

            await asyncio.sleep(20)

        except Exception as e:
            print(f"[ERROR - ChainMonitor] 循环异常: {e}")
            await asyncio.sleep(60)
