"""Pure-Python unit tests for utility helpers (no host I/O)."""
import pytest
from rdlp_ytdlp_compat.info_extractor import (
    int_or_none, try_get, urljoin, unified_timestamp, InfoExtractor,
)


class TestIntOrNone:
    def test_none_input_returns_none(self):
        assert int_or_none(None) is None

    def test_string_digit_parses(self):
        assert int_or_none("42") == 42

    def test_invalid_returns_none(self):
        assert int_or_none("not a number") is None

    def test_scale_divides(self):
        assert int_or_none("1000", scale=1000) == 1
        assert int_or_none("999", scale=1000) == 0  # int division

    def test_default_when_invalid(self):
        assert int_or_none("oops", default=-1) == -1

    def test_invalid_string_with_default(self):
        assert int_or_none("abc", default=99) == 99

    def test_get_attr_reads_attribute(self):
        class O:
            x = "42"
        assert int_or_none(O(), get_attr="x") == 42

    def test_invscale_multiplies(self):
        # bytes -> bits
        assert int_or_none("10", invscale=8) == 80

    def test_base_radix(self):
        assert int_or_none("ff", base=16) == 255


class TestTryGet:
    def test_simple_path(self):
        d = {"a": {"b": 1}}
        assert try_get(d, lambda x: x["a"]["b"]) == 1

    def test_keyerror_returns_none(self):
        d = {"a": {}}
        assert try_get(d, lambda x: x["a"]["b"]) is None

    def test_typeerror_returns_none(self):
        assert try_get(None, lambda x: x["a"]) is None

    def test_expected_type_filter(self):
        d = {"a": "not an int"}
        assert try_get(d, lambda x: x["a"], expected_type=int) is None

    def test_expected_type_pass(self):
        d = {"a": 42}
        assert try_get(d, lambda x: x["a"], expected_type=int) == 42

    def test_iterable_of_getters_first_match_wins(self):
        d = {"b": 99}
        result = try_get(d, (lambda x: x["a"], lambda x: x["b"]))
        assert result == 99


class TestUrljoin:
    def test_absolute_url_unchanged(self):
        assert urljoin("https://example.com/a", "https://other.com/b") == "https://other.com/b"

    def test_relative_path(self):
        assert urljoin("https://example.com/a/b", "/c") == "https://example.com/c"

    def test_protocol_relative(self):
        assert urljoin("https://example.com/a", "//cdn.example.com/x") == "https://cdn.example.com/x"

    def test_none_base_returns_path_when_absolute(self):
        # yt-dlp behavior: returns path unchanged when base is None and path is absolute
        assert urljoin(None, "https://x.com/a") == "https://x.com/a"

    def test_none_path_returns_none(self):
        assert urljoin("https://example.com", None) is None

    def test_non_url_base_returns_none(self):
        # yt-dlp's behaviour: non-URL base produces None, not stdlib ValueError
        assert urljoin("not-a-url", "/path") is None

    def test_bytes_path_decoded(self):
        assert urljoin("https://example.com/a", b"/c") == "https://example.com/c"


class TestUnifiedTimestamp:
    def test_iso_8601_z(self):
        # 2026-04-29T12:00:00Z
        assert unified_timestamp("2026-04-29T12:00:00Z") == 1777464000

    def test_rfc_2822(self):
        assert unified_timestamp("Wed, 29 Apr 2026 12:00:00 GMT") == 1777464000

    def test_invalid_returns_none(self):
        assert unified_timestamp("not a date") is None

    def test_none_returns_none(self):
        assert unified_timestamp(None) is None


class TestParseJson:
    def test_valid_json(self):
        ie = InfoExtractor()
        assert ie._parse_json('{"a": 1}', "x") == {"a": 1}

    def test_transform_source(self):
        ie = InfoExtractor()
        # yt-dlp pattern: strip a JSONP wrapper
        assert ie._parse_json("callback({\"a\": 1})", "x",
                              transform_source=lambda s: s[s.index("(")+1:s.rindex(")")]) == {"a": 1}

    def test_invalid_non_fatal_returns_none(self):
        ie = InfoExtractor()
        assert ie._parse_json("not json", "x", fatal=False) is None

    def test_invalid_fatal_raises(self):
        ie = InfoExtractor()
        with pytest.raises((ValueError, TypeError)):
            ie._parse_json("not json", "x", fatal=True)

    def test_lenient_kwarg_does_not_exist(self):
        # yt-dlp doesn't have a lenient kwarg; **parser_kwargs will pass it to
        # json.loads which rejects unknown kwargs. This test pins the negative.
        ie = InfoExtractor()
        with pytest.raises(TypeError):
            # `lenient=True` is not a valid kwarg for json.loads
            ie._parse_json('{"a": 1}', "x", lenient=True)
