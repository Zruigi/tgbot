# -*- coding: utf-8 -*-
"""
配额 Service - 商户 AI 配额校验与计量
"""
from typing import Optional, Dict, Any

from backend.repositories import quota_repo


class QuotaService:
    async def ensure_quota(
        self,
        merchant_id: int,
        tenant_id: str,
        *,
        daily_quota: int = 0,
        monthly_quota: int = 0,
    ) -> Dict[str, Any]:
        current = await quota_repo.get_merchant_ai_quota(tenant_id)
        if current:
            return current
        await quota_repo.upsert_merchant_ai_quota(
            merchant_id=merchant_id,
            tenant_id=tenant_id,
            daily_quota=daily_quota,
            monthly_quota=monthly_quota,
        )
        return await quota_repo.get_merchant_ai_quota(tenant_id) or {}

    async def get_quota(self, tenant_id: str) -> Optional[Dict[str, Any]]:
        return await quota_repo.get_merchant_ai_quota(tenant_id)

    async def check_available(self, tenant_id: str, amount: int = 1) -> Dict[str, Any]:
        quota = await quota_repo.get_merchant_ai_quota(tenant_id)
        if not quota:
            return {'allowed': True, 'reason': 'no_quota', 'quota': None}

        used_daily = int(quota.get('used_daily') or 0)
        used_monthly = int(quota.get('used_monthly') or 0)
        daily_quota = int(quota.get('daily_quota') or 0)
        monthly_quota = int(quota.get('monthly_quota') or 0)
        amount = int(amount or 0)

        if daily_quota > 0 and used_daily + amount > daily_quota:
            return {'allowed': False, 'reason': 'daily_quota_exceeded', 'quota': quota}
        if monthly_quota > 0 and used_monthly + amount > monthly_quota:
            return {'allowed': False, 'reason': 'monthly_quota_exceeded', 'quota': quota}
        return {'allowed': True, 'reason': 'ok', 'quota': quota}

    async def record_usage(self, tenant_id: str, amount: int = 1) -> bool:
        return await quota_repo.increment_ai_quota_usage(
            tenant_id=tenant_id,
            daily_inc=int(amount or 0),
            monthly_inc=int(amount or 0),
        )


_quota_service: Optional[QuotaService] = None


def get_quota_service() -> QuotaService:
    global _quota_service
    if _quota_service is None:
        _quota_service = QuotaService()
    return _quota_service
