# -*- coding: utf-8 -*-
"""
从 .env 文件加载配置
"""
import os
from dotenv import load_dotenv

# 加载 .env 文件中的环境变量
load_dotenv()

# --- Telegram Bot 配置 ---
BOT_TOKEN = os.getenv("BOT_TOKEN", "")
ADMIN_ID = int(os.getenv("ADMIN_ID", "0"))

# --- 进程内标志（由 /restart_bot 设置，main 在 run_polling 结束后 os.execl 重启）---
RESTART_REQUESTED = False

# --- 邮件服务 (SMTP) 配置 ---
SMTP_SERVER = os.getenv("SMTP_SERVER", "smtp.gmail.com")
SMTP_PORT = int(os.getenv("SMTP_PORT", 465))
SMTP_USER = os.getenv("SMTP_USER", "")
SMTP_PASS = os.getenv("SMTP_PASS", "")

# --- 区块链 API 配置 ---
TRONGRID_API_KEY = os.getenv("TRONGRID_API_KEY", "")
BSCSCAN_API_KEY = os.getenv("BSCSCAN_API_KEY", "")

# --- 币安 API (公开，无需 Key) ---
# CCXT 会自动使用币安的公共 API，这里无需特别设置

# --- 数据库配置 ---
BASE_DIR = os.path.dirname(os.path.abspath(__file__))
# 支持通过环境变量 DB_NAME 覆盖（多租户部署时由主控平台注入）
DB_NAME = os.getenv("DB_NAME") or os.path.join(BASE_DIR, "crypto_bot.sqlite3")

# --- OCR 配置 (用于地址截图验证) ---
# TESSERACT_CMD: Tesseract 执行文件路径。
# 如果为 None，系统会尝试自动探测 (Windows 常见路径或 Linux 默认路径)。
TESSERACT_CMD = os.getenv("TESSERACT_CMD", None)

# --- 大模型 API 配置 (用于智能翻译) ---
# LLM API 配置
LLM_API_KEY = os.getenv("LLM_API_KEY") or None
LLM_BASE_URL = os.getenv("LLM_BASE_URL", "https://api.openai.com/v1")
LLM_MODEL_NAME = os.getenv("LLM_MODEL_NAME", "gpt-3.5-turbo")

# Gemini API Key
GEMINI_API_KEY = os.getenv("GEMINI_API_KEY", "")
# DeepSeek API Key
DEEPSEEK_API_KEY = os.getenv("DEEPSEEK_API_KEY", "")
# 默认使用的大模型 (gemini 或 deepseek)
DEFAULT_LLM = os.getenv("DEFAULT_LLM", "gemini")

# --- Web 管理端配置 ---
WEB_ADMIN_TOKEN = os.getenv("WEB_ADMIN_TOKEN", "bot_admin_secret_2026")
WEB_BASE_URL = os.getenv("WEB_BASE_URL", "http://127.0.0.1:8080")

# --- 校验配置 ---
if not BOT_TOKEN or not ADMIN_ID:
    raise ValueError("请在 .env 文件中设置 BOT_TOKEN 和 ADMIN_ID")
