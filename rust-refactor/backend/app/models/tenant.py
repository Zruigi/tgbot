# -*- coding: utf-8 -*-
"""
多商户数据模型 - 行级安全隔离
"""
import uuid
from datetime import datetime
from sqlalchemy import Column, String, DateTime, Boolean, Text, JSONB, BigInteger, Index
from sqlalchemy.dialects.postgresql import UUID
from sqlalchemy.orm import relationship, declared_attr
from sqlalchemy.ext.declarative import declarative_base

Base = declarative_base()


class Tenant(Base):
    """租户/商户表"""
    __tablename__ = tenants
    
    id = Column(UUID(as_uuid=True), primary_key=True, default=uuid.uuid4)
    name = Column(String(255), nullable=False)
    plan = Column(String(50), default=free)  # free/pro/enterprise
    domain = Column(String(255), unique=True)
    
    # 配额
    max_merchants = Column(BigInteger, default=1)
    max_users = Column(BigInteger, default=1000)
    max_groups = Column(BigInteger, default=10)
    
    # 功能开关
    features = Column(JSONB, default=dict)
    
    created_at = Column(DateTime, default=datetime.utcnow)
    updated_at = Column(DateTime, default=datetime.utcnow, onupdate=datetime.utcnow)
    
    # 关系
    merchants = relationship(Merchant, back_populates=tenant)


class Merchant(Base):
    """商户表 (每个商户对应一个Telegram Bot)"""
    __tablename__ = merchants
    
    id = Column(UUID(as_uuid=True), primary_key=True, default=uuid.uuid4)
    tenant_id = Column(UUID(as_uuid=True), ForeignKey(tenants.id), nullable=False)
    
    name = Column(String(255), nullable=False)
    bot_token_encrypted = Column(Text, nullable=False)
    bot_username = Column(String(255))
    
    # 管理员TG ID列表 (支持多管理员)
    admin_tg_ids = Column(JSONB, default=list)
    
    # 群组配置
    monitored_groups = Column(JSONB, default=list)  # 监控的群组ID
    ad_detection_enabled = Column(Boolean, default=True)
    ad_auto_delete = Column(Boolean, default=False)
    ad_channel_id = Column(BigInteger)  # 广告推送频道
    
    # 翻译配置
    auto_translate = Column(Boolean, default=False)
    default_target_lang = Column(String(10), default=zh)
    
    created_at = Column(DateTime, default=datetime.utcnow)
    updated_at = Column(DateTime, default=datetime.utcnow, onupdate=datetime.utcnow)
    
    # 关系
    tenant = relationship(Tenant, back_populates=merchants)
    users = relationship(User, back_populates=merchant)
    groups = relationship(MonitoredGroup, back_populates=merchant)
    messages = relationship(Message, back_populates=merchant)


class User(Base):
    """用户表 (按商户隔离)"""
    __tablename__ = users
    
    id = Column(BigInteger, primary_key=True)  # Telegram User ID
    merchant_id = Column(UUID(as_uuid=True), ForeignKey(merchants.id), nullable=False)
    
    username = Column(String(255))
    first_name = Column(String(255))
    last_name = Column(String(255))
    language_code = Column(String(10))
    
    # 状态
    is_blocked = Column(Boolean, default=False)
    is_verified = Column(Boolean, default=False)
    
    created_at = Column(DateTime, default=datetime.utcnow)
    updated_at = Column(DateTime, default=datetime.utcnow, onupdate=datetime.utcnow)
    
    # 索引
    __table_args__ = (
        Index('ix_users_merchant_id', 'merchant_id'),
        Index('ix_users_merchant_username', 'merchant_id', 'username'),
    )
    
    # 关系
    merchant = relationship(Merchant, back_populates=users)


class Message(Base):
    """消息表 (按商户隔离)"""
    __tablename__ = messages
    
    id = Column(BigInteger, primary_key=True, autoincrement=True)
    merchant_id = Column(UUID(as_uuid=True), ForeignKey(merchants.id), nullable=False)
    
    # 消息内容
    from_userid = Column(BigInteger, nullable=False)
    to_userid = Column(BigInteger)
    content = Column(Text)
    msg_type = Column(String(50), default=text)  # text/photo/video/audio/document
    file_id = Column(String(500))
    
    # 翻译
    original_text = Column(Text)
    translated_text = Column(Text)
    
    # 状态
    is_read = Column(Boolean, default=False)
    status = Column(String(50), default=sent)  # sending/sent/delivered/failed
    
    created_at = Column(DateTime, default=datetime.utcnow)
    
    # ��引
    __table_args__ = (
        Index('ix_messages_merchant_from', 'merchant_id', 'from_userid'),
        Index('ix_messages_merchant_to', 'merchant_id', 'to_userid'),
        Index('ix_messages_created', 'created_at'),
    )
    
    # 关系
    merchant = relationship(Merchant, back_populates=messages)


class MonitoredGroup(Base):
    """监控的群组"""
    __tablename__ = monitored_groups
    
    id = Column(BigInteger, primary_key=True)  # Telegram Chat ID
    merchant_id = Column(UUID(as_uuid=True), ForeignKey(merchants.id), nullable=False)
    
    title = Column(String(255))
    username = Column(String(255))
    member_count = Column(BigInteger, default=0)
    
    # 广告检测配置
    ad_detection_enabled = Column(Boolean, default=True)
    ad_threshold = Column(BigInteger, default=40)
    ad_auto_delete = Column(Boolean, default=False)
    ad_notify_channel = Column(BigInteger)
    
    # 状态
    is_active = Column(Boolean, default=True)
    
    created_at = Column(DateTime, default=datetime.utcnow)
    updated_at = Column(DateTime, default=datetime.utcnow, onupdate=datetime.utcnow)
    
    # 关系
    merchant = relationship(Merchant, back_populates=groups)
    advertisements = relationship(Advertisement, back_populates=group)


class Advertisement(Base):
    """广告记录"""
    __tablename__ = advertisements
    
    id = Column(BigInteger, primary_key=True, autoincrement=True)
    merchant_id = Column(UUID(as_uuid=True), ForeignKey(merchants.id), nullable=False)
    group_id = Column(BigInteger, ForeignKey(monitored_groups.id))
    
    # 广告内容
    content = Column(Text, nullable=False)
    ad_score = Column(BigInteger, default=0)  # 广告评分 0-100
    ad_type = Column(JSONB, default=dict)  # spam/phishing/fraud/other
    
    # 来源
    source_message_id = Column(BigInteger)
    user_id = Column(BigInteger)  # 发布广告的用户
    
    # 处理状态
    status = Column(String(50), default=pending)  # pending/detected/deleted/appealed
    action_taken = Column(String(50))  # deleted/banned/warned
    
    # AI处理
    ai_beautified_content = Column(Text)  # AI美化后的内容
    ai_beautify_prompt = Column(Text)
    
    created_at = Column(DateTime, default=datetime.utcnow)
    processed_at = Column(DateTime)
    
    # 关系
    group = relationship(MonitoredGroup, back_populates=advertisements)


# SQLAlchemy 需要这个导入
from sqlalchemy import ForeignKey
