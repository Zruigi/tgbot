# -*- coding: utf-8 -*-
"""
API 层单元测试 - 验证 API 函数签名和导入
"""
import asyncio
import sys
import inspect
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent))


async def test_auth_api():
    """测试认证 API"""
    print("\n=== Auth API 测试 ===")
    
    try:
        from backend.app.api.auth import (
            api_login, api_login_otp, api_get_csrf_token, api_csp_report,
            generate_otp, generate_csrf_token, auth_middleware
        )
        
        assert callable(api_login), "api_login 不是可调用对象"
        assert callable(api_login_otp), "api_login_otp 不是可调用对象"
        assert callable(api_get_csrf_token), "api_get_csrf_token 不是可调用对象"
        assert callable(api_csp_report), "api_csp_report 不是可调用对象"
        assert callable(generate_otp), "generate_otp 不是可调用对象"
        assert callable(generate_csrf_token), "generate_csrf_token 不是可调用对象"
        assert callable(auth_middleware), "auth_middleware 不是可调用对象"
        
        assert inspect.iscoroutinefunction(api_login), "api_login 不是 async 函数"
        assert inspect.iscoroutinefunction(api_login_otp), "api_login_otp 不是 async 函数"
        
        print("   ✓ 所有函数已定义且为 async")
        print("✅ Auth API 测试通过！\n")
        return True
        
    except Exception as e:
        print(f"❌ Auth API 测试失败：{e}\n")
        return False


async def test_chat_api():
    """测试聊天 API"""
    print("\n=== Chat API 测试 ===")
    
    try:
        from backend.app.api.chat import (
            api_get_users, api_get_messages, api_send_message,
            api_delete_single_message, api_mark_read, api_delete_chat, api_search
        )
        
        for func in [api_get_users, api_get_messages, api_send_message, 
                     api_delete_single_message, api_mark_read, api_delete_chat, api_search]:
            assert callable(func), f"{func.__name__} 不是可调用对象"
            assert inspect.iscoroutinefunction(func), f"{func.__name__} 不是 async 函数"
        
        print("   ✓ 所有函数已定义且为 async")
        print("✅ Chat API 测试通过！\n")
        return True
        
    except Exception as e:
        print(f"❌ Chat API 测试失败：{e}\n")
        return False


async def test_group_api():
    """测试群组 API"""
    print("\n=== Group API 测试 ===")
    
    try:
        from backend.app.api.group import (
            api_list_groups, api_toggle_group_listen, api_get_group_members,
            api_get_group_messages, api_get_group_verify_config,
            api_save_group_verify_config, api_get_verify_stats, api_get_verify_growth
        )
        
        for func in [api_list_groups, api_toggle_group_listen, api_get_group_members,
                     api_get_group_messages, api_get_group_verify_config,
                     api_save_group_verify_config, api_get_verify_stats, api_get_verify_growth]:
            assert callable(func), f"{func.__name__} 不是可调用对象"
            assert inspect.iscoroutinefunction(func), f"{func.__name__} 不是 async 函数"
        
        print("   ✓ 所有函数已定义且为 async")
        print("✅ Group API 测试通过！\n")
        return True
        
    except Exception as e:
        print(f"❌ Group API 测试失败：{e}\n")
        return False


async def test_monitor_api():
    """测试监控 API"""
    print("\n=== Monitor API 测试 ===")
    
    try:
        from backend.app.api.monitor import (
            api_list_monitors_wrapper, api_create_monitor_wrapper,
            api_delete_monitor_wrapper, api_update_monitor_remark_wrapper,
            api_monitor_pnl_wrapper, api_update_tx_meta_wrapper,
            api_batch_update_tx_meta_wrapper, api_export_wrapper,
            api_monitor_stats_wrapper
        )
        
        for func in [api_list_monitors_wrapper, api_create_monitor_wrapper,
                     api_delete_monitor_wrapper, api_update_monitor_remark_wrapper,
                     api_monitor_pnl_wrapper, api_update_tx_meta_wrapper,
                     api_batch_update_tx_meta_wrapper, api_export_wrapper,
                     api_monitor_stats_wrapper]:
            assert callable(func), f"{func.__name__} 不是可调用对象"
            assert inspect.iscoroutinefunction(func), f"{func.__name__} 不是 async 函数"
        
        print("   ✓ 所有函数已定义且为 async")
        print("✅ Monitor API 测试通过！\n")
        return True
        
    except Exception as e:
        print(f"❌ Monitor API 测试失败：{e}\n")
        return False


async def test_broadcast_api():
    """测试广播 API"""
    print("\n=== Broadcast API 测试 ===")
    
    try:
        from backend.app.api.broadcast import (
            api_get_broadcast_targets, api_sync_broadcast_targets,
            api_broadcast_unified, api_list_push_tasks, api_create_push_task,
            api_update_push_task_status
        )
        
        for func in [api_get_broadcast_targets, api_sync_broadcast_targets,
                     api_broadcast_unified, api_list_push_tasks, api_create_push_task,
                     api_update_push_task_status]:
            assert callable(func), f"{func.__name__} 不是可调用对象"
            assert inspect.iscoroutinefunction(func), f"{func.__name__} 不是 async 函数"
        
        print("   ✓ 所有函数已定义且为 async")
        print("✅ Broadcast API 测试通过！\n")
        return True
        
    except Exception as e:
        print(f"❌ Broadcast API 测试失败：{e}\n")
        return False


async def test_finance_module():
    """测试财务 API"""
    print("\n=== Finance API 测试 ===")
    
    try:
        from backend.app.api.finance import (
            api_get_finance_summary, api_reconcile_finance,
            api_get_finance_categories, api_calculate_pnl,
            api_generate_finance_report
        )
        
        for func in [api_get_finance_summary, api_reconcile_finance,
                     api_get_finance_categories, api_calculate_pnl,
                     api_generate_finance_report]:
            assert callable(func), f"{func.__name__} 不是可调用对象"
            assert inspect.iscoroutinefunction(func), f"{func.__name__} 不是 async 函数"
        
        print("   ✓ 所有函数已定义且为 async")
        print("✅ Finance API 测试通过！\n")
        return True
        
    except Exception as e:
        print(f"❌ Finance API 测试失败：{e}\n")
        return False


async def test_services():
    """测试 Services 层"""
    print("\n=== Services 层测试 ===")
    
    try:
        from backend.app.services.ai_service import AIService, get_ai_service
        from backend.app.services.broadcast_service import BroadcastService, get_broadcast_service
        from backend.app.services.finance_calc import FinanceCalcService
        from backend.app.services.pusher import PusherService, get_pusher_service
        
        # 测试类存在
        assert AIService is not None, "AIService 未定义"
        assert BroadcastService is not None, "BroadcastService 未定义"
        assert FinanceCalcService is not None, "FinanceCalcService 未定义"
        assert PusherService is not None, "PusherService 未定义"
        
        # 测试工厂函数
        ai_service = get_ai_service()
        assert ai_service is not None, "get_ai_service() 返回 None"
        
        broadcast_service = get_broadcast_service()
        assert broadcast_service is not None, "get_broadcast_service() 返回 None"
        
        pusher_service = get_pusher_service()
        assert pusher_service is not None, "get_pusher_service() 返回 None"
        
        # 测试 FinanceCalcService 方法
        calc = FinanceCalcService()
        from decimal import Decimal
        result = calc.calculate_pnl(
            buy_price=Decimal('100'),
            sell_price=Decimal('120'),
            quantity=Decimal('10')
        )
        assert 'pnl' in result, "calculate_pnl 返回值缺少 pnl 字段"
        print(f"   ✓ 盈亏计算测试通过：pnl={result['pnl']}")
        
        print("   ✓ 所有 Services 已定义并可实例化")
        print("✅ Services 层测试通过！\n")
        return True
        
    except Exception as e:
        print(f"❌ Services 层测试失败：{e}\n")
        import traceback
        traceback.print_exc()
        return False


async def main():
    """运行所有 API 层测试"""
    print("=" * 60)
    print("API 层 & Services 层 单元测试")
    print("=" * 60)
    
    results = []
    
    results.append(("Auth API", await test_auth_api()))
    results.append(("Chat API", await test_chat_api()))
    results.append(("Group API", await test_group_api()))
    results.append(("Monitor API", await test_monitor_api()))
    results.append(("Broadcast API", await test_broadcast_api()))
    results.append(("Finance API", await test_finance_module()))
    results.append(("Services 层", await test_services()))
    
    print("\n" + "=" * 60)
    print("测试结果汇总")
    print("=" * 60)
    
    passed = sum(1 for _, result in results if result)
    total = len(results)
    
    for name, result in results:
        status = "✅ 通过" if result else "❌ 失败"
        print(f"{status} - {name}")
    
    print(f"\n总计：{passed}/{total} API/Services 测试通过")
    
    if passed == total:
        print("\n🎉 所有 API/Services 测试通过！")
        return 0
    else:
        print(f"\n⚠️  有 {total - passed} 个测试失败，请检查问题")
        return 1


if __name__ == "__main__":
    exit_code = asyncio.run(main())
    sys.exit(exit_code)
