# -*- coding: utf-8 -*-
"""
商户 Bot 运行时管理器。

职责：
- 从 merchants + tenant secret 中发现可运行的商户 Bot
- 为每个商户维护独立 Application
- 监听 runtime/config/feature 版本变化并重建运行时
"""
from __future__ import annotations

import asyncio
import hashlib
import logging
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any, Dict, Optional

from telegram.ext import Application

from backend.repositories import merchant_repo
from services.secret_manager import SecretManager
from handlers.merchant_runtime import register_merchant_runtime_handlers, setup_merchant_runtime_entrypoints

logger = logging.getLogger(__name__)


@dataclass
class MerchantRuntimeState:
    tenant_id: str
    merchant_id: int
    owner_tg_user_id: int
    bot_username: str
    bot_id: Optional[int]
    runtime_version: int
    config_version: int
    feature_version: int
    token_fingerprint: str
    application: Application
    started_at: datetime = field(default_factory=datetime.utcnow)
    last_error: str = ""
    running: bool = True


class MerchantRuntimeManager:
    # 商户运行时数量上限，防止内存无限增长
    MAX_MERCHANT_RUNTIMES = 50

    def __init__(self, *, reconcile_interval_seconds: int = 20):
        self._states: Dict[str, MerchantRuntimeState] = {}
        self._reconcile_interval_seconds = max(5, int(reconcile_interval_seconds))
        self._lock = asyncio.Lock()
        self._wake_event = asyncio.Event()
        self._runner_task: Optional[asyncio.Task] = None
        self._started = False

    async def start(self) -> None:
        if self._started:
            return
        self._started = True
        self._runner_task = asyncio.create_task(self._runner(), name="merchant-runtime-manager")
        self.request_reconcile()
        logger.info("merchant runtime manager started")

    async def stop(self) -> None:
        if not self._started:
            return
        self._started = False
        if self._runner_task:
            self._runner_task.cancel()
            try:
                await self._runner_task
            except asyncio.CancelledError:
                pass
            self._runner_task = None
        await self._stop_all("manager_stop")
        logger.info("merchant runtime manager stopped")

    def request_reconcile(self, tenant_id: Optional[str] = None) -> None:
        if tenant_id:
            logger.info("merchant runtime reconcile requested for tenant=%s", tenant_id)
        self._wake_event.set()

    def get_runtime_snapshot(self, tenant_id: str) -> Dict[str, Any]:
        state = self._states.get(str(tenant_id or '').strip())
        if not state:
            return {'tenant_id': tenant_id, 'status': 'stopped'}
        return {
            'tenant_id': state.tenant_id,
            'merchant_id': state.merchant_id,
            'owner_tg_user_id': state.owner_tg_user_id,
            'bot_username': state.bot_username,
            'bot_id': state.bot_id,
            'runtime_version': state.runtime_version,
            'config_version': state.config_version,
            'feature_version': state.feature_version,
            'started_at': state.started_at.isoformat(),
            'last_error': state.last_error,
            'status': 'running' if state.running else 'stopped',
        }

    def list_runtime_snapshots(self) -> list[Dict[str, Any]]:
        return [self.get_runtime_snapshot(tenant_id) for tenant_id in sorted(self._states.keys())]

    async def _runner(self) -> None:
        try:
            while self._started:
                try:
                    await self._reconcile_once()
                except Exception as e:
                    logger.error("merchant runtime reconcile failed: %s", e, exc_info=True)
                self._wake_event.clear()
                try:
                    await asyncio.wait_for(self._wake_event.wait(), timeout=self._reconcile_interval_seconds)
                except asyncio.TimeoutError:
                    pass
        except asyncio.CancelledError:
            raise

    async def _reconcile_once(self) -> None:
        async with self._lock:
            merchants = await merchant_repo.list_merchants(limit=500)
            desired_tenant_ids = set()

            for merchant in merchants:
                tenant_id = str(merchant.get('tenant_id') or '').strip()
                if not tenant_id:
                    continue
                desired_tenant_ids.add(tenant_id)

                token = await SecretManager.get_config('TELEGRAM_BOT_TOKEN', tenant_id=tenant_id, default='')
                token = str(token or '').strip()
                should_run = self._should_run_runtime(merchant, token)

                if not should_run:
                    if tenant_id in self._states:
                        await self._stop_runtime(tenant_id, reason='not_runnable')
                    continue

                fingerprint = self._fingerprint(token)
                current = self._states.get(tenant_id)
                if current and not self._needs_restart(current, merchant, fingerprint):
                    continue

                if current:
                    await self._stop_runtime(tenant_id, reason='version_or_token_changed')

                await self._start_runtime(merchant, token, fingerprint)

            for tenant_id in list(self._states.keys()):
                if tenant_id not in desired_tenant_ids:
                    await self._stop_runtime(tenant_id, reason='merchant_removed')

    @staticmethod
    def _normalize_status(merchant: Dict[str, Any]) -> str:
        status = str(merchant.get('status') or '').strip().lower()
        plan_type = str(merchant.get('plan_type') or '').strip().lower()
        subscription_status = str(merchant.get('subscription_status') or '').strip().lower()

        if status in {'disabled', 'suspended'}:
            return status
        if status in {'active_paid', 'active_pro'}:
            return 'active_pro'
        if status in {'active_free', 'active'}:
            return 'active_free' if plan_type == 'free' else 'active_pro'
        if plan_type in {'pro', 'paid'} or subscription_status in {'paid', 'active_subscription'}:
            return 'active_pro'
        if plan_type == 'free' or subscription_status in {'free', 'active', 'expired'}:
            return 'active_free'
        if merchant.get('onboarding_completed'):
            return 'active_free'
        return status or 'unknown'

    def _should_run_runtime(self, merchant: Dict[str, Any], token: str) -> bool:
        if not str(token or '').strip():
            return False
        if not int(merchant.get('owner_tg_user_id') or 0):
            return False
        status = self._normalize_status(merchant)
        subscription_status = str(merchant.get('subscription_status') or '').strip().lower()
        if status in {'disabled', 'suspended'}:
            return False
        if status not in {'active_free', 'active_pro'}:
            return False
        if subscription_status in {'inactive', 'blocked'}:
            return False
        return True

    def _needs_restart(self, state: MerchantRuntimeState, merchant: Dict[str, Any], fingerprint: str) -> bool:
        return any([
            state.runtime_version != int(merchant.get('runtime_version') or 0),
            state.config_version != int(merchant.get('config_version') or 0),
            state.feature_version != int(merchant.get('feature_version') or 0),
            state.token_fingerprint != fingerprint,
            not state.running,
        ])

    async def _start_runtime(self, merchant: Dict[str, Any], token: str, fingerprint: str) -> None:
        tenant_id = str(merchant.get('tenant_id') or '').strip()
        merchant_id = int(merchant.get('id'))
        owner_tg_user_id = int(merchant.get('owner_tg_user_id') or 0)
        bot_username = str(merchant.get('bot_username') or '').strip()
        bot_id = merchant.get('bot_id')

        # 商户运行时数量上限保护
        if len(self._states) >= self.MAX_MERCHANT_RUNTIMES:
            logger.warning(
                "skipping merchant runtime tenant=%s: reached max limit %d",
                tenant_id, self.MAX_MERCHANT_RUNTIMES,
            )
            return

        logger.info("starting merchant runtime tenant=%s merchant_id=%s bot_username=%s", tenant_id, merchant_id, bot_username or '-')

        application = Application.builder().token(token).build()
        application.bot_data['merchant_runtime'] = {
            'tenant_id': tenant_id,
            'merchant_id': merchant_id,
            'owner_tg_user_id': owner_tg_user_id,
            'merchant_name': str(merchant.get('merchant_name') or '').strip(),
            'plan_type': str(merchant.get('plan_type') or 'free').strip() or 'free',
            'subscription_status': str(merchant.get('subscription_status') or 'unknown').strip() or 'unknown',
            'bot_username': bot_username,
            'bot_id': int(bot_id) if bot_id is not None else None,
        }
        register_merchant_runtime_handlers(application)

        await application.initialize()
        await setup_merchant_runtime_entrypoints(application)
        await application.start()
        if application.updater is None:
            raise RuntimeError('merchant runtime updater unavailable')
        await application.updater.start_polling(allowed_updates=None)

        self._states[tenant_id] = MerchantRuntimeState(
            tenant_id=tenant_id,
            merchant_id=merchant_id,
            owner_tg_user_id=owner_tg_user_id,
            bot_username=bot_username,
            bot_id=int(bot_id) if bot_id is not None else None,
            runtime_version=int(merchant.get('runtime_version') or 0),
            config_version=int(merchant.get('config_version') or 0),
            feature_version=int(merchant.get('feature_version') or 0),
            token_fingerprint=fingerprint,
            application=application,
            running=True,
        )
        logger.info("merchant runtime started tenant=%s", tenant_id)

    async def _stop_runtime(self, tenant_id: str, *, reason: str) -> None:
        state = self._states.pop(tenant_id, None)
        if not state:
            return

        logger.info("stopping merchant runtime tenant=%s reason=%s", tenant_id, reason)
        state.running = False
        try:
            if state.application.updater is not None:
                await state.application.updater.stop()
        except Exception:
            logger.debug("merchant runtime updater stop failed tenant=%s", tenant_id, exc_info=True)
        try:
            await state.application.stop()
        except Exception:
            logger.debug("merchant runtime application stop failed tenant=%s", tenant_id, exc_info=True)
        try:
            await state.application.shutdown()
        except Exception:
            logger.debug("merchant runtime shutdown failed tenant=%s", tenant_id, exc_info=True)
        logger.info("merchant runtime stopped tenant=%s", tenant_id)

    async def _stop_all(self, reason: str) -> None:
        for tenant_id in list(self._states.keys()):
            await self._stop_runtime(tenant_id, reason=reason)

    @staticmethod
    def _fingerprint(token: str) -> str:
        return hashlib.sha256(str(token or '').encode('utf-8')).hexdigest()[:16]


_merchant_runtime_manager: Optional[MerchantRuntimeManager] = None


def get_merchant_runtime_manager() -> MerchantRuntimeManager:
    global _merchant_runtime_manager
    if _merchant_runtime_manager is None:
        _merchant_runtime_manager = MerchantRuntimeManager()
    return _merchant_runtime_manager
