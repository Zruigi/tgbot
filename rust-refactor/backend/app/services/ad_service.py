# -*- coding: utf-8 -*-
"""
广告检测服务 - AI Agent 工具实现
"""
import re
import logging
from typing import Dict, Any, Optional
from datetime import datetime

logger = logging.getLogger(__name__)


class AdDetectionService:
    """广告检测服务"""

    # 关键词模式
    SPAM_KEYWORDS = [
        "加我", "扫码", "免费领取", "点击链接", "点击下方",
        "联系微信", "加微信", "vip", "代理", "兼职",
        "日赚", "月入", "稳赚", "盈利", "投资",
        "菠菜", "棋牌", "彩票", "赌博", "av", "黄色",
        "木马", "钓鱼", "盗号", "外挂", "破解"
    ]

    # URL模式
    URL_PATTERN = re.compile(
        r'(https?://|www\.)[^\s]+',
        re.IGNORECASE
    )

    # 重复字符模式 (刷屏)
    REPEAT_PATTERN = re.compile(r'(.)\1{5,}')

    @classmethod
    async def detect(
        cls,
        text: str,
        context: str = "group_message",
        user_id: Optional[int] = None
    ) -> Dict[str, Any]:
        """
        检测广告

        Args:
            text: 待检测文本
            context: 上下文 (group_message/private)
            user_id: 用户ID

        Returns:
            {
                "is_advertising": bool,
                "ad_score": int (0-100),
                "ad_type": dict,
                "reasons": list
            }
        """
        if not text:
            return {"is_advertising": False, "ad_score": 0, "ad_type": {}, "reasons": []}

        reasons = []
        ad_score = 0

        # 1. 关键词检测
        text_lower = text.lower()
        keyword_matches = []
        for kw in cls.SPAM_KEYWORDS:
            if kw in text_lower:
                keyword_matches.append(kw)
                ad_score += 15

        if keyword_matches:
            reasons.append(f"关键词: {', '.join(keyword_matches[:3])}")

        # 2. URL检测
        urls = cls.URL_PATTERN.findall(text)
        if urls:
            # 批量推广链接
            if len(urls) > 1:
                ad_score += 40
                reasons.append(f"多个URL链接 ({len(urls)}个)")
            # 可疑短链接
            for url in urls:
                if any(x in url.lower() for x in ['.xyz', '.top', '.click', '.link', 'bit.ly']):
                    ad_score += 20
                    reasons.append("可疑短链接")
                    break

        # 3. 重复字符检测 (刷屏)
        if cls.REPEAT_PATTERN.search(text):
            ad_score += 25
            reasons.append("重复字符刷屏")

        # 4. 特殊符号检测
        if text.count('❗') > 3 or text.count('❕') > 3:
            ad_score += 10
            reasons.append("过多特殊符号")

        # 5. 表情包刷屏
        emoji_count = len(re.findall(r'[\U0001F300-\U0001F9FF]', text))
        if emoji_count > 10:
            ad_score += 15
            reasons.append("表情包刷屏")

        # 限制分数上限
        ad_score = min(ad_score, 100)

        # 判断是否为广告
        is_advertising = ad_score >= 40

        # 分类
        ad_type = {}
        if "菠菜" in text_lower or "棋牌" in text_lower or "赌博" in text_lower:
            ad_type["gambling"] = True
        if "免费" in text_lower or "领取" in text_lower:
            ad_type["spam"] = True
        if "投资" in text_lower or "盈利" in text_lower or "日赚" in text_lower:
            ad_type["scam"] = True
        if keyword_matches:
            ad_type["keyword"] = True
        if urls:
            ad_type["url"] = True

        return {
            "is_advertising": is_advertising,
            "ad_score": ad_score,
            "ad_type": ad_type,
            "reasons": reasons,
            "checked_at": datetime.utcnow().isoformat()
        }


class AdBeautifyService:
    """广告美化服务 - 转换为合规推广"""

    @classmethod
    async def beautify(
        cls,
        original_content: str,
        target_style: str = "product",
        language: str = "zh"
    ) -> Dict[str, Any]:
        """
        美化广告内容

        Args:
            original_content: 原始广告内容
            target_style: 目标风格 (product/event/service)
            language: 语言

        Returns:
            {
                "beautified": str,
                "style": str,
                "prompt": str
            }
        """
        prompts = {
            "product": "将以下内容改写为简洁的产品介绍，突出产品优点，不要夸大:",
            "event": "将以下内容改写为正式的活动通知格式:",
            "service": "将以下内容改写为专业的服务说明:"
        }

        prompt = prompts.get(target_style, prompts["product"]) + "\n" + original_content

        # 这里应该调用LLM进行美化，实际使用时替换为真实LLM调用
        beautified = f"[美化后的内容] {original_content[:50]}..."

        return {
            "beautified": beautified,
            "style": target_style,
            "prompt": prompt,
            "language": language
        }


class AdActionService:
    """广告操作服务 - 删除/推送/封禁"""

    @classmethod
    async def delete_message(
        cls,
        bot,
        chat_id: int,
        message_id: int,
        reason: str = "广告违规"
    ) -> Dict[str, Any]:
        """删除广告消息"""
        try:
            await bot.delete_message(chat_id=chat_id, message_id=message_id)
            return {"success": True, "action": "deleted", "reason": reason}
        except Exception as e:
            logger.error(f"删除消息失败: {e}")
            return {"success": False, "error": str(e)}

    @classmethod
    async def ban_user(
        cls,
        bot,
        chat_id: int,
        user_id: int,
        duration_minutes: int = 60,
        reason: str = "广告违规"
    ) -> Dict[str, Any]:
        """禁言用户"""
        try:
            until_date = datetime.now() + timedelta(minutes=duration_minutes)
            await bot.ban_chat_member(
                chat_id=chat_id,
                user_id=user_id,
                until_date=until_date
            )
            return {"success": True, "action": "banned", "duration": duration_minutes}
        except Exception as e:
            logger.error(f"禁言失败: {e}")
            return {"success": False, "error": str(e)}

    @classmethod
    async def push_to_channel(
        cls,
        bot,
        channel_id: int,
        content: str,
        ad_id: int
    ) -> Dict[str, Any]:
        """推送到监控频道"""
        try:
            msg = await bot.send_message(
                chat_id=channel_id,
                text=f"🚨 **广告检测**\n\n{content}\n\nID: `{ad_id}`",
                parse_mode="Markdown"
            )
            return {"success": True, "message_id": msg.message_id}
        except Exception as e:
            logger.error(f"推送失败: {e}")
            return {"success": False, "error": str(e)}


from datetime import timedelta