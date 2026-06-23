# -*- coding: utf-8 -*-
"""handlers/customer/admin_reply_handlers 纯函数单元测试"""
from types import SimpleNamespace

from handlers.customer.admin_reply_handlers import (
    build_admin_feedback,
    classify_message_content,
    extract_target_user_info,
    extract_quoted_message_info,
    prepare_translation_feedback,
)


def test_prepare_translation_feedback_no_change():
    assert prepare_translation_feedback("hello", "hello") == (None, None)


def test_prepare_translation_feedback_translated():
    t, o = prepare_translation_feedback("hello", "你好")
    assert t == "你好" and o == "hello"


def test_build_admin_feedback_untranslated():
    fb = build_admin_feedback("U1", "photo", "vtext", "[图片]", None, None)
    assert "发送成功 [photo]" in fb
    assert "内容:" in fb
    assert "U1" in fb


def test_build_admin_feedback_translated():
    fb = build_admin_feedback("U1", "text", "", "", "译文", "原文")
    assert "原文:" in fb and "译文:" in fb
    assert "U1" in fb


def test_classify_message_content_text():
    m = SimpleNamespace(
        text="hi",
        photo=None,
        sticker=None,
        animation=None,
        video=None,
        video_note=None,
        voice=None,
        audio=None,
        document=None,
    )
    assert classify_message_content(m, "verified") == ("text", "verified", None)


def test_classify_message_content_sticker():
    st = SimpleNamespace(emoji="😀", file_id="fid")
    m = SimpleNamespace(
        text=None,
        photo=None,
        sticker=st,
        animation=None,
        video=None,
        video_note=None,
        voice=None,
        audio=None,
        document=None,
    )
    mt, content, fid = classify_message_content(m, "")
    assert mt == "sticker"
    assert "[表情包:" in content
    assert fid == "fid"


def test_extract_quoted_message_info_plain():
    rm = SimpleNamespace(
        text="hello world",
        caption=None,
        photo=None,
        sticker=None,
        document=None,
        video=None,
    )
    info = extract_quoted_message_info(rm)
    assert info["text"] == "hello world"
    assert info["display_text"] == "hello world"
    assert info["type"] == "text"


def test_extract_quoted_message_info_yuanwen_marker():
    rm = SimpleNamespace(
        text="原文：inner\n----------------",
        caption=None,
        photo=None,
        sticker=None,
        document=None,
        video=None,
    )
    info = extract_quoted_message_info(rm)
    assert info["text"] == "inner"


async def test_extract_target_user_info_falls_back_to_userid_text(monkeypatch):
    async def no_mapping(_message_id):
        return None

    monkeypatch.setattr("database.get_user_id_by_admin_msg", no_mapping)
    monkeypatch.setattr("database.get_original_msg_id_by_admin_msg", no_mapping)

    reply = SimpleNamespace(
        message_id=42,
        text='新咨询\nUserID: <code>123456</code>\n\n内容: hello',
        caption=None,
    )

    target_user_id, original_tg_msg_id = await extract_target_user_info(reply)

    assert target_user_id == 123456
    assert original_tg_msg_id is None
