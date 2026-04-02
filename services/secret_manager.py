# -*- coding: utf-8 -*-
"""
SecretManager - 租户级加密配置管理服务
- 负责从数据库加载加密的 API 密钥
- 联动 field_crypto 进行透明解密
- 支持多租户 Key 隔离
"""
import logging
import aiosqlite
from typing import Optional, Any
from config import DB_NAME
from services import field_crypto

logger = logging.getLogger(__name__)

class SecretManager:
    _cache = {}  # 简单的内存缓存，减少数据库压力

    @classmethod
    async def get_config(cls, key: str, tenant_id: str = 'default', default: Any = None) -> Any:
        """
        获取配置，优先从数据库读取加密值并解密
        """
        cache_key = f"{tenant_id}:{key}"
        if cache_key in cls._cache:
            return cls._cache[cache_key]

        try:
            async with aiosqlite.connect(DB_NAME) as db:
                db.row_factory = aiosqlite.Row
                async with db.execute(
                    "SELECT config_value FROM tenant_configs WHERE tenant_id = ? AND config_key = ?",
                    (tenant_id, key)
                ) as cursor:
                    row = await cursor.fetchone()
                    if row:
                        encrypted_val = row['config_value']
                        # 尝试解密
                        decrypted_val = field_crypto.decrypt(encrypted_val)
                        cls._cache[cache_key] = decrypted_val
                        return decrypted_val
        except Exception as e:
            logger.error(f"[SecretManager] 获取配置 {key} 失败: {e}")

        return default

    @classmethod
    async def set_config(cls, key: str, value: str, tenant_id: str = 'default', remark: str = '') -> bool:
        """
        设置配置，自动进行加密存储
        """
        try:
            encrypted_val = field_crypto.encrypt(value)
            async with aiosqlite.connect(DB_NAME) as db:
                await db.execute(
                    """INSERT INTO tenant_configs (tenant_id, config_key, config_value, remark, updated_at) 
                       VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP)
                       ON CONFLICT(tenant_id, config_key) DO UPDATE SET 
                       config_value=excluded.config_value, 
                       remark=excluded.remark,
                       updated_at=CURRENT_TIMESTAMP""",
                    (tenant_id, key, encrypted_val, remark)
                )
                await db.commit()
            
            # 更新缓存
            cls._cache[f"{tenant_id}:{key}"] = value
            return True
        except Exception as e:
            logger.error(f"[SecretManager] 保存配置 {key} 失败: {e}")
            return False

    @classmethod
    def invalidate_cache(cls):
        cls._cache = {}
