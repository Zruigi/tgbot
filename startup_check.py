#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""
Bot 启动检查脚本 - 确保所有依赖都已准备好
"""
import sys
import os
import subprocess
import time

def check_python_version():
    """检查Python版本"""
    if sys.version_info < (3, 8):
        print("❌ Python 3.8+ required")
        return False
    print(f"✅ Python {sys.version_info.major}.{sys.version_info.minor}")
    return True

def check_required_files():
    """检查必要文件"""
    required = ['main.py', 'config.py', 'database.py']
    for f in required:
        if not os.path.exists(f):
            print(f"❌ Missing: {f}")
            return False
        print(f"✅ Found: {f}")
    return True

def check_dependencies():
    """检查依赖包"""
    try:
        import telegram
        print(f"✅ python-telegram-bot installed")
        return True
    except ImportError:
        print("❌ python-telegram-bot not installed")
        print("   Run: pip install python-telegram-bot")
        return False

def check_config():
    """检查配置"""
    try:
        import config
        if not config.BOT_TOKEN:
            print("❌ BOT_TOKEN not set in config.py")
            return False
        if not config.ADMIN_ID:
            print("❌ ADMIN_ID not set in config.py")
            return False
        print("✅ Config valid")
        return True
    except Exception as e:
        print(f"❌ Config error: {e}")
        return False

def start_bot():
    """启动Bot"""
    print("\n" + "="*50)
    print("Starting Bot...")
    print("="*50 + "\n")
    
    try:
        subprocess.Popen([sys.executable, 'main.py'])
        print("✅ Bot process started")
        print("   Check the bot window for status")
        return True
    except Exception as e:
        print(f"❌ Failed to start bot: {e}")
        return False

def main():
    print("\n" + "="*50)
    print("TG Bot Startup Check")
    print("="*50 + "\n")
    
    checks = [
        ("Python Version", check_python_version),
        ("Required Files", check_required_files),
        ("Dependencies", check_dependencies),
        ("Configuration", check_config),
    ]
    
    all_ok = True
    for name, check_func in checks:
        print(f"\n[*] Checking {name}...")
        if not check_func():
            all_ok = False
            print(f"❌ {name} check failed!")
    
    if not all_ok:
        print("\n" + "="*50)
        print("❌ Startup checks failed!")
        print("="*50)
        input("Press Enter to exit...")
        return False
    
    print("\n" + "="*50)
    print("✅ All checks passed!")
    print("="*50)
    
    if start_bot():
        print("\n✅ Bot started successfully!")
        print("   Management UI: http://127.0.0.1:8080")
        return True
    else:
        print("\n❌ Failed to start bot")
        input("Press Enter to exit...")
        return False

if __name__ == "__main__":
    success = main()
    sys.exit(0 if success else 1)
