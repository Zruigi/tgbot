# -*- coding: utf-8 -*-
"""
轻量级 Bot 多语言服务。

- 使用本地 JSON 语言包，避免引入额外重量级依赖
- 用户语言偏好存储在 admin_settings，作用域覆盖主 Bot / 商户 Bot / Mini App 入口
"""
from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Dict

import database

DEFAULT_LOCALE = "zh-CN"
SUPPORTED_LOCALES = ("zh-CN", "en", "ja", "ru")
LANGUAGE_NAMES = {
    "zh-CN": "简体中文",
    "en": "English",
    "ja": "日本語",
    "ru": "Русский",
}
LOCALE_ALIASES = {
    "zh": "zh-CN",
    "zh-cn": "zh-CN",
    "zh-hans": "zh-CN",
    "en-us": "en",
    "en-gb": "en",
    "ja-jp": "ja",
    "ru-ru": "ru",
}

_LOCALES_DIR = Path(__file__).resolve().parent.parent / "locales" / "bot"
_TRANSLATIONS: Dict[str, Dict[str, str]] = {}


def _load_translations() -> Dict[str, Dict[str, str]]:
    loaded: Dict[str, Dict[str, str]] = {}
    for locale in SUPPORTED_LOCALES:
        locale_file = _LOCALES_DIR / f"{locale}.json"
        if not locale_file.exists():
            loaded[locale] = {}
            continue
        with locale_file.open("r", encoding="utf-8") as fh:
            loaded[locale] = json.load(fh)
    return loaded


_TRANSLATIONS = _load_translations()


def normalize_locale(locale: str | None) -> str:
    raw = str(locale or "").strip()
    if not raw:
        return DEFAULT_LOCALE
    lowered = raw.lower()
    if lowered in LOCALE_ALIASES:
        return LOCALE_ALIASES[lowered]
    for candidate in SUPPORTED_LOCALES:
        if lowered == candidate.lower():
            return candidate
    primary = lowered.split("-", 1)[0]
    return LOCALE_ALIASES.get(primary, DEFAULT_LOCALE)


async def resolve_user_locale(user_id: int | None, telegram_language_code: str | None = None) -> str:
    if user_id:
        stored = await database.get_admin_setting(f"user_lang:{int(user_id)}", None)
        if stored:
            return normalize_locale(str(stored))
    return normalize_locale(telegram_language_code)


async def set_user_locale(user_id: int, locale: str) -> str:
    normalized = normalize_locale(locale)
    await database.save_admin_setting(f"user_lang:{int(user_id)}", normalized)
    return normalized


def language_label(locale: str) -> str:
    normalized = normalize_locale(locale)
    return LANGUAGE_NAMES.get(normalized, normalized)


def tr(locale: str, key: str, **kwargs: Any) -> str:
    normalized = normalize_locale(locale)
    template = (
        _TRANSLATIONS.get(normalized, {}).get(key)
        or _TRANSLATIONS.get(DEFAULT_LOCALE, {}).get(key)
        or key
    )
    if kwargs:
        try:
            return template.format(**kwargs)
        except Exception:
            return template
    return template
