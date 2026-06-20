# -*- coding: utf-8 -*-
"""
订阅生命周期 Service - 套餐激活、支付确认、到期降级
"""
from typing import Optional, Dict, Any, List
from datetime import datetime, timedelta

from backend.repositories import merchant_repo, subscription_repo, payment_repo
from backend.app.services.feature_gate_service import get_feature_gate_service
from backend.app.services.audit_service import get_audit_service
from backend.app.services.merchant_notification_service import (
    notify_subscription_activated,
    notify_subscription_downgraded,
    notify_payment_confirmed,
    notify_payment_rejected,
)


class SubscriptionLifecycleService:
    def _allocate_unique_amount(self, base_amount_usdt: float, open_orders: List[Dict[str, Any]]) -> Dict[str, float]:
        base_amount_usdt = round(float(base_amount_usdt or 0), 3)
        used_offsets = {
            round(float(row.get('amount_offset_usdt') or 0), 3)
            for row in open_orders
        }
        step = 0.001
        offset = 0.0
        while round(offset, 3) in used_offsets:
            offset = round(offset + step, 3)
        return {
            'base_amount_usdt': base_amount_usdt,
            'amount_offset_usdt': offset,
            'amount_usdt': round(base_amount_usdt + offset, 3),
        }

    async def create_payment_order(self, merchant_id: int, plan_code: str) -> Dict[str, Any]:
        plan = await subscription_repo.get_plan(plan_code)
        if not plan:
            raise ValueError('plan_not_found')

        payment_config = await payment_repo.get_active_platform_payment_config('TRON')
        payment_address = (payment_config or {}).get('wallet_address')
        if not payment_address:
            raise ValueError('payment_config_not_found')

        open_orders = await payment_repo.list_open_orders_for_matching('TRON', payment_address)
        amount_meta = self._allocate_unique_amount(float(plan.get('price_usdt') or 0), open_orders)
        expire_at = (datetime.utcnow() + timedelta(minutes=30)).strftime('%Y-%m-%d %H:%M:%S')
        order_id = await payment_repo.create_payment_order(
            merchant_id=merchant_id,
            plan_code=plan_code,
            base_amount_usdt=amount_meta['base_amount_usdt'],
            amount_offset_usdt=amount_meta['amount_offset_usdt'],
            amount_usdt=amount_meta['amount_usdt'],
            chain_type='TRON',
            payment_address=payment_address,
            payment_qr_image_path=(payment_config or {}).get('wallet_qr_image_path'),
            expire_at=expire_at,
        )
        return await payment_repo.get_payment_order(order_id) or {}

    async def get_latest_open_order(self, merchant_id: int) -> Optional[Dict[str, Any]]:
        return await payment_repo.get_latest_open_payment_order(merchant_id)

    async def mark_order_paid(self, order_id: int) -> bool:
        return await payment_repo.mark_payment_order_paid(order_id)

    async def submit_tx_hash(self, order_id: int, tx_hash: str) -> bool:
        return await payment_repo.submit_payment_tx_hash(order_id, tx_hash)

    async def confirm_payment(self, order_id: int) -> Dict[str, Any]:
        order = await payment_repo.get_payment_order(order_id)
        if not order:
            raise ValueError('payment_order_not_found')

        now_str = datetime.utcnow().strftime('%Y-%m-%d %H:%M:%S')
        await payment_repo.update_payment_status(order_id, 'confirmed', confirmed_at=now_str)

        plan = await subscription_repo.get_plan(str(order['plan_code']))
        if not plan:
            raise ValueError('plan_not_found')

        duration_days = int(plan.get('duration_days') or 30)
        end_at = (datetime.utcnow() + timedelta(days=duration_days)).strftime('%Y-%m-%d %H:%M:%S')
        subscription_id = await subscription_repo.create_subscription(
            merchant_id=int(order['merchant_id']),
            plan_code=str(order['plan_code']),
            status='active',
            start_at=now_str,
            end_at=end_at,
            payment_order_id=order_id,
        )
        await merchant_repo.update_merchant_plan(
            merchant_id=int(order['merchant_id']),
            plan_type=str(order['plan_code']),
            subscription_status='active',
            subscription_expires_at=end_at,
            status='active_paid',
        )

        merchant = await merchant_repo.get_merchant_by_id(int(order['merchant_id']))
        if merchant:
            await get_feature_gate_service().rebuild_for_tenant(
                str(merchant['tenant_id']),
                merchant_id=int(merchant['id']),
                plan_code=str(order['plan_code']),
                source='payment_confirm',
            )
            await get_audit_service().log(
                tenant_id=str(merchant['tenant_id']),
                merchant_id=int(merchant['id']),
                operator_role='platform_admin',
                action='subscription_activated',
                resource_type='subscription',
                resource_id=str(subscription_id),
                payload={
                    'plan_code': order['plan_code'],
                    'payment_order_id': order_id,
                    'base_amount_usdt': order.get('base_amount_usdt'),
                    'amount_usdt': order.get('amount_usdt'),
                    'end_at': end_at,
                },
            )
            await notify_payment_confirmed(int(order['merchant_id']), order_id, str(order['plan_code']))
            await notify_subscription_activated(int(merchant['id']), str(order['plan_code']), end_at)
            from backend.app.services import get_merchant_runtime_manager
            get_merchant_runtime_manager().request_reconcile(str(merchant.get('tenant_id') or ''))

        return await payment_repo.get_payment_order(order_id) or {}

    async def reject_payment(self, order_id: int) -> bool:
        order = await payment_repo.get_payment_order(order_id)
        if not order:
            raise ValueError('payment_order_not_found')
        ok = await payment_repo.update_payment_status(order_id, 'rejected')
        if ok:
            await notify_payment_rejected(int(order['merchant_id']), order_id)
        return ok

    async def downgrade_expired_subscriptions(self) -> List[int]:
        expired = await subscription_repo.list_expired_subscriptions()
        downgraded: List[int] = []
        for row in expired:
            subscription_id = int(row['id'])
            merchant_id = int(row['merchant_id'])
            await subscription_repo.update_subscription_status(subscription_id, 'expired')
            await merchant_repo.update_merchant_plan(
                merchant_id=merchant_id,
                plan_type='free',
                subscription_status='expired',
                subscription_expires_at=row.get('end_at'),
                status='active_free',
            )
            merchant = await merchant_repo.get_merchant_by_id(merchant_id)
            if merchant:
                await get_feature_gate_service().rebuild_for_tenant(
                    str(merchant['tenant_id']),
                    merchant_id=merchant_id,
                    plan_code='free',
                    source='expiry_downgrade',
                )
                await get_audit_service().log(
                    tenant_id=str(merchant['tenant_id']),
                    merchant_id=merchant_id,
                    operator_role='system',
                    action='subscription_downgraded',
                    resource_type='subscription',
                    resource_id=str(subscription_id),
                    payload={'from_plan': row.get('plan_code'), 'to_plan': 'free'},
                )
                await notify_subscription_downgraded(merchant_id, str(row.get('plan_code') or 'pro'))
                from backend.app.services import get_merchant_runtime_manager
                get_merchant_runtime_manager().request_reconcile(str(merchant.get('tenant_id') or ''))
            downgraded.append(subscription_id)
        return downgraded


_subscription_lifecycle_service: Optional[SubscriptionLifecycleService] = None


def get_subscription_lifecycle_service() -> SubscriptionLifecycleService:
    global _subscription_lifecycle_service
    if _subscription_lifecycle_service is None:
        _subscription_lifecycle_service = SubscriptionLifecycleService()
    return _subscription_lifecycle_service
