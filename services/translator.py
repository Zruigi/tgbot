# -*- coding: utf-8 -*-
"""
LLM 翻译服务模块 - 2026 规格
支持全自动语种识别、Context 翻译、LLM/Google 自动路由与地址保护
"""
import logging
import asyncio
import config
import database
import aiohttp
import re
from services.secret_manager import SecretManager
from deep_translator import GoogleTranslator
from services.security_shield import shield_text, unshield_text, verify_address_consistency

logger = logging.getLogger(__name__)

def is_chinese(text: str) -> bool:
    """简单正则判断是否包含中文字符"""
    return bool(re.search(r'[\u4e00-\u9fa5]', text))


# 常见英文聊天填充词/语气词（中文用户混用时多为寒暄，不触发机翻）
_CASUAL_ENGLISH_WORDS = frozenset({
    "a", "an", "the", "i", "u", "im", "you", "we", "it", "is", "are", "am", "be", "to", "do", "so", "my", "me",
    "and", "or", "but", "also", "too", "very", "really", "just", "only", "not", "all", "any", "some", "that", "this",
    "ok", "okay", "kk", "k", "yes", "no", "yeah", "yep", "nope", "nah", "yup", "ya", "yea",
    "good", "nice", "cool", "fine", "great", "wow", "omg", "lol", "haha", "hehe", "rofl", "lmao",
    "hi", "hey", "hello", "bye", "pls", "please", "thx", "thanks", "thank", "ty", "sry", "sorry",
    "gm", "gn", "morning", "night", "afternoon", "evening",
    "sure", "maybe", "well", "hmm", "um", "uh", "emm", "mmm",
    "perfect", "awesome", "super", "cute", "love",
    "wtf", "idk", "ikr", "tbh", "imo", "fyi", "np", "gl", "hf",
    "wait", "hold", "stop", "go", "see", "look", "got", "get", "come", "back",
    "bro", "dude", "man", "miss", "sir", "mam", "babe",
})

# 出现任一即视为可能含专业/业务语义，需要翻译
_PROFESSIONAL_HINTS = (
    "withdraw", "deposit", "wallet", "blockchain", "crypto", "bitcoin", "ethereum",
    "usdt", "btc", "eth", "bnb", "trx", "fee", "gas", "staking", "contract",
    "liquidat", "margin", "leverage", "kyc", "aml", "verify", "verification",
    "pending", "confirm", "network", "chain", "token", "defi", "nft", "mining",
    "validator", "address", "hash", "transaction", "transfer", "balance", "order",
    "api", "key", "seed", "phrase", "private", "public", "sign", "message",
    "support", "ticket", "account", "bind", "unbind", "freeze", "risk", "audit",
    "interest", "apy", "apr", "yield", "farm", "pool", "swap", "bridge",
    "mainnet", "testnet", "rpc", "node", "sync", "error", "failed", "success",
)


def _normalize_repeated_letters(word: str) -> str:
    """okkk→ok、yesss→yes：连续 3 个及以上相同字母压成 1 个，便于匹配闲聊词。"""
    return re.sub(r"(.)\1{2,}", r"\1", word.lower())


def _looks_like_stretched_casual(word: str) -> bool:
    """识别 okkk、goood、niceee 等拉长拼写的常见寒暄词。"""
    w = word.lower()
    if re.fullmatch(r"o+k+", w):
        return True
    if re.fullmatch(r"g+o+d+", w):
        return True
    if re.fullmatch(r"n+i+c+e+", w):
        return True
    if re.fullmatch(r"y+e+s+", w):
        return True
    return False


def _word_is_casual_filler(word: str) -> bool:
    w = word.lower().strip("'").strip('"')
    if not w:
        return True
    if _looks_like_stretched_casual(w):
        return True
    if w in _CASUAL_ENGLISH_WORDS:
        return True
    w2 = _normalize_repeated_letters(w)
    if w2 in _CASUAL_ENGLISH_WORDS:
        return True
    # 极短无意义碎片
    if len(w) <= 2 and w.isalpha():
        return True
    return False


def should_translate_user_to_admin(text: str) -> bool:
    """
    用户 -> 管理员：是否值得调用翻译。
    - 纯中文：不机翻（由上层直接回原文）。
    - 纯表情/无字母：不翻。
    - 短句且仅为常见英文寒暄/语气词：不翻（避免 okkk、good nice 等触发译文）。
    - 较长句、含业务/加密相关关键词、或非闲聊词：需要翻译（可带术语注解）。
    """
    if not text or not text.strip():
        return False
    if is_chinese(text):
        return False
    if not re.search(r"[a-zA-Z]", text):
        return False
    tl = text.lower()
    for hint in _PROFESSIONAL_HINTS:
        if hint in tl:
            return True
    # 句子较长，按正文处理
    if len(text.strip()) > 52:
        return True
    words = re.findall(r"[a-zA-Z']+", text)
    if len(words) >= 5:
        return True
    if not words:
        return False
    for w in words:
        if not _word_is_casual_filler(w):
            return True
    return False


def _parse_llm_translation_output(raw: str) -> str:
    """
    解析 LLM 按约定输出的【译文】【术语注解】；若未按格式则整体作为译文展示。
    """
    if not raw:
        return ""
    t = raw.strip()
    if "【译文】" in t:
        start = t.index("【译文】") + len("【译文】")
        end = t.find("【术语注解】", start)
        body = t[start:end].strip() if end != -1 else t[start:].strip()
        note = ""
        if "【术语注解】" in t:
            ni = t.index("【术语注解】") + len("【术语注解】")
            note = t[ni:].strip()
        if note and note not in ("无", "无。", "none", "None", "N/A"):
            return f"{body}\n\n【术语注解】\n{note}"
        return body
    return t


def _build_system_prompt_for_user_to_admin(custom_suffix: str) -> str:
    """用户消息译成中文：固定规则 + 管理员在库里保存的补充说明。"""
    base = (
        "你是加密货币客服场景的专业翻译。将用户消息译为流畅的简体中文。\n"
        "输出格式（必须严格遵守，方便系统解析）：\n"
        "【译文】\n"
        "（这里只写中文译文正文，保留占位符 [[ADDR_N]] 不变）\n"
        "【术语注解】\n"
        "若存在专业术语、项目名、缩写（如 KYC、DeFi、爆仓、链上确认），用一行或多行简短解释；"
        "若没有需要解释的术语，只写一个字：无\n"
        "不要输出任何其它前言、Markdown 代码块或角色扮演。\n"
    )
    if custom_suffix and custom_suffix.strip():
        base += "\n管理员补充规则（在遵守上述格式前提下参考）：\n" + custom_suffix.strip()
    return base


def _build_system_prompt_for_admin_to_user(target_lang: str, custom_suffix: str) -> str:
    base = (
        f"你是专业翻译。将管理员回复译为目标语言：{target_lang}。\n"
        "只输出译文，不要解释。保留占位符 [[ADDR_N]] 不变。\n"
        "若原文已是目标语言，保持原文。\n"
    )
    if custom_suffix and custom_suffix.strip():
        base += "\n管理员补充规则：\n" + custom_suffix.strip()
    return base

async def _call_openai_compatible(url: str, api_key: str, model: str, messages: list, temperature: float = 0.3) -> str:
    """高性能 OpenAI 兼容接口调用"""
    headers = {
        "Authorization": f"Bearer {api_key}",
        "Content-Type": "application/json"
    }
    api_url = url.rstrip('/') 
    if not api_url.endswith('/chat/completions'):
        api_url += '/chat/completions'

    payload = {
        "model": model,
        "messages": messages,
        "temperature": temperature,
        "max_tokens": 2048
    }
    
    async with aiohttp.ClientSession() as session:
        async with session.post(api_url, headers=headers, json=payload, timeout=15.0) as response:
            if response.status != 200:
                error_text = await response.text()
                raise Exception(f"API Error {response.status}: {error_text}")
                
            data = await response.json()
            try:
                return data['choices'][0]['message']['content'].strip()
            except (KeyError, IndexError):
                raise Exception("Failed to parse 2026 spec response")

async def verify_token_async(token: str, model: str, base_url: str) -> bool:
    """异步链路验证 (2026 Spec)"""
    try:
        messages = [{"role": "user", "content": "Ping"}]
        headers = {"Authorization": f"Bearer {token}", "Content-Type": "application/json"}
        api_url = base_url.rstrip('/') 
        if not api_url.endswith('/chat/completions'):
            api_url += '/chat/completions'
            
        payload = {"model": model, "messages": messages, "max_tokens": 1}
        
        async with aiohttp.ClientSession() as session:
            async with session.post(api_url, headers=headers, json=payload, timeout=8.0) as response:
                return response.status == 200
    except Exception as e:
        logger.error(f"2026 Spec Token verification failed: {e}")
        return False

async def translate_text_advanced(text: str, system_prompt: str, user_prompt: str) -> str:
    """核心翻译中枢 (2026 Spec)"""
    # 2026 修复：此处 text 参数已在外部 user_prompt 中构造，不再二次拼接
    if not user_prompt: return ""
    
    api_mode = await database.get_admin_setting("llm_api_mode", "rented")
    
    if api_mode == "rented":
        # 租用模式：强制使用 config 中的预置配置
        current_model = config.LLM_MODEL_NAME
        api_url = config.LLM_BASE_URL
        api_key = config.LLM_API_KEY or config.GEMINI_API_KEY or config.DEEPSEEK_API_KEY
        logger.info(f"Using Rented API Mode: {current_model}")
    else:
        # 自有模式：优先从数据库读取用户配置
        current_model = await database.get_admin_setting("llm_model", config.LLM_MODEL_NAME)
        api_url = await database.get_admin_setting("llm_base_url", config.LLM_BASE_URL)
        
        api_key = None
        # 先尝试获取用户自定义的通用 Key
        custom_key = await SecretManager.get_config("LLM_API_KEY", default=None)
        if custom_key:
            api_key = custom_key
        else:
            # 回退到具体的 API Key 类型
            if 'deepseek' in current_model.lower():
                api_key = await SecretManager.get_config("DEEPSEEK_API_KEY", default=config.DEEPSEEK_API_KEY)
            elif 'gemini' in current_model.lower():
                api_key = await SecretManager.get_config("GEMINI_API_KEY", default=config.GEMINI_API_KEY)
            else:
                api_key = await SecretManager.get_config("LLM_API_KEY", default=config.LLM_API_KEY)
    
    if not api_key:
        raise ValueError("Missing valid API Key for 2026 models")

    try:
        messages = [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_prompt}
        ]
        logger.info(f"Calling 2026 AI ({current_model}) with base_url: {api_url}")
        return await _call_openai_compatible(api_url, api_key, current_model, messages)
    except Exception as e:
        logger.error(f"2026 Model ({current_model}) call failed at {api_url}: {e}")
        # 如果是 DeepSeek 失败且没有 fallback，尝试检查是否是 URL 问题
        if 'deepseek' in current_model.lower() and not api_url.endswith('/v1'):
            logger.warning("DeepSeek URL might be missing /v1, but we already added /chat/completions logic.")
        raise

async def translate_text_with_shield(text: str, target_lang: str = "zh-CN", is_to_admin: bool = True):
    """
    2026 规格：带地址保护的 Context 翻译逻辑
    """
    if not text: return ""
    
    # 2026 修复：如果是中文且要翻译成中文，直接返回，严禁中翻中
    if is_to_admin and is_chinese(text):
        return text

    # 1. 引擎与配置加载
    engine = await database.get_admin_setting("translation_engine", "llm")
    custom_prompt = await database.get_admin_setting("system_prompt", "") or ""
    
    # 2. 地址脱敏
    shielded_text, addr_mapping = shield_text(text)
    if not shielded_text.strip(): return ""
    
    # 3. Prompt：用户->管理员（结构化译文+术语注解）；管理员->用户（纯译文）
    if is_to_admin:
        system_prompt = _build_system_prompt_for_user_to_admin(custom_prompt)
        user_prompt = "请翻译以下用户消息：\n" + shielded_text
    else:
        system_prompt = _build_system_prompt_for_admin_to_user(target_lang, custom_prompt)
        user_prompt = shielded_text

    # 4. 执行翻译
    try:
        if engine == "llm":
            translated_shielded = await translate_text_advanced(
                shielded_text,
                system_prompt,
                user_prompt,
            )
            if is_to_admin:
                translated_shielded = _parse_llm_translation_output(translated_shielded)
        else:
            # Google 翻译降级（无结构化术语注解）
            target = 'zh-CN' if is_to_admin else target_lang
            translated_shielded = await asyncio.to_thread(
                lambda: GoogleTranslator(source='auto', target=target).translate(shielded_text)
            )
    except Exception as e:
        logger.warning(f"Translation failed, fallback to raw: {e}")
        translated_shielded = shielded_text

    # 5. 地址回填与一致性校验
    # 2026 修复补丁：过滤掉 AI 可能会复读的提示词标签
    instruction_labels = ["Text to translate:", "Translated text:", "Simplified Chinese:", "### TRANSLATION:"]
    for label in instruction_labels:
        translated_shielded = translated_shielded.replace(label, "").strip()
    
    # --- 防御性过滤 (Defensive Filtering) ---
    # 如果译文中包含翻译指令关键词，说明 LLM 出现指令泄露，直接丢弃译文返回原文
    leakage_keywords = [
        "Dịch sang", "Translate to", "翻译成", "占位符", "保持不变", 
        "Output ONLY", "original text", "Target language"
    ]
    for kw in leakage_keywords:
        if kw.lower() in translated_shielded.lower():
            logger.warning(f"LLM Instruction Leakage Detected: keyword='{kw}'. Discarding translation.")
            return text

    # 清理 AI 可能会带出的引号
    translated_shielded = translated_shielded.strip('"').strip("'")
    
    final_text = unshield_text(translated_shielded, addr_mapping)
    if not verify_address_consistency(text, final_text):
        logger.error("2026 Security: Address consistency breach detected!")
        return text
        
    return final_text

# 兼容接口
async def translate_to_admin(text: str):
    """用户发送消息给管理员时的翻译入口 (自动识别并转中文)"""
    if not should_translate_user_to_admin(text):
        logger.info("Translation skipped: casual filler / no meaningful English to translate")
        return text
    return await translate_text_with_shield(text, target_lang="zh-CN", is_to_admin=True)

async def translate_to_customer(content: str, customer_msg: str):
    from langdetect import detect
    try: target_lang = detect(customer_msg)
    except: target_lang = 'en'
    return await translate_text_with_shield(content, target_lang=target_lang, is_to_admin=False)
