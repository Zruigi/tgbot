# -*- coding: utf-8 -*-
"""
审计 Service - 租户审计日志服务
"""
from typing import Optional, Dict, Any, List
import json

from backend.repositories import audit_repo


class AuditService:
    async def log(
        self,
        tenant_id: str,
        action: str,
        *,
        merchant_id: Optional[int] = None,
        operator_role: str = 'system',
        operator_id: Optional[str] = None,
        resource_type: Optional[str] = None,
        resource_id: Optional[str] = None,
        payload: Optional[Dict[str, Any]] = None,
        trace_id: Optional[str] = None,
    ) -> int:
        return await audit_repo.create_audit_log(
            tenant_id=tenant_id,
            merchant_id=merchant_id,
            operator_role=operator_role,
            operator_id=operator_id,
            action=action,
            resource_type=resource_type,
            resource_id=resource_id,
            payload_json=json.dumps(payload, ensure_ascii=False) if payload is not None else None,
            trace_id=trace_id,
        )

    async def list_logs(
        self,
        tenant_id: str,
        *,
        action: Optional[str] = None,
        resource_type: Optional[str] = None,
        limit: int = 100,
    ) -> List[Dict[str, Any]]:
        return await audit_repo.list_audit_logs(
            tenant_id=tenant_id,
            action=action,
            resource_type=resource_type,
            limit=limit,
        )


_audit_service: Optional[AuditService] = None


def get_audit_service() -> AuditService:
    global _audit_service
    if _audit_service is None:
        _audit_service = AuditService()
    return _audit_service
