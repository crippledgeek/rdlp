"""Verify Python InfoExtractor methods correctly delegate to _host."""
from unittest.mock import MagicMock

import pytest

from rdlp_ytdlp_compat import InfoExtractor


@pytest.fixture
def ie():
    class _Concrete(InfoExtractor):
        _VALID_URL = r".*"
    return _Concrete()


def test_search_regex_calls_host(monkeypatch, ie):
    captured = {}
    def fake(pat, s, flags):
        captured["pat"] = pat; captured["s"] = s; captured["flags"] = flags
        return "matched"
    from rdlp_ytdlp_compat import _host
    monkeypatch.setattr(_host, "search_regex", fake)

    result = ie._search_regex(r"foo", "string foo bar", "name")

    assert result == "matched"
    assert captured["pat"] == r"foo"
    assert captured["s"] == "string foo bar"


def test_search_regex_returns_default_on_miss(monkeypatch, ie):
    from rdlp_ytdlp_compat import _host
    monkeypatch.setattr(_host, "search_regex", lambda *a, **k: None)
    assert ie._search_regex(r"x", "y", "name", default="fallback", fatal=False) == "fallback"


def test_search_regex_raises_on_miss_fatal(monkeypatch, ie):
    from rdlp_ytdlp_compat import _host, RegexNotFoundError
    monkeypatch.setattr(_host, "search_regex", lambda *a, **k: None)
    with pytest.raises(RegexNotFoundError):
        ie._search_regex(r"x", "y", "name")


def test_html_search_regex_calls_host(monkeypatch, ie):
    from rdlp_ytdlp_compat import _host
    monkeypatch.setattr(_host, "html_search_regex", lambda pat, s, f: "Hello World")
    result = ie._html_search_regex(r"<title>(.+?)</title>",
                                    "<title>Hello <b>World</b></title>", "title")
    assert result == "Hello World"


def test_html_search_meta_calls_host(monkeypatch, ie):
    from rdlp_ytdlp_compat import _host
    monkeypatch.setattr(_host, "html_search_meta", lambda name, html: "value")
    result = ie._html_search_meta("og:title", "<html/>")
    assert result == "value"
