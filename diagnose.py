#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""
Bot 诊断脚本 - 排查重启问题
"""
import os
import sys
import subprocess
import time

def diagnose():
    print("\n" + "="*60)
    print("TG Bot Diagnostic Tool")
    print("="*60 + "\n")
    
    # 1. 检查Python
    print("[1] Python Environment")
    print(f"    Version: {sys.version}")
    print(f"    Executable: {sys.executable}")
    
    # 2. 检查文件
    print("\n[2] Required Files")
    files = ['main.py', 'config.py', 'database.py', 'handlers/customer_service.py']
    for f in files:
        exists = "✓" if os.path.exists(f) else "✗"
        print(f"    {exists} {f}")
    
    # 3. 检查依赖
    print("\n[3] Dependencies")
    packages = ['telegram', 'aiosqlite', 'aiohttp']
    for pkg in packages:
        try:
            __import__(pkg)
            print(f"    ✓ {pkg}")
        except ImportError:
            print(f"    ✗ {pkg} (missing)")
    
    # 4. 检查配置
    print("\n[4] Configuration")
    try:
        import config
        print(f"    ✓ config.py loaded")
        print(f"    - BOT_TOKEN: {'set' if config.BOT_TOKEN else 'NOT SET'}")
        print(f"    - ADMIN_ID: {config.ADMIN_ID if config.ADMIN_ID else 'NOT SET'}")
    except Exception as e:
        print(f"    ✗ Error loading config: {e}")
    
    # 5. 检查进程
    print("\n[5] Running Processes")
    try:
        result = subprocess.run(['tasklist'], capture_output=True, text=True)
        if 'python.exe' in result.stdout:
            print("    ✓ Python process found")
            # 尝试找到main.py进程
            for line in result.stdout.split('\n'):
                if 'python' in line.lower():
                    print(f"      {line.strip()}")
        else:
            print("    ✗ No Python process found")
    except:
        print("    ? Unable to check processes")
    
    # 6. 检查端口
    print("\n[6] Network Ports")
    try:
        result = subprocess.run(['netstat', '-aon'], capture_output=True, text=True)
        ports = ['8080', '5000', '3000']
        for port in ports:
            if f':{port}' in result.stdout:
                print(f"    ✓ Port {port} in use")
            else:
                print(f"    - Port {port} available")
    except:
        print("    ? Unable to check ports")
    
    # 7. 建议
    print("\n[7] Recommendations")
    print("    If bot won't restart:")
    print("    1. Check if python.exe is still running (Task Manager)")
    print("    2. Manually kill python.exe processes")
    print("    3. Check config.py for BOT_TOKEN and ADMIN_ID")
    print("    4. Run: pip install -r requirements.txt")
    print("    5. Try running: python main.py (directly)")
    
    print("\n" + "="*60 + "\n")

if __name__ == "__main__":
    diagnose()
    input("Press Enter to exit...")
