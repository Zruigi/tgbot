# -*- coding: utf-8 -*-
"""
PaddleOCR 集成服务
"""
import base64
import logging
from typing import Dict, Any, Optional
import aiohttp

logger = logging.getLogger(__name__)


class PaddleOCRService:
    """PaddleOCR 服务客户端"""

    def __init__(self, base_url: str = "http://localhost:8000"):
        self.base_url = base_url

    async def detect_text(
        self,
        image_url: Optional[str] = None,
        image_base64: Optional[str] = None,
        use_angle_cls: bool = True,
        lang: str = "ch"
    ) -> Dict[str, Any]:
        """
        检测图片中的文字

        Args:
            image_url: 图片URL
            image_base64: Base64编码的图片
            use_angle_cls: 是否使用方向分类器
            lang: 语言 (ch/en/korean/japanese)

        Returns:
            {
                "success": bool,
                "texts": list,
                "boxes": list,
                "scores": list
            }
        """
        payload = {
            "use_angle_cls": use_angle_cls,
            "lang": lang
        }

        if image_url:
            payload["image_url"] = image_url
        elif image_base64:
            payload["image"] = image_base64
        else:
            return {"success": False, "error": "需要提供image_url或image_base64"}

        try:
            async with aiohttp.ClientSession() as session:
                async with session.post(
                    f"{self.base_url}/ocr/system",
                    json=payload,
                    timeout=aiohttp.ClientTimeout(total=30)
                ) as resp:
                    if resp.status == 200:
                        data = await resp.json()
                        return {
                            "success": True,
                            "texts": data.get("data", []),
                            "raw_response": data
                        }
                    else:
                        return {"success": False, "error": f"HTTP {resp.status}"}
        except Exception as e:
            logger.error(f"PaddleOCR请求失败: {e}")
            return {"success": False, "error": str(e)}

    async def detect_ad_from_image(
        self,
        image_url: Optional[str] = None,
        image_base64: Optional[str] = None
    ) -> Dict[str, Any]:
        """
        从图片中检测广告内容

        1. OCR识别文字
        2. 调用广告检测服务
        """
        # 1. OCR识别
        ocr_result = await self.detect_text(image_url, image_base64)
        if not ocr_result.get("success"):
            return {"success": False, "error": "OCR识别失败", "details": ocr_result}

        texts = ocr_result.get("texts", [])
        full_text = "\n".join([t.get("text", "") for t in texts if t.get("text")])

        if not full_text.strip():
            return {"success": False, "error": "未识别到文字"}

        # 2. 广告检测
        from backend.app.services.ad_service import AdDetectionService
        ad_result = await AdDetectionService.detect(full_text, "group_message")

        return {
            "success": True,
            "ocr_texts": texts,
            "full_text": full_text,
            "ad_detection": ad_result
        }


# 全局单例
_paddle_ocr_service: Optional[PaddleOCRService] = None


def get_paddle_ocr_service(base_url: str = "http://paddleocr:8000") -> PaddleOCRService:
    global _paddle_ocr_service
    if _paddle_ocr_service is None:
        _paddle_ocr_service = PaddleOCRService(base_url)
    return _paddle_ocr_service