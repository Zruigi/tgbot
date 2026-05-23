# -*- coding: utf-8 -*-
"""
商户通知 Service - 关键事件主动推送

通过主 Bot 给商户 owner 发送 Telegram 消息通知：
- 订阅激活
- 订阅到期/降级
- 支付确认/拒绝
- 商户接入确认
"""
import logging
from datetime import datetime
from typing import Optional

from backend.repositories import merchant_repo

logger = logging.getLogger(__name__)

# 模块级引用，避免循环导入；在首次使用时从 main.py 注入
_bot_instance = None


def set_notification_bot(bot) -> None:
    """注入主 Bot 实例，由 main.py 在 post_init 时调用"""
    global _bot_instance
    _bot_instance = bot
    logger.info("merchant notification bot instance set")


async def notify_merchant(
    owner_tg_user_id: int,
    text: str,
    *,
    parse_mode: str = "HTML",
) -> bool:
    """向商户 owner 发送 Telegram 通知"""
    if not _bot_instance:
        logger.warning("notification bot not set, skip notification to user %s", owner_tg_user_id)
        return False
    try:
        await _bot_instance.send_message(
            chat_id=int(owner_tg_user_id),
            text=text,
            parse_mode=parse_mode,
            disable_web_page_preview=True,
        )
        return True
    except Exception as e:
        logger.warning("notify merchant user %s failed: %s", owner_tg_user_id, e)
        return False


async def notify_subscription_activated(merchant_id: int, plan_code: str, end_at: Optional[str] = None) -> bool:
    """通知商户：订阅已激活"""
    merchant = await merchant_repo.get_merchant_by_id(merchant_id)
    if not merchant:
        return False
    owner_id = int(merchant.get('owner_tg_user_id') or 0)
    if not owner_id:
        return False
    end_line = f"\n到期时间：<code>{end_at}</code>" if end_at else ""
    text = (
        "✅ <b>订阅已激活</b>\n\n"
        f"套餐：<code>{plan_code}</code>{end_line}\n\n"
        "你的商户功能已升级，可在商户控制台查看。"
    )
    return await notify_merchant(owner_id, text)


async def notify_subscription_downgraded(merchant_id: int, from_plan: str) -> bool:
    """通知商户：订阅已到期降级"""
    merchant = await merchant_repo.get_merchant_by_id(merchant_id)
    if not merchant:
        return False
    owner_id = int(merchant.get('owner_tg_user_id') or 0)
    if not owner_id:
        return False
    text = (
        "⚠️ <b>订阅已到期降级</b>\n\n"
        f"原套餐：<code>{from_plan}</code>\n"
        f"当前套餐：<code>free</code>\n\n"
        "付费功能已关闭。如需恢复，请重新购买订阅。"
    )
    return await notify_merchant(owner_id, text)


async def notify_payment_confirmed(merchant_id: int, order_id: int, plan_code: str) -> bool:
    """通知商户：支付已确认"""
    merchant = await merchant_repo.get_merchant_by_id(merchant_id)
    if not merchant:
        return False
    owner_id = int(merchant.get('owner_tg_user_id') or 0)
    if not owner_id:
        return False
    text = (
        "✅ <b>支付已确认</b>\n\n"
        f"订单号：<code>{order_id}</code>\n"
        f"套餐：<code>{plan_code}</code>\n\n"
        "订阅已激活，功能已升级。"
    )
    return await notify_merchant(owner_id, text)


async def notify_payment_rejected(merchant_id: int, order_id: int) -> bool:
    """通知商户：支付已被驳回"""
    merchant = await merchant_repo.get_merchant_by_id(merchant_id)
    if not merchant:
        return False
    owner_id = int(merchant.get('owner_tg_user_id') or 0)
    if not owner_id:
        return False
    text = (
        "❌ <b>支付已被驳回</b>\n\n"
        f"订单号：<code>{order_id}</code>\n\n"
        "平台审核未通过，请检查付款金额和地址后重新下单。"
    )
    return await notify_merchant(owner_id, text)


async def notify_onboarding_confirmed(merchant_id: int) -> bool:
    """通知商户：接入已确认"""
    merchant = await merchant_repo.get_merchant_by_id(merchant_id)
    if not merchant:
        return False
    if merchant.get('onboarding_notified_at'):
        logger.info("skip onboarding notification for merchant %s: already notified", merchant_id)
        return False
    owner_id = int(merchant.get('owner_tg_user_id') or 0)
    if not owner_id:
        return False
    text = (
        "🎉 <b>商户接入已确认</b>\n\n"
        f"商户名称：<code>{merchant.get('merchant_name') or '-'}</code>\n"
        f"Tenant ID：<code>{merchant.get('tenant_id') or '-'}</code>\n\n"
        "你的商户账号已就绪，点击\"商户控制台\"开始配置。"
    )
    sent = await notify_merchant(owner_id, text)
    if sent:
        await merchant_repo.update_merchant(merchant_id, {
            'onboarding_notified_at': datetime.utcnow().strftime('%Y-%m-%d %H:%M:%S'),
        })
    return sent
