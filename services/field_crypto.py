# -*- coding: utf-8 -*-
"""
字段级 AES-256-GCM 加密模块
- 密钥从环境变量 DB_ENCRYPT_KEY 读取（32字节 base64url 编码）
- 加密格式: ENC::<base64(nonce+ciphertext+tag)>
- 未加密的明文字段可透明读取（向后兼容）
"""
import os
import base64
import logging
from typing import Optional

logger = logging.getLogger(__name__)

_key: Optional[bytes] = None
_ENABLED: Optional[bool] = None  # None = 未初始化

def _load_key():
    global _key, _ENABLED
    if _ENABLED is not None:
        return  # 已初始化，跳过
    raw = os.environ.get('DB_ENCRYPT_KEY', '').strip()
    if not raw:
        logger.warning('[FieldCrypto] DB_ENCRYPT_KEY 未设置，字段加密已禁用')
        _ENABLED = False
        return
    try:
        key = base64.urlsafe_b64decode(raw + '==')
        if len(key) != 32:
            raise ValueError(f'密钥长度必须为32字节，当前: {len(key)}')
        _key = key
        _ENABLED = True
        logger.info('[FieldCrypto] AES-256-GCM 字段加密已启用')
    except Exception as e:
        logger.error(f'[FieldCrypto] 密钥加载失败: {e}，加密已禁用')
        _ENABLED = False

def is_enabled() -> bool:
    _load_key()
    return bool(_ENABLED)

def encrypt(plaintext: Optional[str]) -> Optional[str]:
    """加密字符串，返回 ENC::<base64> 格式，None 直接返回 None"""
    if not _ENABLED or plaintext is None:
        return plaintext
    if isinstance(plaintext, str) and plaintext.startswith('ENC::'):
        return plaintext  # 已加密，跳过
    try:
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM
        nonce = os.urandom(12)
        aesgcm = AESGCM(_key)
        ct = aesgcm.encrypt(nonce, plaintext.encode('utf-8'), None)
        encoded = base64.urlsafe_b64encode(nonce + ct).decode('ascii')
        return f'ENC::{encoded}'
    except Exception as e:
        logger.error(f'[FieldCrypto] 加密失败: {e}')
        return plaintext

def decrypt(ciphertext: Optional[str]) -> Optional[str]:
    """解密 ENC::<base64> 格式，明文直接返回（向后兼容）"""
    if ciphertext is None:
        return None
    if not isinstance(ciphertext, str) or not ciphertext.startswith('ENC::'):
        return ciphertext  # 明文，直接返回（兼容旧数据）
    if not _ENABLED or _key is None:
        logger.warning('[FieldCrypto] 收到加密字段但密钥未设置，返回占位符')
        return '[已加密]'
    try:
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM
        raw = base64.urlsafe_b64decode(ciphertext[5:] + '==')
        nonce, ct = raw[:12], raw[12:]
        aesgcm = AESGCM(_key)
        return aesgcm.decrypt(nonce, ct, None).decode('utf-8')
    except Exception as e:
        logger.error(f'[FieldCrypto] 解密失败: {e}')
        return '[解密失败]'

def encrypt_dict(d: Optional[dict], fields: list) -> Optional[dict]:
    """批量加密 dict 中指定字段"""
    if not d:
        return d
    result = dict(d)
    for f in fields:
        if f in result:
            result[f] = encrypt(result[f])
    return result

def decrypt_dict(d: Optional[dict], fields: list) -> Optional[dict]:
    """批量解密 dict 中指定字段"""
    if not d:
        return d
    result = dict(d)
    for f in fields:
        if f in result:
            result[f] = decrypt(result[f])
    return result

def decrypt_list(rows: list, fields: list) -> list:
    """批量解密 list[dict] 中指定字段"""
    return [decrypt_dict(r, fields) for r in rows]
