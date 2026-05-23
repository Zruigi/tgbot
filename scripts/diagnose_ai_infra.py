#!/usr/bin/env python3
"""
AI 基础设施诊断工具
自动检查 LiteLLM、凭证、配置和网络连通性
"""
import asyncio
import subprocess
import sys
import json
from pathlib import Path

PROJECT_DIR = Path("/home/ubuntu/tgbot")
LOG_DIR = PROJECT_DIR / "logs"

class AIDiagnoser:
    def __init__(self):
        self.issues = []
        self.warnings = []
    
    def run_check(self, name, check_func):
        """执行检查并记录结果"""
        print(f"\n🔍 检查: {name}")
        try:
            result = check_func()
            if asyncio.iscoroutine(result):
                result = asyncio.run(result)
            
            if result is True:
                print(f"  ✅ 通过")
                return True
            elif result is False:
                print(f"  ❌ 失败")
                self.issues.append(name)
                return False
            else:
                print(f"  ⚠️  警告: {result}")
                self.warnings.append(name)
                return None
        except Exception as e:
            print(f"  ❌ 异常: {e}")
            self.issues.append(name)
            return False
    
    def check_litellm_running(self):
        """检查 LiteLLM 容器是否运行"""
        result = subprocess.run(
            ["sudo", "docker", "ps", "--filter", "name=litellm", "--format", "{{.Names}}"],
            capture_output=True, text=True
        )
        return "litellm" in result.stdout
    
    async def check_credentials_mounted(self):
        """检查凭证文件是否正确挂载"""
        try:
            result = subprocess.run(
                ["sudo", "docker", "exec", "litellm", "test", "-f", "/app/google-key.json"],
                capture_output=True, text=True
            )
            return result.returncode == 0
        except:
            return "容器未运行或文件不存在"
    
    async def check_llm_config_in_db(self):
        """检查数据库中的 LLM 配置"""
        import aiosqlite
        
        db_path = Path("crypto_bot.sqlite3")
        if not db_path.exists():
            return "数据库文件不存在"
        
        async with aiosqlite.connect(str(db_path)) as db:
            cursor = await db.execute(
                "SELECT key, value FROM admin_settings WHERE key LIKE '%llm%'"
            )
            rows = await cursor.fetchall()
            
            required_keys = ['llm_base_url', 'llm_api_key', 'llm_provider']
            found_keys = {row[0]: row[1] for row in rows}
            
            missing = [k for k in required_keys if k not in found_keys]
            if missing:
                return f"缺少配置: {', '.join(missing)}"
            
            print(f"  配置项:")
            for k, v in found_keys.items():
                masked_v = v[:10] + "***" if len(v) > 10 else v
                print(f"    {k}: {masked_v}")
            
            return True
    
    def check_cloudflare_tunnel(self):
        """检查 Cloudflare 隧道状态"""
        tunnel_log = LOG_DIR / "tunnel" / "cloudflared" / "cloudflared.stdout"
        if not tunnel_log.exists():
            return "隧道日志不存在"
        
        with open(tunnel_log, 'r') as f:
            content = f.read()
        
        import re
        match = re.search(r'https://([a-z0-9-]+\.trycloudflare\.com)', content)
        if match:
            print(f"  当前隧道: {match.group(1)}")
            return True
        else:
            return "未找到活跃的隧道域名"
    
    async def test_litellm_endpoint(self):
        """测试 LiteLLM API 端点"""
        import aiohttp
        
        try:
            async with aiohttp.ClientSession() as session:
                # 测试健康检查
                async with session.get("http://localhost:4000/health") as resp:
                    if resp.status != 200:
                        return f"健康检查失败: {resp.status}"
                
                # 测试模型列表
                async with session.get(
                    "http://localhost:4000/v1/models",
                    headers={"Authorization": "Bearer sk-vertex-1300"}
                ) as resp:
                    if resp.status != 200:
                        return f"模型列表失败: {resp.status}"
                    
                    data = await resp.json()
                    models = [m['id'] for m in data.get('data', [])]
                    print(f"  可用模型: {', '.join(models[:5])}...")
                    
                    if 'gemini-3.1-pro-preview' not in models:
                        return "gemini-3.1-pro-preview 未在模型列表中"
                    
                    return True
        except Exception as e:
            return f"连接失败: {e}"
    
    def generate_report(self):
        """生成诊断报告"""
        print("\n" + "="*60)
        print("📊 诊断报告")
        print("="*60)
        
        if not self.issues:
            print("✅ 所有检查通过！")
        else:
            print(f"❌ 发现 {len(self.issues)} 个问题:")
            for issue in self.issues:
                print(f"  - {issue}")
        
        if self.warnings:
            print(f"\n⚠️  {len(self.warnings)} 个警告:")
            for warning in self.warnings:
                print(f"  - {warning}")
        
        print("\n💡 建议操作:")
        if "LiteLLM 容器运行" in self.issues:
            print("  1. 启动 LiteLLM: sudo docker start litellm")
        if "凭证文件挂载" in self.issues:
            print("  2. 检查 google-key.json 挂载路径")
        if "数据库 LLM 配置" in self.issues:
            print("  3. 运行 configure_llm.py 重新配置")
        if "Cloudflare 隧道" in self.issues:
            print("  4. 重启 Cloudflare 隧道并同步域名")

def main():
    diagnoser = AIDiagnoser()
    
    print("🏥 AI 基础设施诊断工具")
    print("="*60)
    
    diagnoser.run_check("LiteLLM 容器运行", diagnoser.check_litellm_running)
    diagnoser.run_check("凭证文件挂载", diagnoser.check_credentials_mounted)
    diagnoser.run_check("数据库 LLM 配置", diagnoser.check_llm_config_in_db)
    diagnoser.run_check("Cloudflare 隧道", diagnoser.check_cloudflare_tunnel)
    diagnoser.run_check("LiteLLM API 端点", diagnoser.test_litellm_endpoint)
    
    diagnoser.generate_report()

if __name__ == "__main__":
    main()
