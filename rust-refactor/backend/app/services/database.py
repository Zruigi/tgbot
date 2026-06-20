# -*- coding: utf-8 -*-
"""
数据库操作工具 - 多商户隔离
"""
import json
import logging
from typing import Optional, List, Dict, Any
from datetime import datetime
from sqlalchemy import select, update, delete
from sqlalchemy.ext.asyncio import AsyncSession

from backend.app.models.tenant import Tenant, Merchant, User, Message, MonitoredGroup, Advertisement

logger = logging.getLogger(__name__)


class DBOperations:
    """数据库操作类 - 带商户隔离"""

    def __init__(self, session: AsyncSession):
        self.session = session

    # === 商户操作 ===
    async def get_merchant_by_id(self, merchant_id: str) -> Optional[Merchant]:
        """获取商户"""
        result = await self.session.execute(
            select(Merchant).where(Merchant.id == merchant_id)
        )
        return result.scalar_one_or_none()

    async def get_merchant_by_bot_token(self, bot_token: str) -> Optional[Merchant]:
        """通过Bot Token获取商户"""
        result = await self.session.execute(
            select(Merchant).where(Merchant.bot_token_encrypted == bot_token)
        )
        return result.scalar_one_or_none()

    # === 用户操作 ===
    async def get_or_create_user(self, merchant_id: str, user_id: int, **kwargs) -> User:
        """获取或创建用户"""
        result = await self.session.execute(
            select(User).where(
                User.merchant_id == merchant_id,
                User.id == user_id
            )
        )
        user = result.scalar_one_or_none()

        if not user:
            user = User(
                id=user_id,
                merchant_id=merchant_id,
                **kwargs
            )
            self.session.add(user)
            await self.session.commit()

        return user

    async def list_users(self, merchant_id: str, limit: int = 50, offset: int = 0) -> List[User]:
        """列出商户用户"""
        result = await self.session.execute(
            select(User)
            .where(User.merchant_id == merchant_id)
            .order_by(User.created_at.desc())
            .limit(limit)
            .offset(offset)
        )
        return list(result.scalars().all())

    # === 消息操作 ===
    async def save_message(
        self,
        merchant_id: str,
        from_userid: int,
        to_userid: Optional[int],
        content: str,
        msg_type: str = "text",
        **kwargs
    ) -> Message:
        """保存消息"""
        msg = Message(
            merchant_id=merchant_id,
            from_userid=from_userid,
            to_userid=to_userid,
            content=content,
            msg_type=msg_type,
            **kwargs
        )
        self.session.add(msg)
        await self.session.commit()
        return msg

    async def get_messages(
        self,
        merchant_id: str,
        user_id: int,
        limit: int = 50
    ) -> List[Message]:
        """获取用户消息"""
        result = await self.session.execute(
            select(Message)
            .where(
                Message.merchant_id == merchant_id,
                (Message.from_userid == user_id) | (Message.to_userid == user_id)
            )
            .order_by(Message.created_at.desc())
            .limit(limit)
        )
        return list(result.scalars().all())

    # === 群组操作 ===
    async def get_or_create_group(self, merchant_id: str, group_id: int, **kwargs) -> MonitoredGroup:
        """获取或创建群组"""
        result = await self.session.execute(
            select(MonitoredGroup).where(
                MonitoredGroup.merchant_id == merchant_id,
                MonitoredGroup.id == group_id
            )
        )
        group = result.scalar_one_or_none()

        if not group:
            group = MonitoredGroup(
                id=group_id,
                merchant_id=merchant_id,
                **kwargs
            )
            self.session.add(group)
            await self.session.commit()

        return group

    async def list_groups(self, merchant_id: str) -> List[MonitoredGroup]:
        """列出商户的群组"""
        result = await self.session.execute(
            select(MonitoredGroup)
            .where(
                MonitoredGroup.merchant_id == merchant_id,
                MonitoredGroup.is_active == True
            )
        )
        return list(result.scalars().all())

    # === 广告操作 ===
    async def register_advertisement(
        self,
        merchant_id: str,
        content: str,
        ad_score: int,
        group_id: Optional[int] = None,
        user_id: Optional[int] = None,
        **kwargs
    ) -> Advertisement:
        """登记广告"""
        ad = Advertisement(
            merchant_id=merchant_id,
            content=content,
            ad_score=ad_score,
            group_id=group_id,
            user_id=user_id,
            status="detected",
            processed_at=datetime.utcnow(),
            **kwargs
        )
        self.session.add(ad)
        await self.session.commit()
        return ad

    async def list_advertisements(
        self,
        merchant_id: str,
        status: str = "pending",
        limit: int = 50
    ) -> List[Advertisement]:
        """列出广告"""
        result = await self.session.execute(
            select(Advertisement)
            .where(
                Advertisement.merchant_id == merchant_id,
                Advertisement.status == status
            )
            .order_by(Advertisement.created_at.desc())
            .limit(limit)
        )
        return list(result.scalars().all())

    async def update_ad_status(
        self,
        ad_id: int,
        status: str,
        action_taken: Optional[str] = None
    ) -> bool:
        """更新广告状态"""
        await self.session.execute(
            update(Advertisement)
            .where(Advertisement.id == ad_id)
            .values(status=status, action_taken=action_taken)
        )
        await self.session.commit()
        return True