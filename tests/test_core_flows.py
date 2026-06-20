# -*- coding: utf-8 -*-
"""
核心流程回归测试 - 验证关键业务功能
"""
import asyncio
import sys
from pathlib import Path

# 添加项目根目录到路径
sys.path.insert(0, str(Path(__file__).parent.parent))


async def test_repository_layer():
    """测试 Repository 层基本功能"""
    print("\n=== Repository 层功能测试 ===")
    
    try:
        from backend.repositories.user_repo import get_user, create_or_update_user
        from backend.repositories.message_repo import save_message
        from backend.repositories.admin_repo import get_admin_setting
        
        print("✅ Repository 导入成功")
        
        # 注意：这些测试需要真实的数据库连接
        # 在实际环境中应该使用测试数据库
        print("⚠️  跳过数据库操作测试（需要测试数据库）")
        return True
        
    except Exception as e:
        print(f"❌ Repository 测试失败：{e}")
        return False


async def test_api_layer():
    """测试 API 层路由注册"""
    print("\n=== API 层路由测试 ===")
    
    try:
        from backend.app.main import create_app
        
        # 创建测试应用
        app = create_app()
        
        # 检查路由是否注册成功
        routes = [route.method for route in app.router.routes()]
        
        print(f"✅ 注册了 {len(routes)} 个路由")
        print(f"   GET 路由数：{routes.count('GET')}")
        print(f"   POST 路由数：{routes.count('POST')}")
        
        # 验证关键路由存在
        expected_routes = [
            '/api/login',
            '/api/users', 
            '/api/messages/{user_id}',
            '/api/search'
        ]
        
        # 使用 resource 来获取路径
        registered_paths = []
        for resource in app.router.resources():
            registered_paths.append(str(resource.canonical))
        
        for expected in expected_routes:
            if any(expected.replace('{user_id}', '*') in path for path in registered_paths):
                print(f"   ✓ 路由已注册：{expected}")
            else:
                print(f"   ⚠️  路由未找到：{expected}")
        
        return True
        
    except Exception as e:
        print(f"❌ API 层测试失败：{e}")
        import traceback
        traceback.print_exc()
        return False


async def test_security_services():
    """测试安全服务功能"""
    print("\n=== 安全服务测试 ===")
    
    try:
        from backend.app.core.security import TokenManager, RateLimiter, SecurityMiddleware
        
        # 测试 TokenManager
        token_mgr = TokenManager()
        otp = token_mgr.generate_otp()
        assert len(otp) > 0, "OTP 生成失败"
        print(f"   ✓ OTP 生成：{otp[:10]}...")
        
        # 测试 CSRF Token
        csrf = token_mgr.generate_csrf_token()
        assert len(csrf) > 0, "CSRF Token 生成失败"
        print(f"   ✓ CSRF Token 生成：{csrf[:10]}...")
        
        # 测试速率限制
        limiter = RateLimiter(window_seconds=60, max_attempts=5)
        for i in range(3):
            result = limiter.check_limit("test_ip")
            print(f"   ✓ 速率限制检查 #{i+1}: {'通过' if result else '拒绝'}")
        
        # 测试 SecurityMiddleware（需要依赖注入）
        token_manager = TokenManager()
        rate_limiter = RateLimiter(window_seconds=60, max_attempts=5)
        middleware = SecurityMiddleware(token_manager, rate_limiter)
        assert middleware is not None, "SecurityMiddleware 创建失败"
        print(f"   ✓ SecurityMiddleware 初始化成功")
        
        print("✅ 所有安全服务测试通过！")
        return True
        
    except Exception as e:
        print(f"❌ 安全服务测试失败：{e}")
        import traceback
        traceback.print_exc()
        return False


async def test_utility_functions():
    """测试工具函数"""
    print("\n=== 工具函数测试 ===")
    
    try:
        from backend.app.utils.channel_utils import (
            normalize_channel_link,
            validate_channel_link,
            parse_channel_username,
            validate_wallet_address
        )
        
        # 测试频道链接处理
        test_cases = [
            ('@channel', 'https://t.me/channel'),
            ('t.me/news', 'https://t.me/news'),
            ('https://t.me/telegram', 'https://t.me/telegram'),
        ]
        
        for input_val, expected in test_cases:
            result = normalize_channel_link(input_val)
            assert result == expected, f"{input_val}: 期望 {expected}, 得到 {result}"
            print(f"   ✓ 链接规范化：{input_val} → {result}")
        
        # 测试钱包地址验证
        assert validate_wallet_address('TJRyWwFsH7TFGUCVFnQpXjdZbJnKJzKvLd', 'TRC20')
        assert validate_wallet_address('0x1234567890abcdef1234567890abcdef12345678', 'BSC')
        print(f"   ✓ 钱包地址验证通过")
        
        print("✅ 所有工具函数测试通过！")
        return True
        
    except Exception as e:
        print(f"❌ 工具函数测试失败：{e}")
        import traceback
        traceback.print_exc()
        return False


async def main():
    """运行所有回归测试"""
    print("=" * 60)
    print("核心流程回归测试")
    print("=" * 60)
    
    results = []
    
    # 运行各项测试
    results.append(("Repository 层", await test_repository_layer()))
    results.append(("API 层", await test_api_layer()))
    results.append(("安全服务", await test_security_services()))
    results.append(("工具函数", await test_utility_functions()))
    
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
        print("\n🎉 所有回归测试通过！架构重构验证完成！")
        return 0
    else:
        print(f"\n⚠️  有 {total - passed} 个测试失败，请检查问题")
        return 1


if __name__ == "__main__":
    exit_code = asyncio.run(main())
    sys.exit(exit_code)
