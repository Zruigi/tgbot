# -*- coding: utf-8 -*-
"""Stable text normalization shared by message storage and ad clustering."""

import hashlib
import re


_URL_RE = re.compile(r"https?://\S+|t\.me/\S+|@\w{3,}", re.I)
_SPACE_RE = re.compile(r"\s+")


def normalize_ad_text(text: str) -> str:
    value = str(text or "").strip().lower()
    value = _URL_RE.sub("[link]", value)
    return _SPACE_RE.sub(" ", value).strip()


def ad_text_hash(text: str) -> str:
    return hashlib.md5(normalize_ad_text(text).encode("utf-8")).hexdigest()
