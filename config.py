# -*- coding: utf-8 -*-
"""
从 .env 文件加载配置
"""
import os
from dotenv import load_dotenv

# 加载 .env 文件中的环境变量
load_dotenv()


def _first_non_empty_env(*keys: str, default: str = "") -> str:
    """按顺序读取环境变量，返回第一个非空值。"""
    for key in keys:
        value = os.getenv(key)
        if value is not None and str(value).strip():
            return str(value).strip()
    return default

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
# 兼容旧命名：DMV(=TRC20 TronGrid key), BC(=BSCScan key)
TRONGRID_API_KEY = _first_non_empty_env(
    "TRONGRID_API_KEY",
    "DMV",
    "DMV_API_KEY",
    "DMV_API_TOKEN",
    "DMV_TOKEN",
)
BSCSCAN_API_KEY = _first_non_empty_env(
    "BSCSCAN_API_KEY",
    "BC",
    "BC_API_KEY",
    "BC_API_TOKEN",
    "BC_TOKEN",
)

# --- 币安 API (公开，无需 Key) ---
# CCXT 会自动使用币安的公共 API，这里无需特别设置

# --- 数据库配置 ---
BASE_DIR = os.path.dirname(os.path.abspath(__file__))
# 支持通过环境变量 DB_NAME 覆盖（多租户部署时由主控平台注入）
DB_NAME = os.getenv("DB_NAME") or os.path.join(BASE_DIR, "crypto_bot.sqlite3")

# --- 日志配置 ---
DEFAULT_LOG_DIR = "/home/ubuntu/tgbot/logs"
LOG_DIR = os.getenv("LOG_DIR", DEFAULT_LOG_DIR)
if not os.path.isabs(LOG_DIR):
    LOG_DIR = os.path.abspath(os.path.join(BASE_DIR, LOG_DIR))
LOG_RETENTION_HOURS = int(os.getenv("LOG_RETENTION_HOURS", "48"))

LOG_APP_DIR = os.path.join(LOG_DIR, "app")
LOG_BOT_DIR = os.path.join(LOG_APP_DIR, "bot")
LOG_BACKEND_DIR = os.path.join(LOG_APP_DIR, "backend")
LOG_TUNNEL_DIR = os.path.join(LOG_DIR, "tunnel")
LOG_CLOUDFLARED_DIR = os.path.join(LOG_TUNNEL_DIR, "cloudflared")
LOG_FRONTEND_DIR = os.path.join(LOG_DIR, "frontend")
LOG_VITE_DIR = os.path.join(LOG_FRONTEND_DIR, "vite")
LOG_AI_DIR = os.path.join(LOG_DIR, "ai")
LOG_LITELLM_DIR = os.path.join(LOG_AI_DIR, "litellm")
LOG_RUNTIME_DIR = os.path.join(LOG_DIR, "runtime")
LOG_PROCESS_DIR = os.path.join(LOG_RUNTIME_DIR, "process")

BOT_LOG_PATH = os.path.join(LOG_BOT_DIR, "bot.log")
BACKEND_LOG_PATH = os.path.join(LOG_BACKEND_DIR, "backend.log")
CLOUDFLARED_LOG_PATH = os.path.join(LOG_CLOUDFLARED_DIR, "cloudflared.log")
CLOUDFLARED_STDOUT_PATH = os.path.join(LOG_CLOUDFLARED_DIR, "cloudflared.stdout")
FRONTEND_LOG_PATH = os.path.join(LOG_VITE_DIR, "frontend.stdout")
LITELLM_LOG_PATH = os.path.join(LOG_LITELLM_DIR, "litellm.log")
DEV_PID_PATH = os.path.join(LOG_PROCESS_DIR, "dev.pids")

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
N8N_GROUP_MESSAGE_WEBHOOK_URL = os.getenv("N8N_GROUP_MESSAGE_WEBHOOK_URL", "").strip()

# --- Telegram 广告检测聚合管线 ---
AD_PIPELINE_ENABLED = os.getenv("AD_PIPELINE_ENABLED", "false").strip().lower() in {"1", "true", "yes", "on"}
AD_PIPELINE_PG_DSN = os.getenv("AD_PIPELINE_PG_DSN", "").strip()
OLLAMA_BASE_URL = os.getenv("OLLAMA_BASE_URL", "http://localhost:11434").strip().rstrip("/")
OLLAMA_EMBED_MODEL = os.getenv("OLLAMA_EMBED_MODEL", "nomic-embed-text").strip()
AD_SIMILARITY_THRESHOLD = float(os.getenv("AD_SIMILARITY_THRESHOLD", "0.92"))
AD_MIN_DUPLICATE_COUNT = int(os.getenv("AD_MIN_DUPLICATE_COUNT", "2"))
AD_DELETE_ENABLED = os.getenv("AD_DELETE_ENABLED", "false").strip().lower() in {"1", "true", "yes", "on"}
AD_KICK_ENABLED = os.getenv("AD_KICK_ENABLED", "false").strip().lower() in {"1", "true", "yes", "on"}
AD_EMBEDDING_DIM = int(os.getenv("AD_EMBEDDING_DIM", "768"))

# --- 安全加固配置 ---
import hashlib as _hashlib
import time as _time

class SecurityConfig:
    """安全相关配置，集中管理凭证哈希与速率限制参数"""
    # 存储 WEB_ADMIN_TOKEN 的 SHA256 哈希（避免明文比较泄露原始 Token）
    # 若环境变量 ADMIN_TOKEN_HASH 存在则使用，否则在运行时动态生成（兼容旧部署）
    ADMIN_TOKEN_HASH: str = os.getenv(
        "ADMIN_TOKEN_HASH",
        _hashlib.sha256(os.getenv("WEB_ADMIN_TOKEN", "bot_admin_secret_2026").encode()).hexdigest()
    )
    TOKEN_EXPIRY_DAYS: int = 90
    OTP_EXPIRY_SECONDS: int = 300       # 5 分钟
    MAX_LOGIN_ATTEMPTS: int = 5         # 登录速率限制：5次/分钟
    RATE_LIMIT_WINDOW: int = 60         # 速率限制窗口（秒）
    RATE_LIMIT_MAX_CALLS: int = 60      # 通用 API 调用上限（次/分钟）

def verify_admin_token(token: str) -> bool:
    """使用 SHA256 哈希比较校验 Admin Token，防止时序攻击"""
    import hmac
    token_hash = _hashlib.sha256(token.encode()).hexdigest()
    return hmac.compare_digest(token_hash, SecurityConfig.ADMIN_TOKEN_HASH)

def _detect_web_base_url() -> str:
    """Web 地址解析：环境变量优先，探测次之，最后回退本地地址"""
    # 1) FRONTEND_URL 显式配置优先（防止隧道环境误探测到 127.0.0.1）
    frontend_env = os.getenv("FRONTEND_URL", "").strip()
    if frontend_env:
        return frontend_env.rstrip("/")

    # 2) WEB_BASE_URL 显式配置优先于自动探测
    web_base_env = os.getenv("WEB_BASE_URL", "").strip()
    if web_base_env:
        return web_base_env.rstrip("/")

    # 3) 再尝试从 cloudflared.log 探测最新 trycloudflare URL
    try:
        log_path = CLOUDFLARED_LOG_PATH
        if os.path.exists(log_path):
            import re
            with open(log_path, "r", encoding="utf-8", errors="ignore") as f:
                content = f.read()
            matches = re.findall(r'https://[\w\-]+\.trycloudflare\.com', content)
            if matches:
                return matches[-1].rstrip("/")
    except Exception:
        pass

    return "http://127.0.0.1:80"

WEB_BASE_URL = _detect_web_base_url()

# 前端 UI 地址（优先使用显式 FRONTEND_URL）
FRONTEND_URL = os.getenv("FRONTEND_URL", "").strip().rstrip("/") or WEB_BASE_URL

# --- 管理员 TG ID 列表 (支持多管理员) ---
# 默认包含 ADMIN_ID，可通过 ADMIN_TG_IDS 环境变量追加（逗号分隔）
def _parse_admin_ids():
    ids = set()
    if ADMIN_ID:
        ids.add(ADMIN_ID)
    extra = os.getenv("ADMIN_TG_IDS", "")
    for s in extra.split(","):
        s = s.strip()
        if s.isdigit():
            ids.add(int(s))
    return ids

ADMIN_TG_IDS: set = set()  # populated after ADMIN_ID is confirmed valid

# --- 校验配置 ---
if not BOT_TOKEN or not ADMIN_ID:
    raise ValueError("请在 .env 文件中设置 BOT_TOKEN 和 ADMIN_ID")

# Populate ADMIN_TG_IDS after validation
ADMIN_TG_IDS = _parse_admin_ids()
