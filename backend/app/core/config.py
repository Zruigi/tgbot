# -*- coding: utf-8 -*-
"""
配置管理 - 从环境变量加载配置
"""
import os
from typing import Optional
from dotenv import load_dotenv

load_dotenv()


class Config:
    """应用配置"""
    
    # Telegram Bot
    BOT_TOKEN: str = os.getenv('BOT_TOKEN', '')
    
    # Web Admin
    WEB_ADMIN_TOKEN: str = os.getenv('WEB_ADMIN_TOKEN', '')
    
    # Database
    DB_NAME: str = os.getenv('DB_NAME', 'tgbot.db')
    
    # Security
    SECRET_KEY: str = os.getenv('SECRET_KEY', 'change-me-in-production')
    
    # OpenAI
    OPENAI_API_KEY: str = os.getenv('OPENAI_API_KEY', '')
    OPENAI_BASE_URL: str = os.getenv('OPENAI_BASE_URL', 'https://api.openai.com/v1')
    
    # Vertex AI
    VERTEX_API_KEY: str = os.getenv('VERTEX_API_KEY', '')
    
    # Server
    HOST: str = os.getenv('HOST', '0.0.0.0')
    PORT: int = int(os.getenv('PORT', '8080'))
    
    # Feature Flags
    ENABLE_GROUP_LISTEN: bool = os.getenv('ENABLE_GROUP_LISTEN', 'true').lower() == 'true'
    ENABLE_AI_CHAT: bool = os.getenv('ENABLE_AI_CHAT', 'true').lower() == 'true'
    
    @classmethod
    def verify_admin_token(cls, token: str) -> bool:
        """验证管理员 Token"""
        if not cls.WEB_ADMIN_TOKEN:
            return False
        return token == cls.WEB_ADMIN_TOKEN


# 快捷访问
config = Config()
