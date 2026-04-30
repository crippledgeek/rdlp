"""yt-dlp's `url_or_none` (utils/_utils.py @ tag 2026.03.17)."""
from rdlp_ytdlp_compat import url_or_none


def test_none_returns_none():
    assert url_or_none(None) is None


def test_non_string_returns_none():
    assert url_or_none(123) is None


def test_http_url_passes():
    assert url_or_none("http://example.com/x") == "http://example.com/x"


def test_https_url_passes():
    assert url_or_none("https://example.com/x") == "https://example.com/x"


def test_protocol_relative_passes():
    assert url_or_none("//example.com/x") == "//example.com/x"


def test_relative_path_returns_none():
    assert url_or_none("/just/a/path") is None


def test_javascript_uri_returns_none():
    assert url_or_none("javascript:alert(1)") is None


def test_data_uri_passes():
    assert url_or_none("data:image/png;base64,abc") == "data:image/png;base64,abc"
