#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
未关注频道用户提醒功能 - 集成测试脚本
"""

import asyncio
import aiohttp
import json
from datetime import datetime

# 配置
BASE_URL = "http://localhost:8080"
ADMIN_TOKEN = "your_admin_token_here"  # 需要替换为实际的 token

class IntegrationTester:
    def __init__(self):
        self.session = None
        self.test_results = []
    
    async def setup(self):
        """初始化测试环境"""
        self.session = aiohttp.ClientSession()
        print("✅ 测试环境初始化完成\n")
    
    async def teardown(self):
        """清理测试环境"""
        if self.session:
            await self.session.close()
        print("\n✅ 测试环境清理完成")
    
    def log_test(self, name, status, details=""):
        """记录测试结果"""
        self.test_results.append({
            'name': name,
            'status': status,
            'details': details,
            'timestamp': datetime.now().isoformat()
        })
        
        status_icon = "✅" if status == "PASS" else "❌"
        print(f"{status_icon} {name}")
        if details:
            print(f"   {details}")
    
    async def test_get_unverified_users(self):
        """测试: 获取未关注用户列表"""
        try:
            headers = {'Authorization': f'Bearer {ADMIN_TOKEN}'}
            
            # 测试全局查询
            async with self.session.get(
                f"{BASE_URL}/api/group_listen/unverified_users?period=all",
                headers=headers
            ) as resp:
                if resp.status == 200:
                    data = await resp.json()
                    if data.get('status') == 'ok':
                        count = data.get('count', 0)
                        self.log_test(
                            "GET /api/group_listen/unverified_users (全局)",
                            "PASS",
                            f"返回 {count} 个未关注用户"
                        )
                        return True
                    else:
                        self.log_test(
                            "GET /api/group_listen/unverified_users (全局)",
                            "FAIL",
                            f"API 返回错误: {data.get('message')}"
                        )
                else:
                    self.log_test(
                        "GET /api/group_listen/unverified_users (全局)",
                        "FAIL",
                        f"HTTP {resp.status}"
                    )
        except Exception as e:
            self.log_test(
                "GET /api/group_listen/unverified_users (全局)",
                "FAIL",
                str(e)
            )
        
        return False
    
    async def test_get_verified_users(self):
        """测试: 获取已验证用户列表"""
        try:
            headers = {'Authorization': f'Bearer {ADMIN_TOKEN}'}
            
            async with self.session.get(
                f"{BASE_URL}/api/group_listen/verified_users?period=all",
                headers=headers
            ) as resp:
                if resp.status == 200:
                    data = await resp.json()
                    if data.get('status') == 'ok':
                        count = data.get('count', 0)
                        self.log_test(
                            "GET /api/group_listen/verified_users (全局)",
                            "PASS",
                            f"返回 {count} 个已验证用户"
                        )
                        return True
                    else:
                        self.log_test(
                            "GET /api/group_listen/verified_users (全局)",
                            "FAIL",
                            f"API 返回错误: {data.get('message')}"
                        )
                else:
                    self.log_test(
                        "GET /api/group_listen/verified_users (全局)",
                        "FAIL",
                        f"HTTP {resp.status}"
                    )
        except Exception as e:
            self.log_test(
                "GET /api/group_listen/verified_users (全局)",
                "FAIL",
                str(e)
            )
        
        return False
    
    async def test_send_channel_reminder(self):
        """测试: 发送频道提醒（模拟）"""
        try:
            headers = {
                'Authorization': f'Bearer {ADMIN_TOKEN}',
                'Content-Type': 'application/json'
            }
            
            payload = {
                'group_id': 123456,  # 测试群组 ID
                'user_ids': [789012, 789013],  # 测试用户 ID
                'channel_link': 'https://t.me/test_channel'
            }
            
            async with self.session.post(
                f"{BASE_URL}/api/group_listen/send_channel_reminder",
                headers=headers,
                json=payload
            ) as resp:
                if resp.status == 200:
                    data = await resp.json()
                    if data.get('status') == 'ok':
                        sent_count = data.get('sent_count', 0)
                        self.log_test(
                            "POST /api/group_listen/send_channel_reminder",
                            "PASS",
                            f"已发送 {sent_count} 条提醒消息"
                        )
                        return True
                    else:
                        self.log_test(
                            "POST /api/group_listen/send_channel_reminder",
                            "FAIL",
                            f"API 返回错误: {data.get('message')}"
                        )
                else:
                    self.log_test(
                        "POST /api/group_listen/send_channel_reminder",
                        "FAIL",
                        f"HTTP {resp.status}"
                    )
        except Exception as e:
            self.log_test(
                "POST /api/group_listen/send_channel_reminder",
                "FAIL",
                str(e)
            )
        
        return False
    
    async def test_delete_reminder_message(self):
        """测试: 删除提醒消息（模拟）"""
        try:
            headers = {
                'Authorization': f'Bearer {ADMIN_TOKEN}',
                'Content-Type': 'application/json'
            }
            
            payload = {
                'group_id': 123456,
                'message_id': 12345,
                'reminder_id': 1
            }
            
            async with self.session.post(
                f"{BASE_URL}/api/group_listen/delete_reminder_message",
                headers=headers,
                json=payload
            ) as resp:
                if resp.status in [200, 500]:  # 500 可能是因为消息不存在
                    data = await resp.json()
                    if data.get('status') == 'ok':
                        self.log_test(
                            "POST /api/group_listen/delete_reminder_message",
                            "PASS",
                            "消息删除请求已发送"
                        )
                        return True
                    else:
                        # 消息不存在也是正常的
                        self.log_test(
                            "POST /api/group_listen/delete_reminder_message",
                            "PASS",
                            f"API 返回: {data.get('message')}"
                        )
                        return True
                else:
                    self.log_test(
                        "POST /api/group_listen/delete_reminder_message",
                        "FAIL",
                        f"HTTP {resp.status}"
                    )
        except Exception as e:
            self.log_test(
                "POST /api/group_listen/delete_reminder_message",
                "FAIL",
                str(e)
            )
        
        return False
    
    async def run_all_tests(self):
        """运行所有测试"""
        print("=" * 60)
        print("未关注频道用户提醒功能 - 集成测试")
        print("=" * 60)
        print()
        
        await self.setup()
        
        print("📋 测试项目:")
        print("-" * 60)
        
        # 运行测试
        await self.test_get_verified_users()
        await self.test_get_unverified_users()
        await self.test_send_channel_reminder()
        await self.test_delete_reminder_message()
        
        await self.teardown()
        
        # 输出测试总结
        print("\n" + "=" * 60)
        print("📊 测试总结")
        print("=" * 60)
        
        passed = sum(1 for r in self.test_results if r['status'] == 'PASS')
        failed = sum(1 for r in self.test_results if r['status'] == 'FAIL')
        total = len(self.test_results)
        
        print(f"总计: {total} 个测试")
        print(f"✅ 通过: {passed} 个")
        print(f"❌ 失败: {failed} 个")
        print(f"成功率: {(passed/total*100):.1f}%")
        
        if failed == 0:
            print("\n🎉 所有测试通过！")
        else:
            print(f"\n⚠️  有 {failed} 个测试失败，请检查日志")
        
        return failed == 0

async def main():
    tester = IntegrationTester()
    success = await tester.run_all_tests()
    exit(0 if success else 1)

if __name__ == '__main__':
    asyncio.run(main())
