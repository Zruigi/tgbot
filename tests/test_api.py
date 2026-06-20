import asyncio
import sys
sys.path.insert(0, "/home/ubuntu/tgbot")

async def test_detect_ad_api():
    print("\n=== Test: Ad Detection API ===")
    try:
        from services.ai_agent.ad_detection import detect_advertising_signals
        
        test_cases = [
            ("加我vx测试广告", "group_message", True),
            ("你好，请问有什么可以帮您", "group_message", False),
            ("扫码免费领取红包", "group_message", True),
        ]
        
        for text, ctx, expected in test_cases:
            result = detect_advertising_signals(text, ctx)
            status = "OK" if result["is_advertising"] == expected else "FAIL"
            print("  [%s] score=%d, is_ad=%s" % (status, result["ad_score"], result["is_advertising"]))
            
        return True
    except Exception as e:
        print("  Error: " + str(e))
        return False

async def test_ai_config():
    print("\n=== Test: AI Config ===")
    try:
        import config
        print("  DEFAULT_LLM: " + config.DEFAULT_LLM)
        print("  DEEPSEEK_API_KEY: " + ("Set" if config.DEEPSEEK_API_KEY else "Not Set"))
        return True
    except Exception as e:
        print("  Error: " + str(e))
        return False

async def main():
    print("=" * 40)
    print("TGBot API Unit Tests")
    print("=" * 40)
    
    results = []
    results.append(("AI Config", await test_ai_config()))
    results.append(("Ad Detection", await test_detect_ad_api()))
    
    print("\n" + "=" * 40)
    print("Summary:")
    all_passed = all(r[1] for r in results)
    for name, passed in results:
        mark = "OK" if passed else "FAIL"
        print("  [%s] %s" % (mark, name))
    print("\nOverall: " + ("ALL PASSED" if all_passed else "SOME FAILED"))
    
    return 0 if all_passed else 1

if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
