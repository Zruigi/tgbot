#!/usr/bin/env python3
"""
Gemini 模型完整测试脚本
测试所有通过 LiteLLM 配置的 Gemini 模型
"""

import urllib.request
import json
import os
import time
import sys

# 配置
LITELLM_URL = "http://localhost:4000/v1/chat/completions"
API_KEY = os.environ.get("LITELLM_MASTER_KEY", "")

# 要测试的模型列表
MODELS_TO_TEST = [
    ("gpt-3.5-turbo", "gemini-2.5-flash-lite", "经济版 - 快速响应"),
    ("gpt-4", "gemini-2.5-pro", "高性能版 - 最强性能"),
    ("gemini-3.1-pro-preview", "gemini-3.1-pro-preview", "最新预览版 - Pro"),
    ("gemini-3.1-flash-lite-preview", "gemini-3.1-flash-lite-preview", "轻量预览版 - Flash"),
]

def test_model(model_name, actual_model, description):
    """测试单个模型"""
    print(f"\n{'='*80}")
    print(f"🧪 测试: {model_name}")
    print(f"   实际模型: {actual_model}")
    print(f"   说明: {description}")
    print(f"{'='*80}")
    
    # 构建请求
    data = {
        "model": model_name,
        "messages": [
            {"role": "user", "content": "请用中文回复：你好"}
        ],
        "max_tokens": 50,
        "temperature": 0.7
    }
    
    headers = {
        "Authorization": f"Bearer {API_KEY}",
        "Content-Type": "application/json"
    }
    
    try:
        start_time = time.time()
        
        # 发送请求
        req = urllib.request.Request(
            LITELLM_URL,
            data=json.dumps(data).encode('utf-8'),
            headers=headers,
            method='POST'
        )
        
        with urllib.request.urlopen(req, timeout=30) as response:
            elapsed = time.time() - start_time
            result = json.loads(response.read().decode('utf-8'))
            
            # 检查响应
            if 'choices' in result and len(result['choices']) > 0:
                message = result['choices'][0].get('message', {})
                content = message.get('content', '')
                
                # 如果 content 为 None，设置为空字符串
                if content is None:
                    content = ''
                
                usage = result.get('usage', {})
                
                print(f"\n✅ 调用成功！")
                print(f"   回复内容: {content}")
                print(f"   响应时间: {elapsed:.2f}秒")
                print(f"   Token 使用:")
                print(f"     - Prompt tokens: {usage.get('prompt_tokens', 'N/A')}")
                print(f"     - Completion tokens: {usage.get('completion_tokens', 'N/A')}")
                print(f"     - Total tokens: {usage.get('total_tokens', 'N/A')}")
                
                return {
                    "model": model_name,
                    "actual_model": actual_model,
                    "status": "success",
                    "response_time": f"{elapsed:.2f}s",
                    "total_tokens": usage.get('total_tokens', 0),
                    "content_preview": content[:50]
                }
            else:
                print(f"\n❌ 调用失败：无返回内容")
                print(f"   响应: {json.dumps(result, indent=2, ensure_ascii=False)[:200]}")
                return {
                    "model": model_name,
                    "actual_model": actual_model,
                    "status": "failed",
                    "error": "No choices in response"
                }
                
    except urllib.error.HTTPError as e:
        elapsed = time.time() - start_time
        error_body = e.read().decode('utf-8')
        try:
            error_data = json.loads(error_body)
            error_msg = error_data.get('error', {}).get('message', str(e))
        except:
            error_msg = str(e)
        
        print(f"\n❌ HTTP 错误 {e.code}")
        print(f"   响应时间: {elapsed:.2f}秒")
        print(f"   错误信息: {error_msg[:200]}")
        
        return {
            "model": model_name,
            "actual_model": actual_model,
            "status": "error",
            "http_code": e.code,
            "error": error_msg[:200]
        }
        
    except Exception as e:
        elapsed = time.time() - start_time
        print(f"\n❌ 异常错误")
        print(f"   响应时间: {elapsed:.2f}秒")
        print(f"   错误信息: {str(e)[:200]}")
        
        return {
            "model": model_name,
            "actual_model": actual_model,
            "status": "exception",
            "error": str(e)[:200]
        }


def main():
    """主函数"""
    print("\n" + "="*80)
    print("🚀 Gemini 模型完整测试")
    print("="*80)
    print(f"\n📍 LiteLLM 地址: {LITELLM_URL}")
    print(f"🔑 API Key: {API_KEY}")
    print(f"📊 待测模型: {len(MODELS_TO_TEST)} 个")
    print(f"\n开始时间: {time.strftime('%Y-%m-%d %H:%M:%S')}")
    
    # 测试所有模型
    results = []
    for model_name, actual_model, description in MODELS_TO_TEST:
        result = test_model(model_name, actual_model, description)
        results.append(result)
        
        # 避免请求过快
        time.sleep(1)
    
    # 打印汇总报告
    print("\n\n" + "="*80)
    print("📊 测试结果汇总")
    print("="*80)
    
    success_count = sum(1 for r in results if r['status'] == 'success')
    failed_count = len(results) - success_count
    
    print(f"\n总计: {len(results)} 个模型")
    print(f"✅ 成功: {success_count}")
    print(f"❌ 失败: {failed_count}")
    print(f"成功率: {success_count/len(results)*100:.1f}%")
    
    print("\n详细结果:")
    print("-" * 80)
    print(f"{'模型':<35} {'状态':<8} {'耗时':<10} {'Token':<8} {'说明'}")
    print("-" * 80)
    
    for r in results:
        status_icon = "✅" if r['status'] == 'success' else "❌"
        status_text = "成功" if r['status'] == 'success' else "失败"
        
        response_time = r.get('response_time', '-')
        tokens = str(r.get('total_tokens', '-'))
        
        # 找到对应的说明
        desc = ""
        for model_name, actual_model, description in MODELS_TO_TEST:
            if model_name == r['model']:
                desc = description
                break
        
        print(f"{r['model']:<35} {status_text:<8} {response_time:<10} {tokens:<8} {desc}")
    
    print("-" * 80)
    
    # 成功模型详情
    if success_count > 0:
        print("\n✅ 成功模型详情:")
        for r in results:
            if r['status'] == 'success':
                print(f"\n   {r['model']}:")
                print(f"     回复预览: {r.get('content_preview', '')}")
    
    # 失败模型详情
    if failed_count > 0:
        print("\n❌ 失败模型详情:")
        for r in results:
            if r['status'] != 'success':
                print(f"\n   {r['model']}:")
                print(f"     错误: {r.get('error', 'Unknown')[:100]}")
    
    # 总体评估
    print("\n\n" + "="*80)
    print("💡 总体评估")
    print("="*80)
    
    if success_count == len(results):
        print("\n🎉 所有模型测试通过！可以正常使用。")
    elif success_count >= 3:
        print(f"\n✅ 大部分模型可用 ({success_count}/{len(results)})")
        print("   推荐使用的模型:")
        for r in results:
            if r['status'] == 'success':
                print(f"   - {r['model']}")
    elif success_count >= 1:
        print(f"\n⚠️  部分模型可用 ({success_count}/{len(results)})")
        print("   建议:")
        print("   1. 检查 LiteLLM 配置文件")
        print("   2. 验证 Google Cloud 权限")
        print("   3. 查看 LiteLLM 日志排查问题")
    else:
        print("\n❌ 所有模型调用失败")
        print("   请检查:")
        print("   1. LiteLLM 容器是否运行: sudo docker ps | grep litellm")
        print("   2. 配置文件是否正确: /home/ubuntu/tgbot/litellm_config.yaml")
        print("   3. API Key 是否有效")
        print("   4. Google Cloud 项目权限")
    
    print(f"\n结束时间: {time.strftime('%Y-%m-%d %H:%M:%S')}")
    print("="*80 + "\n")
    
    # 返回退出码
    return 0 if success_count > 0 else 1


if __name__ == "__main__":
    sys.exit(main())
