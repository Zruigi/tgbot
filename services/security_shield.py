# -*- coding: utf-8 -*-
"""
安全保护引擎：地址防篡改与强一致性核验
"""
import re
import logging

logger = logging.getLogger(__name__)

# 加密地址正则 (支持多种主流链)
# 1. BTC (Legacy, SegWit): [13][a-km-zA-HJ-NP-Z1-9]{25,34}
# 2. ETH/BSC (EVM): 0x[a-fA-F0-9]{40}
# 3. TRON (TRC20): T[A-Za-z0-9]{33}
# 4. Monero: 4[0-9a-zA-Z]{94}
ADDR_REGEX = r'(?:[13][a-km-zA-HJ-NP-Z1-9]{25,34}|0x[a-fA-F0-9]{40}|T[A-Za-z0-9]{33}|4[0-9a-zA-Z]{94})'

def shield_text(text: str):
    """
    1. 占位符逻辑：提取地址并替换为占位符
    返回：(处理后的文本, 地址映射表)
    """
    if not text:
        return text, {}
        
    addresses = re.findall(ADDR_REGEX, text)
    if addresses:
        logger.info(f"[SHIELD] Found {len(addresses)} addresses: {addresses}")
    mapping = {}
    shielded_text = text
    
    for i, addr in enumerate(addresses):
        placeholder = f"[[ADDR_{i}]]"
        mapping[placeholder] = addr
        shielded_text = shielded_text.replace(addr, placeholder)
        
    return shielded_text, mapping

def unshield_text(shielded_text: str, mapping: dict):
    """
    2. 本地回填：还原占位符
    """
    if not shielded_text or not mapping:
        return shielded_text
        
    final_text = shielded_text
    for placeholder, addr in mapping.items():
        final_text = final_text.replace(placeholder, addr)
        
    return final_text

def verify_address_consistency(original_text: str, final_text: str):
    """
    3. 强制一致性校验：确保原文与最终文本中的地址完全一致
    """
    orig_addrs = set(re.findall(ADDR_REGEX, original_text))
    final_addrs = set(re.findall(ADDR_REGEX, final_text))
    
    if orig_addrs != final_addrs:
        logger.error(f"❌ 地址一致性校验失败！原文地址: {orig_addrs}, 最终地址: {final_addrs}")
        return False
    return True

# 管理员回复前缀正则 (2026 规格)
ADMIN_REPLY_PREFIX_REGEX = r'^💡\s*(?:管理员回复|客服)\s*[^：]+[:：]\s*'
# OCR/系统标签正则 (包含换行符开头的)
SYSTEM_TAG_REGEX = r'\n?\(\s*[⚠️✅❌]\s*[^)]+\)'

def strip_administrative_remarks(text: str):
    """
    4. 提取纯净译文内容：过滤掉管理员前缀、系统标签以及孤立的钱包地址行
    """
    if not text:
        return ""
        
    # 1. 过滤管理员前缀
    cleaned = re.sub(ADMIN_REPLY_PREFIX_REGEX, "", text, flags=re.MULTILINE)
    
    # 2. 过滤系统标签 (⚠️/✅/❌ 等)
    cleaned = re.sub(SYSTEM_TAG_REGEX, "", cleaned)
    
    # 3. 过滤孤立的钱包地址行 (如果一行只有地址，通常是元数据)
    lines = cleaned.split('\n')
    filtered_lines = []
    for line in lines:
        stripped_line = line.strip()
        # 如果整行仅由地址或地址占位符组成，且没有其他文字，则过滤掉
        if stripped_line and (re.fullmatch(ADDR_REGEX, stripped_line) or re.fullmatch(r'\[\[ADDR_\d+\]\]', stripped_line)):
            continue
        filtered_lines.append(line)
        
    final_cleaned = "\n".join(filtered_lines).strip()
    return final_cleaned

def contains_address(text: str):
    """检测是否包含地址 (预处理去空格以防规避)"""
    if not text: return False
    # 尝试直接匹配
    if re.search(ADDR_REGEX, text):
        return True
    # 尝试去掉空格和换行后再匹配 (防止用户通过 T P 3... 规避)
    clean_text = "".join(text.split())
    return bool(re.search(ADDR_REGEX, clean_text))
