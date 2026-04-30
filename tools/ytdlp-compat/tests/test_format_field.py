"""yt-dlp's `format_field` — conditional template formatter."""
from rdlp_ytdlp_compat import format_field


def test_field_present_uses_template():
    obj = {"x": 42}
    assert format_field(obj, "x", "[%s]") == "[42]"


def test_field_missing_returns_default():
    obj = {"x": 42}
    assert format_field(obj, "missing", "[%s]") == ""


def test_field_present_default_template_is_value():
    obj = {"x": "hello"}
    assert format_field(obj, "x") == "hello"


def test_field_none_returns_default():
    obj = {"x": None}
    assert format_field(obj, "x", "[%s]", default="empty") == "empty"
