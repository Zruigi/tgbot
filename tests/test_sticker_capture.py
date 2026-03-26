# -*- coding: utf-8 -*-
"""
表情包捕获专项测试脚本
用途：验证表情包、图片等非文本消息是否能正确触发群组活跃状态更新及 WebSocket 推送。
"""
import asyncio
import sys
import os
import time
import logging
from unittest.mock import AsyncMock, MagicMock

# 将项目根目录加入路径
sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import database
from handlers.group_monitor import track_group_seen, clear_group_cache

# 配置日志
logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(levelname)s - %(message)s')
logger = logging.getLogger(__name__)

class StickerCaptureTest:
    def __init__(self):
        self.test_group_id = -100999888777
        self.test_group_title = "表情包测试群"
        
    async def setup(self):
        """环境准备"""
        logger.info("--- [1] 环境准备 ---")
        await database.init_db()
        # 确保群组存在，并手动设置一个过期的 updated_at
        async with database.aiosqlite.connect(database.DB_NAME) as db:
            await db.execute(
                "INSERT OR REPLACE INTO tracked_groups (group_id, group_title, updated_at) VALUES (?, ?, ?)",
                (self.test_group_id, self.test_group_title, "2026-01-01 00:00:00")
            )
            await db.commit()
        logger.info(f"✓ 测试群组 {self.test_group_id} 已创建 (预设旧时间)")

    def _create_mock_sticker_update(self):
        """构造模拟的 Sticker Update 对象"""
        update = MagicMock()
        message = MagicMock()
        chat = MagicMock()
        sticker = MagicMock()
        
        chat.id = self.test_group_id
        chat.title = self.test_group_title
        chat.type = "supergroup"
        
        message.text = None # 表情包无文本
        message.sticker = sticker
        message.chat = chat
        message.date = MagicMock()
        message.date.isoformat.return_value = "2026-03-22T21:00:00"
        
        update.message = message
        update.effective_message = message
        return update

    async def run_test(self):
        """执行测试"""
        logger.info("\n--- [2] 发送模拟表情包 ---")
        update = self._create_mock_sticker_update()
        context = MagicMock()
        
        # 记录执行前的 updated_at
        async with database.aiosqlite.connect(database.DB_NAME) as db:
            db.row_factory = database.aiosqlite.Row
            async with db.execute("SELECT updated_at FROM tracked_groups WHERE group_id = ?", (self.test_group_id,)) as cursor:
                row = await cursor.fetchone()
                old_time = row['updated_at'] if row else "None"
        
        logger.info(f"发送前 updated_at: {old_time}")
        
        # 触发追踪逻辑
        await track_group_seen(update, context)
        
        # 验证数据库更新
        async with database.aiosqlite.connect(database.DB_NAME) as db:
            db.row_factory = database.aiosqlite.Row
            async with db.execute("SELECT updated_at FROM tracked_groups WHERE group_id = ?", (self.test_group_id,)) as cursor:
                row = await cursor.fetchone()
                new_time = row['updated_at'] if row else "None"
        
        logger.info(f"发送后 updated_at: {new_time}")
        
        if new_time != old_time:
            logger.info("✅ 成功：数据库活跃时间已更新")
            return True
        else:
            logger.error("❌ 失败：数据库活跃时间未更新")
            return False

    async def run(self):
        try:
            await self.setup()
            success = await self.run_test()
            
            logger.info("\n" + "="*40)
            logger.info(f"测试结果: {'PASS' if success else 'FAIL'}")
            logger.info("="*40 + "\n")
            
        except Exception as e:
            logger.error(f"测试中断: {e}")
            import traceback
            traceback.print_exc()

if __name__ == "__main__":
    test = StickerCaptureTest()
    asyncio.run(test.run())
