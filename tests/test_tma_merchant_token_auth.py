# -*- coding: utf-8 -*-
import hashlib
import hmac
import json
import sys
from pathlib import Path
from urllib.parse import urlencode

import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))

import web_admin


def _build_init_data(bot_token: str, user_payload: dict, start_param: str = "") -> str:
    pairs = {
        "auth_date": "1717000000",
        "query_id": "AAHdF6IQAAAAAN0XohDhrOrc",
        "user": json.dumps(user_payload, separators=(",", ":")),
    }
    if start_param:
        pairs["start_param"] = start_param

    check_string = "\n".join(f"{k}={v}" for k, v in sorted(pairs.items()))
    secret_key = hmac.new(
        b"WebAppData",
        bot_token.encode(),
        hashlib.sha256,
    ).digest()
    pairs["hash"] = hmac.new(
        secret_key,
        check_string.encode(),
        hashlib.sha256,
    ).hexdigest()
    return urlencode(pairs)


@pytest.mark.asyncio
async def test_authenticate_tma_uses_merchant_token_with_hint(monkeypatch):
    main_token = "main_bot_token"
    merchant_token = "merchant_bot_token"
    user_payload = {
        "id": 123456,
        "first_name": "Owner",
        "username": "merchant_owner",
        "language_code": "en",
    }
    init_data = _build_init_data(merchant_token, user_payload, start_param="merchant_console")

    async def fake_secret_get(key, tenant_id="default", default=None):
        if key == "TELEGRAM_BOT_TOKEN" and tenant_id == "tenant_a":
            return merchant_token
        return default

    async def fake_get_merchant_by_owner(owner_tg_user_id):
        if int(owner_tg_user_id) == 123456:
            return {
                "id": 1,
                "tenant_id": "tenant_a",
                "owner_tg_user_id": 123456,
                "merchant_name": "Merchant A",
            }
        return None

    async def fake_get_merchant_by_id(merchant_id):
        if int(merchant_id) == 1:
            return {
                "id": 1,
                "tenant_id": "tenant_a",
                "owner_tg_user_id": 123456,
                "merchant_name": "Merchant A",
            }
        return None

    monkeypatch.setattr(web_admin.config, "BOT_TOKEN", main_token, raising=False)
    monkeypatch.setattr(web_admin.SecretManager, "get_config", fake_secret_get)
    monkeypatch.setattr(web_admin.merchant_repo, "get_merchant_by_owner", fake_get_merchant_by_owner)
    monkeypatch.setattr(web_admin.merchant_repo, "get_merchant_by_id", fake_get_merchant_by_id)

    failed = await web_admin._authenticate_tma_init_data(init_data)
    assert failed["ok"] is False

    succeeded = await web_admin._authenticate_tma_init_data(
        init_data,
        tenant_hint="tenant_a",
        merchant_id_hint=1,
    )
    assert succeeded["ok"] is True
    assert succeeded["merchant"]["tenant_id"] == "tenant_a"
    assert succeeded["verified_tenant_id"] == "tenant_a"
