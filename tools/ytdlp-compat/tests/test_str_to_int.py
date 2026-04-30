"""yt-dlp's `str_to_int` — parse 'human-formatted' integer strings."""

from rdlp_ytdlp_compat import str_to_int


def test_none_returns_none():
    assert str_to_int(None) is None


def test_plain_int_string():
    assert str_to_int('123') == 123


def test_comma_thousands_separator():
    assert str_to_int('1,234') == 1234
    assert str_to_int('1,234,567') == 1234567


def test_period_decimal_separator_truncates():
    assert str_to_int('1.5K') == 1500


def test_k_suffix():
    assert str_to_int('5K') == 5000


def test_m_suffix():
    assert str_to_int('2M') == 2000000


def test_b_suffix():
    assert str_to_int('3B') == 3000000000


def test_unparseable_returns_none():
    assert str_to_int('not a number') is None
