#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
财务模块集成测试脚本
"""
import asyncio
import aiosqlite
import json
from datetime import datetime
from config import DB_NAME, WEB_ADMIN_TOKEN

async def test_finance_integration():
    """测试财务模块的完整流程"""
    
    print("=" * 80)
    print("🧪 财务模块集成测试")
    print("=" * 80)
    
    # 1. 测试数据库表是否存在
    print("\n[1] 检查数据库表结构...")
    async with aiosqlite.connect(DB_NAME, timeout=10) as db:
        cursor = await db.execute("""
            SELECT name FROM sqlite_master 
            WHERE type='table' AND name IN ('chart_of_accounts', 'finance_ledgers', 'audit_logs')
        """)
        tables = await cursor.fetchall()
        if len(tables) == 3:
            print("✅ 所有财务表已创建")
            for table in tables:
                print(f"   - {table[0]}")
        else:
            print(f"❌ 缺少财务表，仅找到 {len(tables)} 个")
            return False
    
    # 2. 测试会计科目是否初始化
    print("\n[2] 检查会计科目初始化...")
    async with aiosqlite.connect(DB_NAME, timeout=10) as db:
        cursor = await db.execute("SELECT COUNT(*) FROM chart_of_accounts")
        count = await cursor.fetchone()
        if count[0] > 0:
            print(f"✅ 会计科目已初始化，共 {count[0]} 个")
            cursor = await db.execute("SELECT code, name, type FROM chart_of_accounts LIMIT 5")
            rows = await cursor.fetchall()
            for code, name, acc_type in rows:
                print(f"   - {code}: {name} ({acc_type})")
        else:
            print("❌ 会计科目未初始化")
            return False
    
    # 3. 测试创建总账记录
    print("\n[3] 测试创建总账记录...")
    try:
        from finance_core import create_finance_ledger
        import time
        
        ledger_id = await create_finance_ledger(
            monitor_id=1,
            user_id=123456,
            account_id=1,  # 现金账户
            direction="in",
            amount=2.405,
            token_symbol="ETH",
            base_fiat_value=5000.0,
            fiat_currency="USD",
            exchange_rate=2080.0,
            counterparty="OTC Desk",
            description="测试入金",
            tx_hash="0x123abc",
            tx_timestamp=int(time.time())
        )
        print(f"✅ 总账记录创建成功: {ledger_id}")
    except Exception as e:
        print(f"❌ 创建总账记录失败: {e}")
        # 继续测试其他功能
    
    # 4. 测试查询未核销流水
    print("\n[4] 测试查询未核销流水...")
    try:
        from finance_core import get_unreconciled_ledgers
        
        ledgers = await get_unreconciled_ledgers()
        if ledgers:
            print(f"✅ 查询到 {len(ledgers)} 条未核销流水")
            for ledger in ledgers[:3]:
                print(f"   - {ledger.get('ledger_id')}: {ledger.get('amount')} {ledger.get('token_symbol')}")
        else:
            print("⚠️  未查询到未核销流水（可能是首次运行）")
    except Exception as e:
        print(f"❌ 查询未核销流水失败: {e}")
    
    # 5. 测试查询统计数据
    print("\n[5] 测试查询统计数据...")
    try:
        from finance_core import get_reconciliation_summary
        
        summary = await get_reconciliation_summary()
        print(f"✅ 统计数据查询成功:")
        print(f"   - 未核销笔数: {summary.get('total_unreconciled', 0)}")
        print(f"   - 总入金: ${summary.get('total_inflow', 0):.2f}")
        print(f"   - 总出金: ${summary.get('total_outflow', 0):.2f}")
    except Exception as e:
        print(f"❌ 查询统计数据失败: {e}")
    
    # 6. 测试审计日志
    print("\n[6] 测试审计日志...")
    try:
        from finance_core import get_audit_logs
        
        logs = await get_audit_logs(days=1)
        if logs:
            print(f"✅ 审计日志查询成功，共 {len(logs)} 条")
            for log in logs[:3]:
                print(f"   - {log.get('timestamp')}: {log.get('action')} on {log.get('target_table')}")
        else:
            print("⚠️  未查询到审计日志（可能是首次运行）")
    except Exception as e:
        print(f"❌ 查询审计日志失败: {e}")
    
    print("\n" + "=" * 80)
    print("✅ 财务模块基础测试完成！")
    print("=" * 80)
    return True

if __name__ == "__main__":
    success = asyncio.run(test_finance_integration())
    exit(0 if success else 1)
