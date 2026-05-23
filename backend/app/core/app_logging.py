# -*- coding: utf-8 -*-
"""
日志配置
"""
import logging
import sys
from datetime import datetime


def setup_logging(level: str = 'INFO') -> None:
    """配置应用日志"""
    
    # 创建 formatter
    formatter = logging.Formatter(
        fmt='%(asctime)s [%(levelname)s] %(name)s: %(message)s',
        datefmt='%Y-%m-%d %H:%M:%S'
    )
    
    # 控制台处理器
    console_handler = logging.StreamHandler(sys.stdout)
    console_handler.setFormatter(formatter)
    
    # 根日志器
    root_logger = logging.getLogger()
    root_logger.setLevel(getattr(logging, level.upper()))
    root_logger.addHandler(console_handler)
    
    # 抑制第三方库的调试日志
    logging.getLogger('aiohttp').setLevel(logging.WARNING)
    logging.getLogger('telegram').setLevel(logging.WARNING)
    logging.getLogger('aiosqlite').setLevel(logging.WARNING)
    logging.getLogger('httpx').setLevel(logging.WARNING)
    logging.getLogger('httpcore').setLevel(logging.WARNING)


def get_logger(name: str) -> logging.Logger:
    """获取命名日志器"""
    return logging.getLogger(name)
