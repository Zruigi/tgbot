FROM python:3.10-slim

# 安装PaddlePaddle和PaddleOCR
RUN pip install paddlepaddle==2.6.0 paddleocr==2.7.3 -i https://mirror.baidu.com/pypi/simple

WORKDIR /app

# 复制OCR服务代码
COPY ocr_server.py /app/

EXPOSE 8000

CMD ["python", "-m", "uvicorn", "ocr_server:app", "--host", "0.0.0.0", "--port", "8000"]