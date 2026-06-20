# -*- coding: utf-8 -*-
"""
自动审批服务 - 自动审批符合条件的商户
"""
import logging
from typing import List, Dict, Any
from datetime import datetime

from backend.repositories import merchant_repo
from backend.app.services import get_audit_service
from backend.app.services.feature_gate_service import get_feature_gate_service
from backend.app.services.quota_service import get_quota_service
from services.secret_manager import SecretManager

logger = logging.getLogger(__name__)


class AutoApprovalService:
    """自动审批服务"""

    @staticmethod
    def _is_onboarding_completed(merchant: Dict[str, Any]) -> bool:
        explicit = merchant.get('onboarding_completed')
        if explicit is not None:
            try:
                if int(explicit) == 1:
                    return True
            except Exception:
                if bool(explicit):
                    return True

        status = str(merchant.get('status') or '').strip().lower()
        plan_type = str(merchant.get('plan_type') or '').strip().lower()
        has_bot_binding = bool(merchant.get('bot_id') or merchant.get('bot_username'))
        return has_bot_binding and status in {'active_free', 'active_pro', 'active_paid'} and plan_type in {'free', 'pro'}

    @staticmethod
    def _is_already_approved(merchant: Dict[str, Any]) -> bool:
        return bool(merchant.get('onboarding_approved_at'))
    
    async def auto_approve_pending_merchants(self) -> List[Dict[str, Any]]:
        """
        自动审批待审批的商户

        审批条件：
        1. 商户状态为 active_free
        2. 已绑定Bot (bot_id存在)
        3. Bot Token已存储
        4. 功能快照已生成
        5. 配额已初始化
        6. 已完成onboarding (onboarding_completed=1)

        返回：审批成功的商户列表
        """
        approved = []

        # 查询所有待审批的商户
        merchants = await merchant_repo.list_merchants(limit=500, status='active_free')

        for merchant in merchants:
            merchant_id = int(merchant['id'])
            tenant_id = str(merchant.get('tenant_id') or '').strip()

            if self._is_already_approved(merchant):
                logger.debug(f"商户 {tenant_id} 已审批过，跳过自动审批")
                continue

            # 检查是否已完成onboarding，避免重复审批
            if not self._is_onboarding_completed(merchant):
                logger.debug(f"商户 {tenant_id} 未完成onboarding，跳过自动审批")
                continue

            # 检查是否满足自动审批条件
            checks = await self._check_auto_approval_conditions(merchant)

            if all(checks.values()):
                # 执行自动审批
                try:
                    await self._execute_auto_approval(merchant)
                    approved.append({
                        'merchant_id': merchant_id,
                        'tenant_id': tenant_id,
                        'merchant_name': merchant.get('merchant_name'),
                        'approved_at': datetime.utcnow().isoformat(),
                    })
                    logger.info(f"自动审批商户成功: {tenant_id} ({merchant.get('merchant_name')})")
                except Exception as e:
                    logger.error(f"自动审批商户失败: {tenant_id} - {e}")

        return approved
    
    async def _check_auto_approval_conditions(self, merchant: Dict[str, Any]) -> Dict[str, bool]:
        """检查自动审批条件"""
        tenant_id = str(merchant.get('tenant_id') or '').strip()
        merchant_id = int(merchant['id'])
        
        checks = {
            'has_bot_id': bool(merchant.get('bot_id')),
            'has_bot_username': bool(merchant.get('bot_username')),
            'has_tenant_id': bool(tenant_id),
            'has_owner': bool(merchant.get('owner_tg_user_id')),
        }
        
        # 检查Bot Token是否已存储
        if tenant_id:
            bot_token = await SecretManager.get_config('TELEGRAM_BOT_TOKEN', tenant_id=tenant_id, default='')
            checks['has_bot_token'] = bool(bot_token)
        else:
            checks['has_bot_token'] = False
        
        # 检查功能快照
        try:
            feature_map = await get_feature_gate_service().get_feature_map(tenant_id)
            checks['has_features'] = isinstance(feature_map, dict) and len(feature_map) > 0
        except:
            checks['has_features'] = False
        
        # 检查配额
        try:
            quota = await get_quota_service().get_quota(tenant_id)
            checks['has_quota'] = quota is not None
        except:
            checks['has_quota'] = False
        
        return checks
    
    async def _execute_auto_approval(self, merchant: Dict[str, Any]) -> None:
        """执行自动审批"""
        merchant_id = int(merchant['id'])
        tenant_id = str(merchant.get('tenant_id') or '').strip()
        now_str = datetime.utcnow().strftime('%Y-%m-%d %H:%M:%S')

        # 更新商户状态
        await merchant_repo.update_merchant_plan(
            merchant_id=merchant_id,
            plan_type='free',
            subscription_status='active',
            status='active_free',
        )
        await merchant_repo.update_merchant(merchant_id, {
            'onboarding_approved_at': now_str,
        })
        
        # 更新版本号
        await merchant_repo.bump_config_version(merchant_id)
        await merchant_repo.bump_feature_version(merchant_id)
        
        # 记录审计日志
        await get_audit_service().log(
            tenant_id=tenant_id,
            merchant_id=merchant_id,
            operator_role='system',
            action='merchant_auto_approved',
            resource_type='merchant',
            resource_id=str(merchant_id),
            payload={
                'merchant_name': merchant.get('merchant_name'),
                'source': 'auto_approval_service',
            },
        )
        
        # 通知商户
        try:
            from backend.app.services.merchant_notification_service import notify_onboarding_confirmed
            await notify_onboarding_confirmed(merchant_id)
        except Exception as e:
            logger.warning(f"通知商户失败: {e}")


_auto_approval_service = None


def get_auto_approval_service() -> AutoApprovalService:
    global _auto_approval_service
    if _auto_approval_service is None:
        _auto_approval_service = AutoApprovalService()
    return _auto_approval_service
