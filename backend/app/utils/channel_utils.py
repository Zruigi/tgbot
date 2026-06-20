# -*- coding: utf-8 -*-
"""
渠道工具函数 - 统一处理频道链接验证、规范化
合并自 web_admin.py 中的重复函数：
- _validate_channel_link (1663 行)
- normalize_channel_link (3277 行)
- get_button_url (3308 行)
"""
import re
from typing import Optional


def normalize_channel_link(link: str) -> str:
    """
    规范化频道链接格式
    
    支持格式：
    - https://t.me/username
    - https://telegram.me/username
    - t.me/username
    - @username
    - username
    - /username
    - https://t.me/joinchat/invite_code
    
    返回：标准化后的链接（https://t.me/username 格式）
    """
    if not link or not link.strip():
        return ''
    
    link = link.strip()
    
    # 移除前后缀和多余字符
    if link.startswith('@'):
        link = link[1:]
    if link.startswith('https://'):
        link = link[8:]
    elif link.startswith('http://'):
        link = link[7:]
    if link.startswith('t.me/'):
        link = link[5:]
    elif link.startswith('telegram.me/'):
        link = link[14:]
    if link.startswith('/'):
        link = link[1:]

    if link.startswith('joinchat/'):
        invite_code = link[len('joinchat/'):]
        if not re.match(r'^[a-zA-Z0-9_-]{5,}$', invite_code):
            raise ValueError(f'无效的频道链接：{link}')
        return f'https://t.me/joinchat/{invite_code}'
    
    # 提取用户名部分
    match = re.match(r'^([a-zA-Z][a-zA-Z0-9_]{3,31})$', link)
    if not match:
        raise ValueError(f'无效的频道链接：{link}')
    
    username = match.group(1)
    return f'https://t.me/{username}'


def validate_channel_link(link: str) -> bool:
    """
    验证频道链接是否有效
    
    规则：
    - 长度 5-32 字符
    - 以字母开头
    - 可包含字母、数字、下划线
    
    返回：True 表示有效，False 表示无效
    """
    if not link or not link.strip():
        return False
    
    try:
        normalized = normalize_channel_link(link)
        return bool(normalized)
    except (ValueError, AttributeError):
        return False


def parse_channel_username(link: str) -> Optional[str]:
    """
    从频道链接中提取用户名
    
    返回：用户名字符串，如果无效则返回 None
    """
    if not link or not link.strip():
        return None
    
    try:
        normalized = normalize_channel_link(link)
        # 从 https://t.me/username 中提取 username
        return normalized.replace('https://t.me/', '')
    except (ValueError, AttributeError):
        return None


def get_button_url(link: str) -> str:
    """
    获取按钮显示的 URL（兼容旧代码）
    
    这是 normalize_channel_link 的别名，为了向后兼容
    """
    return normalize_channel_link(link)


# 钱包地址验证正则（保留原有功能）
_TRON_ADDR_RE = re.compile(r'^T[1-9A-HJ-NP-Za-km-z]{33}$')
_ETH_BSC_ADDR_RE = re.compile(r'^0x[0-9a-fA-F]{40}$')


def validate_wallet_address(address: str, network: str) -> bool:
    """
    验证钱包地址格式
    
    Args:
        address: 钱包地址
        network: 网络类型 ('TRC20', 'BSC', 'ETH')
    
    Returns:
        True 表示地址格式有效
    """
    if not address or not address.strip():
        return False
    
    address = address.strip()
    
    if network.upper() == 'TRC20':
        return bool(_TRON_ADDR_RE.match(address))
    elif network.upper() in ('BSC', 'ETH', 'ERC20'):
        return bool(_ETH_BSC_ADDR_RE.match(address))
    else:
        # 未知网络类型，暂时不验证
        return True
