#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
财务 ERP 集成验证脚本 (无需外部依赖)
"""
import os
import sys

def verify_integration():
    """验证财务 ERP 集成"""
    print("\n" + "="*70)
    print("🏆 财务 ERP 集成验证")
    print("="*70 + "\n")
    
    checks = []
    
    # 检查 1: 后端模块文件
    print("✓ 检查后端模块文件...")
    backend_files = [
        'finance_core.py',
        'finance_reports.py',
        'finance_api.py'
    ]
    for f in backend_files:
        exists = os.path.exists(f)
        status = "✅" if exists else "❌"
        print(f"  {status} {f}")
        checks.append(exists)
    
    # 检查 2: 前端组件
    print("\n✓ 检查前端组件...")
    frontend_file = 'frontend/src/views/FinancePanel.vue'
    exists = os.path.exists(frontend_file)
    status = "✅" if exists else "❌"
    print(f"  {status} {frontend_file}")
    checks.append(exists)
    
    # 检查 3: 文档
    print("\n✓ 检查文档...")
    docs = [
        'FINANCE_IMPLEMENTATION.md',
        'FINANCE_DELIVERY_SUMMARY.md',
        'FINANCE_QUICK_START.md'
    ]
    for doc in docs:
        exists = os.path.exists(doc)
        status = "✅" if exists else "❌"
        print(f"  {status} {doc}")
        checks.append(exists)
    
    # 检查 4: 集成代码
    print("\n✓ 检查集成代码...")
    
    # 检查 database.py
    with open('database.py', 'r') as f:
        db_content = f.read()
        has_finance_init = 'init_finance_module' in db_content
        status = "✅" if has_finance_init else "❌"
        print(f"  {status} database.py 已添加 init_finance_module()")
        checks.append(has_finance_init)
    
    # 检查 web_admin.py
    with open('web_admin.py', 'r') as f:
        web_content = f.read()
        has_finance_routes = 'register_finance_routes' in web_content
        status = "✅" if has_finance_routes else "❌"
        print(f"  {status} web_admin.py 已添加 register_finance_routes()")
        checks.append(has_finance_routes)
    
    # 总结
    print("\n" + "="*70)
    total = len(checks)
    passed = sum(checks)
    print(f"集成验证: {passed}/{total} 项通过")
    print("="*70 + "\n")
    
    if all(checks):
        print("✅ 所有集成检查通过！\n")
        print("📝 后续步骤:")
        print("   1. 安装依赖: pip install aiosqlite weasyprint openpyxl")
        print("   2. 在 main.py 中调用: await register_finance_routes(app)")
        print("   3. 在 main.py 中调用: await init_finance_module()")
        print("   4. 启动应用后访问: http://localhost:8080/api/finance/chart_of_accounts")
        print("   5. 前端访问: http://localhost:5173/#/finance\n")
        return True
    else:
        print("❌ 部分集成检查失败，请检查上述标记为 ❌ 的项目\n")
        return False

if __name__ == "__main__":
    result = verify_integration()
    sys.exit(0 if result else 1)
