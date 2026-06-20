# -*- coding: utf-8 -*-
"""
定时任务调度器 - 基于 asyncio 的商户系统自动化调度

任务列表:
- 到期订阅扫描: 每小时执行 downgrade_expired_subscriptions()
- AI 配额日重置: 每天 00:00 UTC 执行 reset_daily_ai_quota()
- AI 配额月重置: 每月1日 00:00 UTC 执行 reset_monthly_ai_quota()
- 过期订单清理: 每30分钟将超时未支付订单标记为 expired
- 链上交易自动匹配: 每5分钟扫描待支付订单，匹配链上交易并确认
"""
import asyncio
import logging
from datetime import datetime, timedelta, timezone
from typing import List, Optional, Dict, Any

import httpx

from backend.app.services.subscription_lifecycle_service import get_subscription_lifecycle_service
from backend.repositories import quota_repo, payment_repo

logger = logging.getLogger(__name__)


def _seconds_until_next_hour() -> float:
    """计算距离下一个整点的秒数"""
    now = datetime.now(timezone.utc)
    seconds = (60 - now.minute) * 60 - now.second - now.microsecond / 1_000_000
    return max(0.0, seconds)


def _seconds_until_midnight() -> float:
    """计算距离下一个 UTC 00:00 的秒数"""
    now = datetime.now(timezone.utc)
    midnight_today = now.replace(hour=0, minute=0, second=0, microsecond=0)
    next_midnight = midnight_today + timedelta(days=1)
    return max(0.0, (next_midnight - now).total_seconds())


def _seconds_until_next_half_hour() -> float:
    """计算距离下一个半点（xx:00 或 xx:30）的秒数"""
    now = datetime.now(timezone.utc)
    if now.minute < 30:
        target = now.replace(minute=30, second=0, microsecond=0)
    else:
        target = now.replace(minute=0, second=0, microsecond=0) + timedelta(hours=1)
    return max(0.0, (target - now).total_seconds())


class SchedulerService:
    """基于 asyncio 的定时任务调度器"""

    def __init__(self) -> None:
        self._tasks: List[asyncio.Task] = []
        self._running = False

    async def start(self) -> None:
        """启动所有定时任务"""
        if self._running:
            logger.warning("scheduler service already running, skip")
            return
        self._running = True

        self._tasks = [
            asyncio.create_task(self._loop_subscription_expiry(), name="scheduler-subscription-expiry"),
            asyncio.create_task(self._loop_daily_quota_reset(), name="scheduler-daily-quota-reset"),
            asyncio.create_task(self._loop_monthly_quota_reset(), name="scheduler-monthly-quota-reset"),
            asyncio.create_task(self._loop_stale_order_expiry(), name="scheduler-stale-order-expiry"),
            asyncio.create_task(self._loop_auto_match_blockchain_payments(), name="scheduler-auto-match-blockchain"),
            asyncio.create_task(self._loop_auto_approval(), name="scheduler-auto-approval"),
        ]
        logger.info("scheduler service started with %d tasks", len(self._tasks))

    async def stop(self) -> None:
        """停止所有定时任务"""
        if not self._running:
            return
        self._running = False

        for task in self._tasks:
            task.cancel()
        for task in self._tasks:
            try:
                await task
            except asyncio.CancelledError:
                pass
            except Exception as e:
                logger.warning("scheduler task stop error: %s", e)
        self._tasks.clear()
        logger.info("scheduler service stopped")

    # ── 任务 1: 到期订阅扫描 (每小时) ──────────────────────────────

    async def _loop_subscription_expiry(self) -> None:
        """每小时扫描到期订阅并执行降级"""
        delay = _seconds_until_next_hour()
        logger.info("[订阅扫描] 首次执行将在 %.0f 秒后", delay)
        await asyncio.sleep(delay)

        while self._running:
            try:
                logger.info("[订阅扫描] 开始执行到期订阅降级")
                service = get_subscription_lifecycle_service()
                downgraded = await service.downgrade_expired_subscriptions()
                logger.info("[订阅扫描] 执行完成，降级 %d 个订阅", len(downgraded))
            except Exception as e:
                logger.error("[订阅扫描] 执行异常: %s", e, exc_info=True)

            await asyncio.sleep(3600)

    # ── 任务 2: AI 配额日重置 (每天 00:00 UTC) ────────────────────

    async def _loop_daily_quota_reset(self) -> None:
        """每天 00:00 UTC 重置所有商户的日配额"""
        delay = _seconds_until_midnight()
        logger.info("[日配额重置] 首次执行将在 %.0f 秒后", delay)
        await asyncio.sleep(delay)

        while self._running:
            try:
                logger.info("[日配额重置] 开始执行每日 AI 配额重置")
                await self._reset_all_daily_quota()
                logger.info("[日配额重置] 执行完成")
            except Exception as e:
                logger.error("[日配额重置] 执行异常: %s", e, exc_info=True)

            await asyncio.sleep(86400)

    # ── 任务 3: AI 配额月重置 (每月1日 00:00 UTC) ─────────────────

    async def _loop_monthly_quota_reset(self) -> None:
        """每月1日 00:00 UTC 重置所有商户的月配额"""
        delay = _seconds_until_midnight()
        await asyncio.sleep(delay)

        while self._running:
            now = datetime.now(timezone.utc)
            if now.day == 1:
                try:
                    logger.info("[月配额重置] 开始执行每月 AI 配额重置")
                    await self._reset_all_monthly_quota()
                    logger.info("[月配额重置] 执行完成")
                except Exception as e:
                    logger.error("[月配额重置] 执行异常: %s", e, exc_info=True)
            else:
                logger.debug("[月配额重置] 当前日期 %d 号，非1日，跳过", now.day)

            await asyncio.sleep(86400)

    # ── 任务 4: 过期订单清理 (每30分钟) ───────────────────────────

    async def _loop_stale_order_expiry(self) -> None:
        """每30分钟将超时未支付的订单标记为 expired"""
        delay = _seconds_until_next_half_hour()
        logger.info("[订单过期] 首次执行将在 %.0f 秒后", delay)
        await asyncio.sleep(delay)

        while self._running:
            try:
                logger.info("[订单过期] 开始执行过期订单清理")
                count = await payment_repo.expire_stale_payment_orders()
                logger.info("[订单过期] 执行完成，标记 %d 个订单为 expired", count)
            except Exception as e:
                logger.error("[订单过期] 执行异常: %s", e, exc_info=True)

            await asyncio.sleep(1800)

    # ── 内部辅助: 批量重置配额 ────────────────────────────────────

    async def _reset_all_daily_quota(self) -> None:
        """遍历所有商户，逐个重置日配额"""
        merchants = await quota_repo.list_merchants_needing_quota_reset(limit=500)
        success = 0
        fail = 0
        for m in merchants:
            tenant_id = m.get('tenant_id', '')
            if not tenant_id:
                continue
            try:
                ok = await quota_repo.reset_daily_ai_quota(tenant_id)
                if ok:
                    success += 1
                else:
                    fail += 1
            except Exception as e:
                fail += 1
                logger.warning("[日配额重置] tenant=%s 重置失败: %s", tenant_id, e)
        logger.info("[日配额重置] 处理完成: 成功=%d 失败=%d 总计=%d", success, fail, len(merchants))

    async def _reset_all_monthly_quota(self) -> None:
        """遍历所有商户，逐个重置月配额"""
        merchants = await quota_repo.list_merchants_needing_quota_reset(limit=500)
        success = 0
        fail = 0
        for m in merchants:
            tenant_id = m.get('tenant_id', '')
            if not tenant_id:
                continue
            try:
                ok = await quota_repo.reset_monthly_ai_quota(tenant_id)
                if ok:
                    success += 1
                else:
                    fail += 1
            except Exception as e:
                fail += 1
                logger.warning("[月配额重置] tenant=%s 重置失败: %s", tenant_id, e)
        logger.info("[月配额重置] 处理完成: 成功=%d 失败=%d 总计=%d", success, fail, len(merchants))

    # ── 任务 5: 链上交易自动匹配 (每5分钟) ────────────────────────

    async def _loop_auto_match_blockchain_payments(self) -> None:
        """每5分钟扫描待支付订单，匹配链上交易并确认"""
        logger.info("[链上匹配] 首次执行将在 30 秒后")
        await asyncio.sleep(30)

        while self._running:
            try:
                matched = await self._auto_match_blockchain_payments()
                logger.info("[链上匹配] 执行完成，匹配 %d 笔订单", matched)
            except Exception as e:
                logger.error("[链上匹配] 执行异常: %s", e, exc_info=True)

            await asyncio.sleep(300)  # 5分钟

    async def _auto_match_blockchain_payments(self) -> int:
        """
        扫描所有活跃的平台支付配置，查询链上交易，
        与待支付订单通过唯一金额 (amount_usdt) 进行匹配。
        匹配成功后自动确认订单。
        """
        matched_count = 0

        # 遍历所有链类型
        for chain_type in ('TRON', 'BSC'):
            try:
                config_row = await payment_repo.get_active_platform_payment_config(chain_type)
            except Exception:
                config_row = None
            if not config_row:
                continue

            payment_address = (config_row.get('wallet_address') or '').strip()
            if not payment_address:
                continue

            # 获取该链/地址下的待匹配订单
            open_orders = await payment_repo.list_open_orders_for_matching(
                chain_type=chain_type,
                payment_address=payment_address,
            )
            if not open_orders:
                continue

            # 查询链上交易
            now_ms = int(datetime.now(timezone.utc).timestamp() * 1000)
            start_ms = now_ms - 30 * 24 * 3600 * 1000  # 最近30天

            try:
                transactions = await self._fetch_chain_history(
                    chain_type=chain_type,
                    address=payment_address,
                    start_ms=start_ms,
                    end_ms=now_ms,
                )
            except Exception as e:
                logger.error("[链上匹配] 查询 %s 链上交易失败: %s", chain_type, e)
                continue

            if not transactions:
                continue

            # 构建 amount_key -> [tx] 索引（仅入账方向，使用格式化字符串避免浮点精度问题）
            amount_to_txs: Dict[str, List[Dict[str, Any]]] = {}
            for tx in transactions:
                if tx.get('direction') != 'in':
                    continue
                amount = float(tx.get('amount', 0))
                if amount <= 0:
                    continue
                amount_key = f"{amount:.6f}"
                amount_to_txs.setdefault(amount_key, []).append(tx)

            # 已使用的 tx_id 集合，防止同一笔交易匹配多个订单
            used_tx_ids: set = set()

            # 匹配订单
            for order in open_orders:
                order_id = order.get('id')
                expected_amount = float(order.get('amount_usdt', 0))
                if expected_amount <= 0:
                    continue
                # 当前订单状态
                current_status = str(order.get('status', ''))

                # 已有 tx_hash 的订单跳过匹配（避免覆盖）
                if order.get('tx_hash'):
                    continue

                # 精确匹配金额（使用格式化字符串键避免浮点精度问题）
                expected_key = f"{expected_amount:.6f}"
                matched_txs = amount_to_txs.get(expected_key, [])
                if not matched_txs:
                    continue

                # 取未使用的最新一笔匹配交易
                best_tx = None
                for tx in matched_txs:
                    tx_id_candidate = str(tx.get('tx_id') or '').strip()
                    if tx_id_candidate and tx_id_candidate not in used_tx_ids:
                        best_tx = tx
                        break
                if not best_tx:
                    continue

                tx_id = str(best_tx.get('tx_id') or '').strip()
                used_tx_ids.add(tx_id)

                # 更新订单：仅记录 tx_hash（不改变状态），然后标记为 paid_pending_review
                try:
                    if tx_id:
                        await payment_repo.update_payment_tx_hash_only(order_id, tx_id)
                    if current_status == 'pending':
                        await payment_repo.mark_payment_order_paid(order_id)
                    matched_count += 1
                    logger.info(
                        "[链上匹配] 订单 %d 匹配成功: amount=%.6f tx=%s",
                        order_id, expected_amount, tx_id,
                    )
                except Exception as e:
                    logger.error("[链上匹配] 订单 %d 状态更新失败: %s", order_id, e)

        return matched_count

    async def _fetch_chain_history(
        self,
        chain_type: str,
        address: str,
        start_ms: int,
        end_ms: int,
    ) -> List[Dict[str, Any]]:
        """调用链上API查询 TRC20/BEP20 USDT 交易记录"""
        import config as _config

        if chain_type == 'TRON':
            return await self._fetch_trc20_history(_config, address, start_ms, end_ms)
        elif chain_type == 'BSC':
            return await self._fetch_bsc_history(_config, address, start_ms, end_ms)
        return []

    @staticmethod
    async def _fetch_trc20_history(
        _config,
        address: str,
        start_ms: int,
        end_ms: int,
    ) -> List[Dict[str, Any]]:
        """查询 TRON 链 TRC20 USDT 交易记录"""
        api_key = getattr(_config, 'TRONGRID_API_KEY', '') or ''
        headers = {"TRON-PRO-API-KEY": api_key} if api_key else {}
        url = f"https://api.trongrid.io/v1/accounts/{address}/transactions/trc20"

        out: List[Dict[str, Any]] = []
        fingerprint: Optional[str] = None
        max_pages = 8

        async with httpx.AsyncClient(timeout=15) as client:
            for _ in range(max_pages):
                params: Dict[str, Any] = {
                    "contract_address": "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t",
                    "limit": 200,
                    "only_confirmed": "true",
                    "order_by": "block_timestamp,desc",
                }
                if fingerprint:
                    params["fingerprint"] = fingerprint

                try:
                    resp = await client.get(url, params=params, headers=headers)
                    if resp.status_code >= 400:
                        break
                    payload = resp.json() if resp.content else {}
                except Exception:
                    break

                rows = payload.get("data") or []
                if not isinstance(rows, list) or not rows:
                    break

                stop_due_to_old = False
                for tx in rows:
                    try:
                        ts = int(tx.get("block_timestamp") or 0)
                    except Exception:
                        ts = 0
                    if ts <= 0:
                        continue
                    if ts < start_ms:
                        stop_due_to_old = True
                        continue
                    if ts >= end_ms:
                        continue

                    raw_from = str(tx.get("from") or "")
                    raw_to = str(tx.get("to") or "")
                    direction = "in" if raw_to == address else "out"
                    counterparty = raw_from if direction == "in" else raw_to

                    amount_raw = 0.0
                    try:
                        amount_raw = float(tx.get("value", 0))
                    except Exception:
                        pass
                    amount = amount_raw / 1e6

                    out.append({
                        "tx_id": str(tx.get("transaction_id") or "").strip(),
                        "direction": direction,
                        "amount": amount,
                        "counterparty": counterparty,
                        "tx_timestamp": ts,
                    })

                if stop_due_to_old:
                    break
                fingerprint = payload.get("meta", {}).get("fingerprint")
                if not fingerprint:
                    break

        return [x for x in out if x.get("tx_id")]

    @staticmethod
    async def _fetch_bsc_history(
        _config,
        address: str,
        start_ms: int,
        end_ms: int,
    ) -> List[Dict[str, Any]]:
        """查询 BSC 链 BEP20 USDT 交易记录"""
        address = (address or "").strip().lower()
        if not address:
            return []

        api_key = getattr(_config, 'BSCSCAN_API_KEY', '') or ""
        out: List[Dict[str, Any]] = []
        max_pages = 6
        offset = 200

        async with httpx.AsyncClient(timeout=15) as client:
            for page in range(1, max_pages + 1):
                params = {
                    "module": "account",
                    "action": "tokentx",
                    "contractaddress": "0x55d398326f99059fF775485246999027B3197955",
                    "address": address,
                    "sort": "desc",
                    "page": page,
                    "offset": offset,
                    "apikey": api_key,
                }
                try:
                    resp = await client.get("https://api.bscscan.com/api", params=params)
                    if resp.status_code >= 400:
                        break
                    payload = resp.json() if resp.content else {}
                except Exception:
                    break

                result = payload.get("result") or []
                if not isinstance(result, list) or not result:
                    break

                stop_due_to_old = False
                for tx in result:
                    try:
                        ts = int(tx.get("timeStamp") or 0) * 1000
                    except Exception:
                        ts = 0
                    if ts <= 0:
                        continue
                    if ts < start_ms:
                        stop_due_to_old = True
                        continue
                    if ts >= end_ms:
                        continue

                    raw_from = str(tx.get("from") or "").lower()
                    raw_to = str(tx.get("to") or "").lower()
                    direction = "in" if raw_to == address else "out"
                    counterparty = raw_from if direction == "in" else raw_to

                    decimals = 18
                    try:
                        decimals = int(tx.get("tokenDecimal") or 18)
                    except Exception:
                        pass
                    amount_raw = 0.0
                    try:
                        amount_raw = float(tx.get("value", 0))
                    except Exception:
                        pass
                    amount = amount_raw / (10 ** decimals) if decimals > 0 else amount_raw

                    out.append({
                        "tx_id": str(tx.get("hash") or "").strip(),
                        "direction": direction,
                        "amount": amount,
                        "counterparty": counterparty,
                        "tx_timestamp": ts,
                    })

                if stop_due_to_old:
                    break

        return [x for x in out if x.get("tx_id")]

    # ── 任务 6: 自动审批商户 (每10分钟) ──────────────────────────────

    async def _loop_auto_approval(self) -> None:
        """每10分钟自动审批符合条件的商户"""
        while self._running:
            try:
                await asyncio.sleep(600)  # 10分钟
                logger.info("[自动审批] 开始执行...")
                
                from backend.app.services.auto_approval_service import get_auto_approval_service
                approved = await get_auto_approval_service().auto_approve_pending_merchants()
                
                if approved:
                    logger.info("[自动审批] 审批成功 %d 个商户", len(approved))
                else:
                    logger.debug("[自动审批] 无待审批商户")
                    
            except asyncio.CancelledError:
                raise
            except Exception as e:
                logger.error("[自动审批] 执行失败: %s", e, exc_info=True)


_scheduler_service: Optional[SchedulerService] = None


def get_scheduler_service() -> SchedulerService:
    """获取调度器单例"""
    global _scheduler_service
    if _scheduler_service is None:
        _scheduler_service = SchedulerService()
    return _scheduler_service
