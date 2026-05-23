# -*- coding: utf-8 -*-
"""
架构重构兼容性测试脚本
验证新的 Repository 层和 API 模块与现有代码的兼容性
"""
import asyncio
import sys
import logging
from pathlib import Path

# 添加项目根目录到路径
sys.path.insert(0, str(Path(__file__).parent.parent))

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


async def test_repository_imports():
    """测试 Repository 层导入"""
    print("\n=== 测试 Repository 层导入 ===")
    
    try:
        from backend.repositories.user_repo import (
            get_user, create_or_update_user, get_inbox_list,
            get_chat_history, mark_messages_as_read
        )
        print("✅ user_repo 导入成功")
        
        from backend.repositories.message_repo import (
            save_message, get_message_by_tg_id, delete_single_message
        )
        print("✅ message_repo 导入成功")
        
        from backend.repositories.admin_repo import (
            get_admin_setting, save_admin_setting, get_ai_usage_summary
        )
        print("✅ admin_repo 导入成功")
        
        from backend.repositories.monitor_repo import (
            get_active_monitors, get_monitor_pnl_summary
        )
        print("✅ monitor_repo 导入成功")
        
        from backend.repositories.group_repo import (
            get_tracked_groups_with_stats, get_potential_tags
        )
        print("✅ group_repo 导入成功")
        
        print("✅ 所有 Repository 模块导入成功！\n")
        return True
        
    except ImportError as e:
        print(f"❌ Repository 导入失败：{e}\n")
        return False


async def test_api_imports():
    """测试 API 模块导入"""
    print("\n=== 测试 API 模块导入 ===")
    
    try:
        from backend.app.api.auth import api_login, api_login_otp
        print("✅ auth.py 导入成功")
        
        from backend.app.api.chat import api_get_users, api_get_messages
        print("✅ chat.py 导入成功")
        
        from backend.app.api.config import api_get_config
        print("✅ config.py 导入成功")
        
        from backend.app.api.group import api_list_groups
        print("✅ group.py 导入成功")
        
        from backend.app.api.broadcast import api_get_broadcast_targets
        print("✅ broadcast.py 导入成功")
        
        from backend.app.api.ai import api_ai_chat
        print("✅ ai.py 导入成功")
        
        from backend.app.api.monitor import api_list_monitors_wrapper
        print("✅ monitor.py 导入成功")
        
        from backend.app.api.potential import api_get_potential_users
        print("✅ potential.py 导入成功")
        
        print("✅ 所有 API 模块导入成功！\n")
        return True
        
    except ImportError as e:
        print(f"❌ API 模块导入失败：{e}\n")
        return False


async def test_core_services():
    """测试核心服务"""
    print("\n=== 测试核心服务 ===")
    
    try:
        from backend.app.core.security import (
            TokenManager, RateLimiter, SecurityMiddleware
        )
        print("✅ security.py 导入成功")
        
        # 测试 TokenManager
        tm = TokenManager()
        otp = tm.generate_otp()
        assert len(otp) > 0, "OTP 生成失败"
        print(f"   ✓ OTP 生成测试通过：{otp[:10]}...")
        
        # 测试速率限制器
        rl = RateLimiter(window_seconds=60, max_attempts=5)
        assert rl.check_limit("test_ip") == True, "速率限制测试失败"
        print("   ✓ 速率限制测试通过")
        
        from backend.app.services.bot_engine import BotEngine, get_bot_engine
        print("✅ bot_engine.py 导入成功")
        
        print("✅ 所有核心服务测试通过！\n")
        return True
        
    except Exception as e:
        print(f"❌ 核心服务测试失败：{e}\n")
        return False


async def test_utility_functions():
    """测试工具函数"""
    print("\n=== 测试工具函数 ===")
    
    try:
        from backend.app.utils.channel_utils import (
            normalize_channel_link, validate_channel_link,
            parse_channel_username, validate_wallet_address
        )
        print("✅ channel_utils.py 导入成功")
        
        # 测试频道链接规范化
        test_links = [
            ('@channel', 'https://t.me/channel'),
            ('t.me/news', 'https://t.me/news'),
            ('https://t.me/telegram', 'https://t.me/telegram'),
        ]
        
        for input_link, expected in test_links:
            result = normalize_channel_link(input_link)
            assert result == expected, f"期望 {expected}, 得到 {result}"
            print(f"   ✓ 链接规范化测试通过：{input_link} → {result}")
        
        # 测试钱包地址验证
        # TRC20 地址使用 base58 字符集 (不含 0OIl)
        assert validate_wallet_address('TJRyWwFsH7TFGUCVFnQpXjdZbJnKJzKvLd', 'TRC20') == True
        assert validate_wallet_address('0x1234567890abcdef1234567890abcdef12345678', 'BSC') == True
        print("   ✓ 钱包地址验证测试通过")
        
        print("✅ 所有工具函数测试通过！\n")
        return True
        
    except Exception as e:
        print(f"❌ 工具函数测试失败：{e}\n")
        return False


async def test_database_compatibility():
    """测试 database.py 兼容性层"""
    print("\n=== 测试 database.py 兼容性 ===")
    
    try:
        # 测试旧代码仍能正常工作
        import database
        
        # 检查兼容性导出是否存在（直接 re-export 的仓库函数名）
        assert hasattr(database, 'get_user'), "缺少 get_user 兼容导出"
        assert hasattr(database, 'save_message'), "缺少 save_message 兼容导出"
        assert hasattr(database, 'get_admin_setting'), "缺少 get_admin_setting 兼容导出"
        
        print("✅ database.py 兼容性层工作正常！\n")
        return True
        
    except Exception as e:
        print(f"❌ 兼容性层测试失败：{e}\n")
        return False


async def main():
    """主测试函数"""
    print("=" * 60)
    print("架构重构兼容性测试")
    print("=" * 60)
    
    results = []
    
    # 运行所有测试
    results.append(("Repository 层导入", await test_repository_imports()))
    results.append(("API 模块导入", await test_api_imports()))
    results.append(("核心服务", await test_core_services()))
    results.append(("工具函数", await test_utility_functions()))
    results.append(("兼容性层", await test_database_compatibility()))
    
    # 汇总结果
    print("\n" + "=" * 60)
    print("测试结果汇总")
    print("=" * 60)
    
    passed = sum(1 for _, result in results if result)
    total = len(results)
    
    for name, result in results:
        status = "✅ 通过" if result else "❌ 失败"
        print(f"{status} - {name}")
    
    print(f"\n总计：{passed}/{total} 测试通过")
    
    if passed == total:
        print("\n🎉 所有兼容性测试通过！架构重构成功！")
        return 0
    else:
        print(f"\n⚠️ 有 {total - passed} 个测试失败，请检查问题")
        return 1


if __name__ == '__main__':
    exit_code = asyncio.run(main())
    sys.exit(exit_code)
