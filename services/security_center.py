# -*- coding: utf-8 -*-
"""
统一安全核验中心 (Security Check Center)
集成 OCR 核验、地址匹配补丁、全平台风控与翻译逻辑
"""
import re
import logging
import os
import io
import difflib
import config
import database

try:
    import pytesseract
    from PIL import Image
    
    # --- OCR 路径钩子 (自动探测) ---
    if config.TESSERACT_CMD:
        pytesseract.pytesseract.tesseract_cmd = config.TESSERACT_CMD
    elif os.name == 'nt':
        common_paths = [
            r'C:\Program Files\Tesseract-OCR\tesseract.exe',
            r'C:\Program Files (x86)\Tesseract-OCR\tesseract.exe',
            os.path.join(os.environ.get('LOCALAPPDATA', ''), r'Tesseract-OCR\tesseract.exe'),
        ]
        for path in common_paths:
            if os.path.exists(path):
                pytesseract.pytesseract.tesseract_cmd = path
                break
except ImportError:
    pytesseract = None

from services.security_shield import ADDR_REGEX, shield_text, unshield_text, verify_address_consistency

logger = logging.getLogger(__name__)


def _tesseract_extra_config() -> str:
    """
    仅当项目 tessdata 目录下确有 .traineddata 时才传入 --tessdata-dir。
    若目录为空或仅有占位，强制指定会覆盖系统自带语言包，导致 OCR 直接失败。
    """
    tessdata_dir = os.path.join(os.path.dirname(os.path.dirname(__file__)), "tessdata")
    if not os.path.isdir(tessdata_dir):
        return ""
    try:
        has_lang = any(name.endswith(".traineddata") for name in os.listdir(tessdata_dir))
    except OSError:
        return ""
    if not has_lang:
        return ""
    # Windows 路径含空格时用引号
    td = os.path.normpath(tessdata_dir)
    return f'--tessdata-dir "{td}"' if " " in td else f"--tessdata-dir {td}"


def _best_address_window_ratio(clean_addr: str, clean_ocr: str) -> float:
    """
    在 OCR 全文中用滑动窗口与地址比相似度（解决整串比对导致 ratio 极低的问题）。
    """
    n, m = len(clean_addr), len(clean_ocr)
    if n == 0 or m == 0:
        return 0.0
    if n > m:
        return difflib.SequenceMatcher(None, clean_addr, clean_ocr).ratio()
    best = 0.0
    # 允许窗口长度随 OCR 漏字/多字略浮动
    for delta in range(-5, 6):
        wlen = n + delta
        if wlen < 1 or wlen > m:
            continue
        for i in range(m - wlen + 1):
            window = clean_ocr[i : i + wlen]
            r = difflib.SequenceMatcher(None, clean_addr, window).ratio()
            if r > best:
                best = r
    return best


def _ocr_matches_crypto_address(clean_addr: str, clean_ocr: str) -> bool:
    """子串命中或滑动窗口相似度达标即视为通过。"""
    if clean_addr in clean_ocr:
        return True
    return _best_address_window_ratio(clean_addr, clean_ocr) > 0.85


def extract_addresses(text):
    """提取文本中的所有加密地址"""
    if not text: return []
    return re.findall(ADDR_REGEX, text)

def is_pure_address(text):
    """
    建立【地址熔断机制】：检测文本是否仅包含加密货币地址、地址占位符或空白分隔符
    """
    if not text: return False
    # 移除所有空白字符
    clean_text = text.strip()
    if not clean_text: return False
    
    # 构建纯地址/占位符匹配正则
    # 允许：地址、[[ADDR_N]]、以及空格/换行/逗号等分隔符
    pattern = rf'^({ADDR_REGEX}|\[\[ADDR_\d+\]\]|[ \n\t,;，；])+$'
    return bool(re.fullmatch(pattern, clean_text))

async def security_check_flow(text, image_bytes=None, do_translate=False, target_lang='zh-CN'):
    """
    统一安全校验流程 (Web & Bot 共用)
    返回: (is_passed, final_text, error_message, is_ocr_verified)
    """
    from services import translator
    
    addresses = extract_addresses(text)
    is_ocr_verified = False
    
    # 获取管理员设置的 OCR 强核验开关
    ocr_force_enabled = (await database.get_admin_setting("ocr_force_enabled", "true")).lower() == "true"
    
    # 1. 地址扫描与初步拦截
    if addresses:
        if image_bytes is None:
            return False, text, "⚠️ <b>安全拦截：检测到加密地址。</b>\n请务必配上高清截图并确保文字一致。", False, ""
            
        # 2. OCR 核验
        ocr_tag = ""
        if not pytesseract:
            logger.warning("OCR 引擎未安装，降级为人工核验提示")
            ocr_tag = "\n(⚠️ 系统未安装 OCR 引擎，请人工核验图片)"
        else:
            try:
                image = Image.open(io.BytesIO(image_bytes))
                if image.mode not in ("RGB", "L"):
                    image = image.convert("RGB")
                config_str = _tesseract_extra_config()
                # 优先中英混合；未装 chi_sim 时回退 eng（地址为字母数字）
                try:
                    ocr_text = pytesseract.image_to_string(
                        image, lang="chi_sim+eng", config=config_str
                    )
                except Exception as lang_err:
                    logger.warning(
                        "OCR chi_sim+eng 失败，回退 eng: %s", lang_err
                    )
                    ocr_text = pytesseract.image_to_string(
                        image, lang="eng", config=config_str
                    )
                
                # --- OCR 模糊算法升级 (Fuzzy Match Upgrade) ---
                # 1. 预处理 OCR 全文：去空格、转大写
                # 修复：pytesseract 在 Windows 下可能返回字节流或非 UTF-8 编码，强制转换并清理
                if isinstance(ocr_text, bytes):
                    ocr_text = ocr_text.decode('utf-8', errors='ignore')
                
                clean_ocr = "".join(ocr_text.split()).upper()
                
                all_passed = True
                for addr in addresses:
                    # 2. 预处理用户输入地址
                    clean_addr = addr.strip().replace(" ", "").upper()
                    
                    # 3. 比对：子串 或 滑动窗口相似度（勿用「整段地址 vs 整段 OCR」的 ratio）
                    window_ratio = _best_address_window_ratio(clean_addr, clean_ocr)
                    if _ocr_matches_crypto_address(clean_addr, clean_ocr):
                        logger.info(
                            "OCR Match Passed: addr=%s, window_best=%.2f",
                            (clean_addr[:20] + "…") if len(clean_addr) > 20 else clean_addr,
                            window_ratio,
                        )
                        continue
                    else:
                        logger.warning(
                            "OCR Match Failed: addr=%s, window_best=%.2f",
                            (clean_addr[:20] + "…") if len(clean_addr) > 20 else clean_addr,
                            window_ratio,
                        )
                        all_passed = False
                        break
                
                if all_passed:
                    is_ocr_verified = True
                    # 只有在识别到地址且通过时，才显示 ✅ 标签；如果是纯地址图片且通过，提示语更简洁
                    ocr_tag = "\n(✅ 地址截图已通过 OCR 核验)"
                else:
                    # 如果开启了强核验，则拦截
                    if ocr_force_enabled:
                        return False, text, "❌ <b>验证不匹配！</b>\n请确保图片清晰。如果是特殊混淆地址，请管理员在 /admin 中临时关闭 OCR 强核验。", False, ""
                    else:
                        is_ocr_verified = False
                        # 优化文案：更专业的风险提示
                        ocr_tag = "\n(⚠️ 提示：地址 OCR 自动核验未完全匹配，请务必人工核对截图)"
            except Exception as e:
                # 修复：捕获编码错误并记录详细日志
                logger.error(f"OCR 验证出错 (可能由于 Windows 编码冲突): {e}")
                # 如果 OCR 失败但是用户发送的地址文本在之前的 logic 中已经通过了，这里给一个更温和的提示
                ocr_tag = "\n(⚠️ 提示：图片 OCR 解析异常，请人工核对截图内容)"
    else:
        ocr_tag = ""

    # 3. 地址熔断与翻译逻辑 (Address Circuit Breaker)
    final_text = text
    if do_translate:
        # 建立【地址熔断机制】：如果是纯地址消息，绝对禁止调用翻译
        if is_pure_address(text):
            logger.info("Address Circuit Breaker: Pure address message detected, skipping LLM.")
            final_text = text
        else:
            try:
                # 2026 规格：提取纯净内容翻译，过滤掉管理员前缀与系统 Tag
                from services.security_shield import strip_administrative_remarks
                clean_content = strip_administrative_remarks(text)
                
                if clean_content:
                    is_to_admin = (target_lang == 'zh-CN')
                    final_text = await translator.translate_text_with_shield(clean_content, target_lang, is_to_admin=is_to_admin)
                else:
                    # 如果清洗后没有内容 (例如只有地址和备注)，则翻译结果为空，前端不显示译文框
                    final_text = ""
            except Exception as e:
                logger.error(f"安全流程中的翻译失败: {e}")
            
    # 最终结果：翻译后的文本 (仅包含正文)
    # 2026 规格：不再将 OCR 标签强行合并到正文中，避免泄露给客户
    if not final_text and not ocr_tag:
        # 如果既没有翻译正文也没有 OCR 标签，则尝试对原始文本进行地址脱敏后返回
        shielded_text, _ = shield_text(text)
        final_text = shielded_text
        
    return True, final_text, None, is_ocr_verified, ocr_tag
