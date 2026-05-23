# -*- coding: utf-8 -*-
"""
广告识别核心逻辑（可被群消息管道与 AI Tool 复用）
"""

import re
from typing import Any, Dict, List


AD_KEYWORDS: List[str] = [
    # 直接推广
    "加我", "扫码", "微信", "vx:", "VX:", "v信",
    "免费领取", "红包", "返利", "佣金", "代理",
    "拉人头", "邀请码", "注册送",
    # 投资理财诈骗
    "稳赚", "翻倍", "百倍", "暴富", "零风险",
    "内部消息", "内幕", "必涨", "抄底",
    # 违规平台
    "电报群", "telegram群", "飞机群", "梯子",
]

_URL_PATTERN = re.compile(r"https?://\S+")
_REPEAT_PATTERN = re.compile(r"(.)\1{5,}")
_PHONE_LIKE_PATTERN = re.compile(r"\d{5,11}")


def detect_advertising_signals(
    text: str,
    context_type: str = "group_message",
) -> Dict[str, Any]:
    """
    识别文本中的广告/推广特征并返回结构化评分结果。
    """
    raw = (text or "").strip()
    if not raw:
        return {
            "is_advertising": False,
            "ad_score": 0,
            "confidence": "low",
            "risk_level": "low",
            "findings": [],
            "matched_keywords": [],
            "url_count": 0,
            "context_type": context_type,
            "recommendation": "文本为空，无法判定",
        }

    findings: List[str] = []
    ad_score = 0

    matched_keywords = [kw for kw in AD_KEYWORDS if kw.lower() in raw.lower()]
    if matched_keywords:
        ad_score += min(len(matched_keywords) * 15, 45)
        findings.append(f"命中广告关键词: {', '.join(matched_keywords)}")

    urls = _URL_PATTERN.findall(raw)
    if len(urls) >= 3:
        ad_score += 20
        findings.append(f"包含 {len(urls)} 个链接（可能为批量推广）")
    elif urls:
        suspicious_urls = [u for u in urls if any(kw in u.lower() for kw in ("t.me/", "shorturl", "bit.ly"))]
        if suspicious_urls:
            ad_score += 15
            findings.append(f"发现可疑链接: {suspicious_urls}")

    repeats = _REPEAT_PATTERN.findall(raw)
    if len(repeats) > 3:
        ad_score += 10
        findings.append("存在大量重复字符（刷屏特征）")

    phone_like = _PHONE_LIKE_PATTERN.findall(raw)
    if len(phone_like) >= 2:
        ad_score += 15
        findings.append("发现多个疑似电话号码/ID")

    is_ad = ad_score >= 40
    confidence = "high" if ad_score >= 60 else "medium" if ad_score >= 40 else "low"
    risk_level = "high" if ad_score >= 60 else "medium" if ad_score >= 40 else "low"

    if is_ad:
        recommendation = "建议拦截该消息（高置信度广告）" if ad_score >= 60 else "疑似广告，建议人工复核"
    else:
        recommendation = "暂未检测到明显广告特征" if ad_score < 20 else "有轻微推广特征，建议持续观察"

    return {
        "is_advertising": is_ad,
        "ad_score": ad_score,
        "confidence": confidence,
        "risk_level": risk_level,
        "findings": findings,
        "matched_keywords": matched_keywords,
        "url_count": len(urls),
        "context_type": context_type,
        "recommendation": recommendation,
    }

