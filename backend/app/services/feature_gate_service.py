# -*- coding: utf-8 -*-
"""
功能门控 Service - 根据套餐快照判断商户能力
"""
from typing import Optional, Dict, Any
import json

from backend.repositories import feature_repo, merchant_repo, subscription_repo


class FeatureGateService:
    async def has_feature(self, tenant_id: str, feature_key: str) -> bool:
        feature = await feature_repo.get_merchant_feature(tenant_id, feature_key)
        return bool(feature and feature.get('enabled'))

    async def get_feature_map(self, tenant_id: str) -> Dict[str, bool]:
        rows = await feature_repo.list_merchant_features(tenant_id)
        return {str(row['feature_key']): bool(row.get('enabled')) for row in rows}

    async def rebuild_for_tenant(
        self,
        tenant_id: str,
        *,
        merchant_id: Optional[int] = None,
        plan_code: Optional[str] = None,
        source: str = 'plan',
    ) -> Dict[str, bool]:
        merchant = None
        if merchant_id is not None:
            merchant = await merchant_repo.get_merchant_by_id(merchant_id)
        if merchant is None:
            merchant = await merchant_repo.get_merchant_by_tenant_id(tenant_id)
        if not merchant:
            return {}

        effective_plan_code = plan_code or merchant.get('plan_type') or 'free'
        plan = await subscription_repo.get_plan(effective_plan_code)
        features = self._parse_plan_features(plan)

        await feature_repo.delete_merchant_features(tenant_id)
        for feature_key, enabled in features.items():
            await feature_repo.upsert_merchant_feature(
                merchant_id=int(merchant['id']),
                tenant_id=tenant_id,
                feature_key=feature_key,
                enabled=bool(enabled),
                source=source,
            )

        await merchant_repo.bump_feature_version(int(merchant['id']))
        return {str(k): bool(v) for k, v in features.items()}

    def _parse_plan_features(self, plan: Optional[Dict[str, Any]]) -> Dict[str, bool]:
        if not plan:
            return {}
        raw = plan.get('features_json') or '{}'
        try:
            parsed = json.loads(raw)
        except Exception:
            return {}
        if not isinstance(parsed, dict):
            return {}
        return {str(k): bool(v) for k, v in parsed.items()}


_feature_gate_service: Optional[FeatureGateService] = None


def get_feature_gate_service() -> FeatureGateService:
    global _feature_gate_service
    if _feature_gate_service is None:
        _feature_gate_service = FeatureGateService()
    return _feature_gate_service
