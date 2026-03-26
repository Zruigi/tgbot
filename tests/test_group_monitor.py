import sys
import os
import asyncio
from unittest.mock import AsyncMock, MagicMock

# 将根目录加入路径
sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import database
from handlers.group_monitor import _get_group_config, _profile_tags_from_keywords, monitor_group_keywords

async def test_profile_tags_logic():
    # 测试关键词转画像标签
    matched = ["加入", "投资"]
    tags = _profile_tags_from_keywords(matched)
    print(f"Tags: {tags}")
    assert "拉新意向" in tags
    assert "投融资" in tags
    
    matched_custom = ["自定义"]
    tags_custom = _profile_tags_from_keywords(matched_custom)
    assert tags_custom == "自定义关键词"
    print("test_profile_tags_logic passed!")

async def test_group_config_cache():
    # 模拟数据库环境
    group_id = 123456789
    
    # 1. 初始状态（未开启监听）
    await database.upsert_tracked_group(group_id, "Test Group")
    await database.set_group_listen_status(group_id, False)
    
    cfg = await _get_group_config(group_id)
    assert cfg["is_monitored"] is False
    
    # 2. 开启监听
    await database.set_group_listen_status(group_id, True)
    from handlers.group_monitor import clear_group_cache
    clear_group_cache(group_id) # 必须清除缓存
    
    cfg = await _get_group_config(group_id)
    assert cfg["is_monitored"] is True
    print("test_group_config_cache passed!")

async def test_keyword_monitoring_flow():
    # 模拟 Telegram Update 和 Context
    mock_user = MagicMock()
    mock_user.id = 999
    mock_user.username = "test_user"
    mock_user.first_name = "Test"
    mock_user.last_name = "User"
    
    mock_chat = MagicMock()
    mock_chat.id = 888
    mock_chat.title = "Monitor Group"
    mock_chat.type = "supergroup"
    
    mock_message = MagicMock()
    mock_message.from_user = mock_user
    mock_message.chat = mock_chat
    mock_message.text = "我想加入这个项目，请问怎么投资？"
    
    mock_update = MagicMock()
    mock_update.message = mock_message
    
    mock_context = MagicMock()
    mock_context.bot.send_message = AsyncMock()
    
    # 确保群组已开启监听
    await database.upsert_tracked_group(mock_chat.id, mock_chat.title)
    await database.set_group_listen_status(mock_chat.id, True)
    from handlers.group_monitor import clear_group_cache
    clear_group_cache(mock_chat.id)
    
    # 执行监控逻辑
    await monitor_group_keywords(mock_update, mock_context)
    
    # 验证数据库中是否产生了潜在用户
    potential = await database.get_potential_users(limit=1)
    assert potential["total"] > 0
    assert potential["users"][0]["user_id"] == 999
    assert "加入" in potential["users"][0]["keywords"]
    assert "投资" in potential["users"][0]["keywords"]
    
    # 验证是否发送了私聊邀请
    mock_context.bot.send_message.assert_called()
    print("test_keyword_monitoring_flow passed!")

async def run_all_tests():
    try:
        await database.init_db() # 确保数据库初始化
        await test_profile_tags_logic()
        await test_group_config_cache()
        await test_keyword_monitoring_flow()
        print("\nAll Core logic tests passed!")
    except Exception as e:
        print(f"\nTests failed: {e}")
        import traceback
        traceback.print_exc()

if __name__ == "__main__":
    asyncio.run(run_all_tests())
