#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
财务 ERP 集成启动脚本
在 main.py 启动前运行此脚本来初始化财务模块
"""
import asyncio
import logging
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
if str(PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(PROJECT_ROOT))

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

async def init_finance():
    """初始化财务 ERP 模块"""
    try:
        logger.info("🚀 开始初始化财务 ERP 模块...")
        
        # 初始化数据库
        from database import init_finance_module
        await init_finance_module()
        
        logger.info("✅ 财务 ERP 模块初始化完成！")
        logger.info("📝 接下来的步骤:")
        logger.info("   1. 在 main.py 中调用 await register_finance_routes(app)")
        logger.info("   2. 在前端路由中添加 FinancePanel.vue")
        logger.info("   3. 在主菜单中添加 💼 企业财务 链接")
        logger.info("   4. 访问 http://localhost:8080/api/finance/chart_of_accounts 测试 API")
        
        return True
    except Exception as e:
        logger.error(f"❌ 初始化失败: {e}", exc_info=True)
        return False

if __name__ == "__main__":
    result = asyncio.run(init_finance())
    sys.exit(0 if result else 1)
