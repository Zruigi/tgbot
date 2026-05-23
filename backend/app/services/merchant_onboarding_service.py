# -*- coding: utf-8 -*-
"""
商户接入 Service - 商户注册、Bot Token 托管、默认套餐初始化
"""
from typing import Optional, Dict, Any
import re
from datetime import datetime
from telegram import Bot

from backend.repositories import merchant_repo
from backend.app.services.feature_gate_service import get_feature_gate_service
from backend.app.services.quota_service import get_quota_service
from backend.app.services.audit_service import get_audit_service
from backend.app.services.merchant_notification_service import notify_onboarding_confirmed
from services.secret_manager import SecretManager


class MerchantOnboardingService:
    async def create_merchant(
        self,
        owner_tg_user_id: int,
        merchant_name: str,
        *,
        tenant_id: Optional[str] = None,
        bot_token: Optional[str] = None,
    ) -> Dict[str, Any]:
        effective_tenant_id = tenant_id or self._build_tenant_id(owner_tg_user_id, merchant_name)

        bot_id = None
        bot_username = None
        if bot_token:
            bot_info = await self.validate_bot_token(bot_token)
            bot_id = int(bot_info['id'])
            bot_username = bot_info.get('username')

        merchant_id = await merchant_repo.create_merchant(
            tenant_id=effective_tenant_id,
            owner_tg_user_id=int(owner_tg_user_id),
            merchant_name=merchant_name.strip(),
            bot_id=bot_id,
            bot_username=bot_username,
            status='active_free',
            plan_type='free',
            subscription_status='free',
        )

        if bot_token:
            await SecretManager.set_config(
                key='TELEGRAM_BOT_TOKEN',
                value=bot_token,
                tenant_id=effective_tenant_id,
                remark='merchant bot token',
            )

        await get_feature_gate_service().rebuild_for_tenant(
            effective_tenant_id,
            merchant_id=merchant_id,
            plan_code='free',
            source='onboarding',
        )
        await get_quota_service().ensure_quota(
            merchant_id=merchant_id,
            tenant_id=effective_tenant_id,
            daily_quota=0,
            monthly_quota=0,
        )
        await get_audit_service().log(
            tenant_id=effective_tenant_id,
            merchant_id=merchant_id,
            operator_role='merchant_owner',
            operator_id=str(owner_tg_user_id),
            action='merchant_onboarded',
            resource_type='merchant',
            resource_id=str(merchant_id),
            payload={'merchant_name': merchant_name, 'bot_username': bot_username},
        )
        return await merchant_repo.get_merchant_by_id(merchant_id) or {}

    async def validate_bot_token(self, bot_token: str) -> Dict[str, Any]:
        bot = Bot(token=bot_token)
        me = await bot.get_me()
        return {
            'id': me.id,
            'username': me.username,
            'first_name': me.first_name,
            'is_bot': me.is_bot,
        }

    async def save_bot_token(self, tenant_id: str, bot_token: str) -> Dict[str, Any]:
        merchant = await merchant_repo.get_merchant_by_tenant_id(tenant_id)
        if not merchant:
            raise ValueError('merchant_not_found')

        bot_info = await self.validate_bot_token(bot_token)
        await SecretManager.set_config(
            key='TELEGRAM_BOT_TOKEN',
            value=bot_token,
            tenant_id=tenant_id,
            remark='merchant bot token',
        )
        await merchant_repo.update_merchant_bot(
            merchant_id=int(merchant['id']),
            bot_id=int(bot_info['id']),
            bot_username=bot_info.get('username'),
        )
        await merchant_repo.bump_runtime_version(int(merchant['id']))
        await get_audit_service().log(
            tenant_id=tenant_id,
            merchant_id=int(merchant['id']),
            operator_role='merchant_owner',
            action='bot_token_updated',
            resource_type='merchant_bot',
            resource_id=str(bot_info['id']),
            payload={'bot_username': bot_info.get('username')},
        )
        
        # 通知管理员有新的Bot Token提交
        try:
            from web_admin import notify_web_clients
            await notify_web_clients('merchant_bot_token_updated', {
                'merchant_id': int(merchant['id']),
                'tenant_id': tenant_id,
                'merchant_name': merchant.get('merchant_name'),
                'bot_username': bot_info.get('username'),
                'bot_id': int(bot_info['id']),
            })
        except Exception as e:
            # 通知失败不影响主流程
            pass
        
        return bot_info

    async def confirm_onboarding(self, merchant_id: int, note: Optional[str] = None) -> Dict[str, Any]:
        merchant = await merchant_repo.get_merchant_by_id(int(merchant_id))
        if not merchant:
            raise ValueError('merchant_not_found')

        tenant_id = str(merchant.get('tenant_id') or '')
        checks = {
            'merchant_exists': True,
            'tenant_id_present': bool(tenant_id),
            'owner_present': bool(merchant.get('owner_tg_user_id')),
            'feature_snapshot_readable': False,
            'quota_ready': False,
            'bot_binding_present': bool(merchant.get('bot_id') or merchant.get('bot_username')),
        }
        warnings = []

        feature_map = await get_feature_gate_service().get_feature_map(tenant_id)
        checks['feature_snapshot_readable'] = isinstance(feature_map, dict)
        if not feature_map:
            warnings.append('feature_snapshot_empty')

        quota = await get_quota_service().get_quota(tenant_id)
        checks['quota_ready'] = quota is not None
        if quota is None:
            warnings.append('quota_missing')

        if checks['bot_binding_present']:
            has_secret = bool(await SecretManager.get_config('TELEGRAM_BOT_TOKEN', tenant_id=tenant_id, default=''))
            checks['bot_secret_ready'] = has_secret
            if not has_secret:
                warnings.append('bot_secret_missing')
        else:
            checks['bot_secret_ready'] = False
            warnings.append('bot_not_bound')

        all_checks_passed = all(bool(v) for v in checks.values())
        status_changed = False
        if all_checks_passed:
            target_status = 'active_free'
            target_plan_type = str(merchant.get('plan_type') or '').strip() or 'free'
            target_subscription_status = 'active' if target_plan_type == 'free' else (str(merchant.get('subscription_status') or '').strip() or 'active')
            target_subscription_expires_at = merchant.get('subscription_expires_at')

            status_changed = await merchant_repo.update_merchant_plan(
                merchant_id=int(merchant['id']),
                plan_type=target_plan_type,
                subscription_status=target_subscription_status,
                subscription_expires_at=target_subscription_expires_at,
                status=target_status,
            )
            await merchant_repo.update_merchant(int(merchant['id']), {
                'onboarding_approved_at': str(merchant.get('onboarding_approved_at') or '').strip() or datetime.utcnow().strftime('%Y-%m-%d %H:%M:%S'),
            })
            await merchant_repo.bump_config_version(int(merchant['id']))
            await merchant_repo.bump_feature_version(int(merchant['id']))
            merchant = await merchant_repo.get_merchant_by_id(int(merchant['id'])) or merchant

        audit_log_id = await get_audit_service().log(
            tenant_id=tenant_id,
            merchant_id=int(merchant['id']),
            operator_role='platform_admin',
            action='merchant_onboarding_confirmed',
            resource_type='merchant',
            resource_id=str(merchant['id']),
            payload={
                'merchant_name': merchant.get('merchant_name'),
                'checks': checks,
                'warnings': warnings,
                'note': note or '',
                'source': 'platform_manual_confirm',
                'all_checks_passed': all_checks_passed,
                'status_changed': status_changed,
            },
        )
        if all_checks_passed:
            await notify_onboarding_confirmed(int(merchant['id']))
        return {
            'merchant': merchant,
            'checks': checks,
            'warnings': warnings,
            'audit_log_id': audit_log_id,
            'all_checks_passed': all_checks_passed,
            'status_changed': status_changed,
        }

    def _build_tenant_id(self, owner_tg_user_id: int, merchant_name: str) -> str:
        slug = re.sub(r'[^a-z0-9]+', '-', (merchant_name or '').strip().lower()).strip('-')
        slug = slug or 'merchant'
        return f"m-{int(owner_tg_user_id)}-{slug[:32]}"


_merchant_onboarding_service: Optional[MerchantOnboardingService] = None


def get_merchant_onboarding_service() -> MerchantOnboardingService:
    global _merchant_onboarding_service
    if _merchant_onboarding_service is None:
        _merchant_onboarding_service = MerchantOnboardingService()
    return _merchant_onboarding_service
