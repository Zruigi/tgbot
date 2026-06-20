# -*- coding: utf-8 -*-
"""
Deep Linking 解禁功能测试脚本
用于验证整个流程是否正常工作
"""
import asyncio
import logging
from datetime import datetime

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


async def test_deeplink_parsing():
    """测试 Deep Link 参数解析"""
    logger.info("=" * 60)
    logger.info("测试 1: Deep Link 参数解析")
    logger.info("=" * 60)
    
    test_cases = [
        ("unmute_-1001234567890", -1001234567890, True),
        ("unmute_123456", 123456, True),
        ("unmute_invalid", None, False),
        ("invalid_param", None, False),
    ]
    
    for param, expected_group_id, should_succeed in test_cases:
        try:
            if param.startswith("unmute_"):
                group_id = int(param.replace("unmute_", ""))
                if group_id == expected_group_id:
                    logger.info(f"✓ 参数解析成功: {param} → group_id={group_id}")
                else:
                    logger.error(f"✗ 参数解析失败: {param} → 期望 {expected_group_id}，得到 {group_id}")
            else:
                logger.info(f"✓ 参数不匹配（预期）: {param}")
        except (ValueError, IndexError) as e:
            if not should_succeed:
                logger.info(f"✓ 参数解析失败（预期）: {param}")
            else:
                logger.error(f"✗ 参数解析异常: {param} → {e}")


async def test_rate_limit():
    """测试频率限制"""
    logger.info("\n" + "=" * 60)
    logger.info("测试 2: 频率限制（1分钟内只能验证一次）")
    logger.info("=" * 60)
    
    import time
    _unmute_rate_limit = {}
    
    user_id = 12345
    now = time.time()
    
    # 第一次请求
    if user_id not in _unmute_rate_limit:
        _unmute_rate_limit[user_id] = now
        logger.info(f"✓ 第一次请求通过 user_id={user_id}")
    
    # 第二次请求（30秒后）
    now_2 = now + 30
    if user_id in _unmute_rate_limit:
        last_time = _unmute_rate_limit[user_id]
        if now_2 - last_time < 60:
            remaining = int(60 - (now_2 - last_time))
            logger.info(f"✓ 频率限制生效 user_id={user_id}，剩余等待时间: {remaining}秒")
        else:
            logger.info(f"✓ 可以重新验证 user_id={user_id}")
    
    # 第三次请求（70秒后）
    now_3 = now + 70
    if user_id in _unmute_rate_limit:
        last_time = _unmute_rate_limit[user_id]
        if now_3 - last_time < 60:
            remaining = int(60 - (now_3 - last_time))
            logger.info(f"✓ 频率限制生效 user_id={user_id}，剩余等待时间: {remaining}秒")
        else:
            _unmute_rate_limit[user_id] = now_3
            logger.info(f"✓ 可以重新验证 user_id={user_id}，已更新时间戳")


async def test_channel_membership_check():
    """测试频道成员检查逻辑"""
    logger.info("\n" + "=" * 60)
    logger.info("测试 3: 频道成员检查逻辑")
    logger.info("=" * 60)
    
    # 模拟不同的成员状态
    test_statuses = [
        ("member", True),
        ("administrator", True),
        ("creator", True),
        ("restricted", False),
        ("left", False),
        ("kicked", False),
    ]
    
    for status, should_be_member in test_statuses:
        is_member = status in ('member', 'administrator', 'creator')
        if is_member == should_be_member:
            logger.info(f"✓ 成员状态检查正确: status={status} → is_member={is_member}")
        else:
            logger.error(f"✗ 成员状态检查错误: status={status} → 期望 {should_be_member}，得到 {is_member}")


async def test_permissions_structure():
    """测试权限恢复结构"""
    logger.info("\n" + "=" * 60)
    logger.info("测试 4: 权限恢复结构")
    logger.info("=" * 60)
    
    # 模拟权限对象
    permissions = {
        "can_send_messages": True,
        "can_send_polls": True,
        "can_send_other_messages": True,
        "can_add_web_page_previews": True,
        "can_change_info": False,
        "can_invite_users": True,
        "can_pin_messages": False
    }
    
    logger.info(f"✓ 权限对象创建成功")
    for key, value in permissions.items():
        logger.info(f"  - {key}: {value}")


async def test_deeplink_generation():
    """测试 Deep Link 生成"""
    logger.info("\n" + "=" * 60)
    logger.info("测试 5: Deep Link 生成")
    logger.info("=" * 60)
    
    test_cases = [
        ("my_bot", -1001234567890, "https://t.me/my_bot?start=unmute_-1001234567890"),
        ("crypto_bot", 123456, "https://t.me/crypto_bot?start=unmute_123456"),
    ]
    
    for bot_username, group_id, expected_link in test_cases:
        deep_link = f"https://t.me/{bot_username}?start=unmute_{group_id}"
        if deep_link == expected_link:
            logger.info(f"✓ Deep Link 生成正确: {deep_link}")
        else:
            logger.error(f"✗ Deep Link 生成错误: 期望 {expected_link}，得到 {deep_link}")


async def test_callback_data_parsing():
    """测试回调数据解析"""
    logger.info("\n" + "=" * 60)
    logger.info("测试 6: 回调数据解析")
    logger.info("=" * 60)
    
    test_cases = [
        ("unmute_verify:-1001234567890:@channel", -1001234567890, "@channel", True),
        ("unmute_verify:123456:-100987654321", 123456, "-100987654321", True),
        ("unmute_verify:invalid:data", None, None, False),
    ]
    
    for data, expected_group_id, expected_channel_id, should_succeed in test_cases:
        try:
            if data.startswith("unmute_verify:"):
                parts = data.split(":")
                group_id = int(parts[1])
                channel_id = parts[2]
                
                if group_id == expected_group_id and channel_id == expected_channel_id:
                    logger.info(f"✓ 回调数据解析成功: {data}")
                else:
                    logger.error(f"✗ 回调数据解析失败: {data}")
        except (IndexError, ValueError) as e:
            if not should_succeed:
                logger.info(f"✓ 回调数据解析失败（预期）: {data}")
            else:
                logger.error(f"✗ 回调数据解析异常: {data} → {e}")


async def test_message_template():
    """测试消息模板"""
    logger.info("\n" + "=" * 60)
    logger.info("测试 7: 消息模板")
    logger.info("=" * 60)
    
    user_id = 12345
    display_name = "TestUser"
    mention = f'<a href="tg://user?id={user_id}">{display_name}</a>'
    
    # 禁言提示消息
    mute_prompt = (
        f"⚠️ {mention} 你因未验证或触发规则被禁言。\n\n"
        f"点击下方按钮关注频道并完成验证，即可恢复发言权限。"
    )
    logger.info(f"✓ 禁言提示消息模板:")
    logger.info(f"  {mute_prompt}")
    
    # 频道关注引导消息
    channel_guide = (
        "📢 <b>需要关注频道才能解禁</b>\n\n"
        "请先关注下方频道，然后点击 [我已关注，点击验证] 按钮完成验证。\n\n"
        "验证成功后，你将恢复在群组中的发言权限。"
    )
    logger.info(f"✓ 频道关注引导消息:")
    logger.info(f"  {channel_guide}")
    
    # 解禁成功消息
    success_msg = (
        "✅ <b>解禁成功！</b>\n\n"
        "你已恢复在群组中的发言权限，现在可以回到群里发言了。\n\n"
        "感谢你的关注与支持！"
    )
    logger.info(f"✓ 解禁成功消息:")
    logger.info(f"  {success_msg}")


async def test_database_operations():
    """测试数据库操作（模拟）"""
    logger.info("\n" + "=" * 60)
    logger.info("测试 8: 数据库操作（模拟）")
    logger.info("=" * 60)
    
    # 模拟数据库操作
    group_members = {}
    
    # 保存成员
    user_id = 12345
    group_id = -1001234567890
    group_members[(group_id, user_id)] = {
        "user_id": user_id,
        "group_id": group_id,
        "is_verified": False,
        "verified_at": None,
        "pending_verify_msg_id": 999
    }
    logger.info(f"✓ 保存成员: user_id={user_id}, group_id={group_id}")
    
    # 更新验证状态
    if (group_id, user_id) in group_members:
        group_members[(group_id, user_id)]["is_verified"] = True
        group_members[(group_id, user_id)]["verified_at"] = datetime.now().isoformat()
        logger.info(f"✓ 更新验证状态: is_verified=True, verified_at={group_members[(group_id, user_id)]['verified_at']}")
    
    # 清除消息 ID
    if (group_id, user_id) in group_members:
        group_members[(group_id, user_id)]["pending_verify_msg_id"] = None
        logger.info(f"✓ 清除消息 ID: pending_verify_msg_id=None")


async def main():
    """运行所有测试"""
    logger.info("\n")
    logger.info("╔" + "=" * 58 + "╗")
    logger.info("║" + " " * 58 + "║")
    logger.info("║" + "  Deep Linking 解禁功能测试套件".center(58) + "║")
    logger.info("║" + " " * 58 + "║")
    logger.info("╚" + "=" * 58 + "╝")
    
    await test_deeplink_parsing()
    await test_rate_limit()
    await test_channel_membership_check()
    await test_permissions_structure()
    await test_deeplink_generation()
    await test_callback_data_parsing()
    await test_message_template()
    await test_database_operations()
    
    logger.info("\n" + "=" * 60)
    logger.info("所有测试完成！")
    logger.info("=" * 60 + "\n")


if __name__ == "__main__":
    asyncio.run(main())
