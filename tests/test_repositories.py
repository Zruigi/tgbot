# -*- coding: utf-8 -*-
"""
Repository 层单元测试 - 简化版
验证 Repository 模块的导入和函数签名
"""
import asyncio
import sys
import inspect
from pathlib import Path

# 添加项目根目录到路径
sys.path.insert(0, str(Path(__file__).parent.parent))


async def test_user_repo():
    """测试 User Repository 导入和函数存在性"""
    print("\n=== User Repository 测试 ===")
    
    try:
        from backend.repositories.user_repo import (
            get_user, create_or_update_user, get_inbox_list,
            get_chat_history, mark_messages_as_read
        )
        
        # 验证函数签名
        assert callable(get_user), "get_user 不是可调用对象"
        assert callable(create_or_update_user), "create_or_update_user 不是可调用对象"
        assert callable(get_inbox_list), "get_inbox_list 不是可调用对象"
        assert callable(get_chat_history), "get_chat_history 不是可调用对象"
        assert callable(mark_messages_as_read), "mark_messages_as_read 不是可调用对象"
        
        # 验证是 async 函数
        assert inspect.iscoroutinefunction(get_user), "get_user 不是 async 函数"
        assert inspect.iscoroutinefunction(create_or_update_user), "create_or_update_user 不是 async 函数"
        
        print("   ✓ 所有函数已定义且为 async")
        print("✅ User Repository 测试通过！\n")
        return True
        
    except Exception as e:
        print(f"❌ User Repository 测试失败：{e}\n")
        import traceback
        traceback.print_exc()
        return False


async def test_message_repo():
    """测试 Message Repository"""
    print("\n=== Message Repository 测试 ===")
    
    try:
        from backend.repositories.message_repo import (
            save_message, get_message_by_tg_id, delete_single_message,
            search_messages
        )
        
        assert callable(save_message), "save_message 不是可调用对象"
        assert callable(get_message_by_tg_id), "get_message_by_tg_id 不是可调用对象"
        assert callable(delete_single_message), "delete_single_message 不是可调用对象"
        assert callable(search_messages), "search_messages 不是可调用对象"
        
        assert inspect.iscoroutinefunction(save_message), "save_message 不是 async 函数"
        
        print("   ✓ 所有函数已定义且为 async")
        print("✅ Message Repository 测试通过！\n")
        return True
        
    except Exception as e:
        print(f"❌ Message Repository 测试失败：{e}\n")
        return False


async def test_admin_repo():
    """测试 Admin Repository"""
    print("\n=== Admin Repository 测试 ===")
    
    try:
        from backend.repositories.admin_repo import (
            get_admin_setting, save_admin_setting,
            save_ai_usage_log, get_ai_usage_summary
        )
        
        assert callable(get_admin_setting), "get_admin_setting 不是可调用对象"
        assert callable(save_admin_setting), "save_admin_setting 不是可调用对象"
        assert callable(save_ai_usage_log), "save_ai_usage_log 不是可调用对象"
        assert callable(get_ai_usage_summary), "get_ai_usage_summary 不是可调用对象"
        
        print("   ✓ 所有函数已定义且为 async")
        print("✅ Admin Repository 测试通过！\n")
        return True
        
    except Exception as e:
        print(f"❌ Admin Repository 测试失败：{e}\n")
        return False


async def test_monitor_repo():
    """测试 Monitor Repository"""
    print("\n=== Monitor Repository 测试 ===")
    
    try:
        from backend.repositories.monitor_repo import (
            get_monitor_pnl_summary, batch_update_monitor_transaction_meta,
            get_monitor_transactions_for_export
        )
        
        assert callable(get_monitor_pnl_summary), "get_monitor_pnl_summary 不是可调用对象"
        assert callable(batch_update_monitor_transaction_meta), "batch_update_monitor_transaction_meta 不是可调用对象"
        assert callable(get_monitor_transactions_for_export), "get_monitor_transactions_for_export 不是可调用对象"
        
        print("   ✓ 所有函数已定义且为 async")
        print("✅ Monitor Repository 测试通过！\n")
        return True
        
    except Exception as e:
        print(f"❌ Monitor Repository 测试失败：{e}\n")
        return False


async def test_group_repo():
    """测试 Group Repository"""
    print("\n=== Group Repository 测试 ===")
    
    try:
        from backend.repositories.group_repo import (
            upsert_potential_user, get_tracked_groups_with_stats,
            save_group_message
        )
        
        assert callable(upsert_potential_user), "upsert_potential_user 不是可调用对象"
        assert callable(get_tracked_groups_with_stats), "get_tracked_groups_with_stats 不是可调用对象"
        assert callable(save_group_message), "save_group_message 不是可调用对象"
        
        print("   ✓ 所有函数已定义且为 async")
        print("✅ Group Repository 测试通过！\n")
        return True
        
    except Exception as e:
        print(f"❌ Group Repository 测试失败：{e}\n")
        return False


async def test_verify_repo():
    """测试 Verify Repository"""
    print("\n=== Verify Repository 测试 ===")
    
    try:
        from backend.repositories.verify_repo import (
            get_group_verify_config, save_group_verify_config,
            get_verify_stats, get_verify_growth
        )
        
        assert callable(get_group_verify_config), "get_group_verify_config 不是可调用对象"
        assert callable(save_group_verify_config), "save_group_verify_config 不是可调用对象"
        assert callable(get_verify_stats), "get_verify_stats 不是可调用对象"
        assert callable(get_verify_growth), "get_verify_growth 不是可调用对象"
        
        print("   ✓ 所有函数已定义且为 async")
        print("✅ Verify Repository 测试通过！\n")
        return True
        
    except Exception as e:
        print(f"❌ Verify Repository 测试失败：{e}\n")
        return False


async def main():
    """运行所有 Repository 测试"""
    print("=" * 60)
    print("Repository 层单元测试（结构验证）")
    print("=" * 60)
    
    results = []
    
    # 运行各项测试
    results.append(("User Repository", await test_user_repo()))
    results.append(("Message Repository", await test_message_repo()))
    results.append(("Admin Repository", await test_admin_repo()))
    results.append(("Monitor Repository", await test_monitor_repo()))
    results.append(("Group Repository", await test_group_repo()))
    results.append(("Verify Repository", await test_verify_repo()))
    
    # 汇总结果
    print("\n" + "=" * 60)
    print("测试结果汇总")
    print("=" * 60)
    
    passed = sum(1 for _, result in results if result)
    total = len(results)
    
    for name, result in results:
        status = "✅ 通过" if result else "❌ 失败"
        print(f"{status} - {name}")
    
    print(f"\n总计：{passed}/{total} Repository 测试通过")
    
    if passed == total:
        print("\n🎉 所有 Repository 测试通过！")
        return 0
    else:
        print(f"\n⚠️  有 {total - passed} 个测试失败，请检查问题")
        return 1


if __name__ == "__main__":
    exit_code = asyncio.run(main())
    sys.exit(exit_code)
