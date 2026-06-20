# -*- coding: utf-8 -*-
import pytest

from backend.app.utils.channel_utils import normalize_channel_link


@pytest.mark.parametrize(
    'inp,expected',
    [
        ('@channel', 'https://t.me/channel'),
        ('t.me/news', 'https://t.me/news'),
        ('https://t.me/telegram', 'https://t.me/telegram'),
        (
            'https://t.me/joinchat/abcdefghij',
            'https://t.me/joinchat/abcdefghij',
        ),
        ('/mygroup', 'https://t.me/mygroup'),
    ],
)
def test_normalize_channel_link_ok(inp, expected):
    assert normalize_channel_link(inp) == expected


def test_normalize_channel_link_rejects_garbage():
    with pytest.raises(ValueError):
        normalize_channel_link('https://evil.com/phish')
