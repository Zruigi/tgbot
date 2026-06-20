# -*- coding: utf-8 -*-
"""
PaddleOCR Server
"""
from fastapi import FastAPI, UploadFile, File, Form
from fastapi.responses import JSONResponse
from PIL import Image
import io
import base64
from paddleocr import PaddleOCR
import logging

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

app = FastAPI(title="PaddleOCR Service", version="2.0")

# 初始化OCR (首次使用下载模型)
# use_angle_cls=True 启用方向分类
# lang='ch' 中文
ocr = PaddleOCR(use_angle_cls=True, lang='ch', show_log=False)


@app.get("/health")
def health():
    return {"status": "ok"}


@app.post("/ocr/system")
async def ocr_system(
    image: UploadFile = File(None),
    image_url: str = Form(None),
    use_angle_cls: bool = Form(True),
    lang: str = Form("ch")
):
    """OCR识别接口"""
    try:
        img_bytes = None

        # 从文件上传
        if image:
            img_bytes = await image.read()

        # 从URL下载 (简化版)
        elif image_url:
            import requests
            resp = requests.get(image_url, timeout=10)
            img_bytes = resp.content

        if not img_bytes:
            return JSONResponse({"status": "error", "message": "No image provided"}, status_code=400)

        # 识别
        result = ocr.ocr(img_bytes, cls=use_angle_cls)

        # 解析结果
        texts = []
        if result and result[0]:
            for line in result[0]:
                box = line[0]
                text = line[1][0]
                score = line[1][1]
                texts.append({
                    "text": text,
                    "score": float(score),
                    "box": box
                })

        return {"status": "success", "data": texts}

    except Exception as e:
        logger.error(f"OCR error: {e}")
        return {"status": "error", "message": str(e)}


@app.post("/ocr/base64")
async def ocr_base64(image: str = Form(...)):
    """Base64图片OCR"""
    try:
        # 解码Base64
        img_bytes = base64.b64decode(image)

        result = ocr.ocr(img_bytes, cls=True)

        texts = []
        if result and result[0]:
            for line in result[0]:
                texts.append({
                    "text": line[1][0],
                    "score": float(line[1][1])
                })

        return {"status": "success", "data": texts}

    except Exception as e:
        return {"status": "error", "message": str(e)}


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="0.0.0.0", port=8000)