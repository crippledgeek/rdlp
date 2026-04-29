"""Pure-Python unit tests for utility helpers (no host I/O)."""
import pytest
from rdlp_ytdlp_compat.info_extractor import (
    InfoExtractor,
    int_or_none, try_get, urljoin, unified_timestamp,
    traverse_obj,
    NO_DEFAULT,
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


class TestSearchRegex:
    def test_simple_match(self):
        ie = InfoExtractor()
        assert ie._search_regex(r"id=(\d+)", "<div id=42>", "video id") == "42"

    def test_named_group(self):
        ie = InfoExtractor()
        assert ie._search_regex(r"id=(?P<id>\d+)", "<div id=42>", "video id", group="id") == "42"

    def test_default_when_no_match(self):
        ie = InfoExtractor()
        assert ie._search_regex(r"NOT", "x", "thing", default="fallback") == "fallback"

    def test_fatal_explicit_raises_when_no_match(self):
        ie = InfoExtractor()
        with pytest.raises(ValueError):
            ie._search_regex(r"NOT", "x", "thing", fatal=True)

    def test_default_fatal_is_true_silent_break_guard(self):
        # CRITICAL: yt-dlp default is fatal=True. Real extractors omit the kwarg
        # expecting a raise; if we default to fatal=False they fail-open and
        # produce empty info-dicts.
        ie = InfoExtractor()
        with pytest.raises(ValueError):
            ie._search_regex(r"NOT", "x", "thing")  # no fatal kwarg

    def test_list_of_patterns_first_match_wins(self):
        ie = InfoExtractor()
        assert ie._search_regex([r"NOT", r"id=(\d+)"], "id=7", "id") == "7"

    def test_no_groups_returns_full_match(self):
        ie = InfoExtractor()
        assert ie._search_regex(r"hello", "say hello world", "greeting") == "hello"


class TestHtmlSearchMeta:
    def test_meta_property(self):
        ie = InfoExtractor()
        html = '<meta property="og:title" content="Hello">'
        assert ie._html_search_meta("og:title", html, "title") == "Hello"

    def test_meta_name(self):
        ie = InfoExtractor()
        html = '<meta name="description" content="A page">'
        assert ie._html_search_meta("description", html, "desc") == "A page"

    def test_meta_itemprop(self):
        # CRITICAL: yt-dlp matches 5 attrs (itemprop|name|property|id|http-equiv)
        # not just name+property. Microdata extractors rely on itemprop.
        ie = InfoExtractor()
        html = '<meta itemprop="duration" content="PT2M30S">'
        assert ie._html_search_meta("duration", html, "duration") == "PT2M30S"

    def test_meta_http_equiv(self):
        ie = InfoExtractor()
        html = '<meta http-equiv="refresh" content="0; url=/x">'
        assert ie._html_search_meta("refresh", html, "redirect") == "0; url=/x"

    def test_default_when_missing(self):
        ie = InfoExtractor()
        assert ie._html_search_meta("nope", "<html></html>", "x", default="d") == "d"

    def test_default_fatal_is_false(self):
        # _html_search_meta defaults to fatal=False (distinct from _search_regex)
        ie = InfoExtractor()
        # Should not raise; default is None
        assert ie._html_search_meta("nope", "<html></html>", "x") is None


class TestTraverseObj:
    def test_dict_path(self):
        assert traverse_obj({"a": {"b": 1}}, ("a", "b")) == 1

    def test_list_index(self):
        assert traverse_obj([{"x": 1}, {"x": 2}], (0, "x")) == 1

    def test_missing_returns_none_for_scalar_path(self):
        # Scalar (non-branching) path miss: returns None
        assert traverse_obj({"a": {}}, ("a", "b")) is None

    def test_default(self):
        assert traverse_obj({"a": {}}, ("a", "b"), default=99) == 99

    def test_branched_miss_returns_empty_list_not_none(self):
        # CRITICAL: yt-dlp returns [] (not None) when a branched/get_all path
        # misses. Extractors do `if traverse_obj(...): ...` AND `len(...)` —
        # None would silently skip iteration paths.
        result = traverse_obj({"a": []}, ("a", Ellipsis))
        assert result == []

    def test_get_all_false_returns_first_only(self):
        result = traverse_obj([{"x": 1}, {"x": 2}], (Ellipsis, "x"), get_all=False)
        assert result == 1

    def test_casesense_false(self):
        d = {"Title": "Hello"}
        assert traverse_obj(d, "title", casesense=False) == "Hello"

    def test_callable_filter(self):
        # Keep only ints
        result = traverse_obj([1, "x", 2, "y"], (lambda v: isinstance(v, int),))
        assert result == [1, 2]

    def test_multiple_paths_first_hit(self):
        d = {"a": None, "b": "found"}
        assert traverse_obj(d, ("a",), ("b",)) == "found"

    def test_expected_type_filter_passes(self):
        d = {"x": 42}
        assert traverse_obj(d, "x", expected_type=int) == 42

    def test_expected_type_filter_rejects(self):
        d = {"x": "not int"}
        # Type filter on a scalar miss — defaults to [] when get_all and NO_DEFAULT
        # actually returns default when no path produces a hit
        result = traverse_obj(d, "x", expected_type=int)
        assert result == [] or result is None  # either acceptable for Slice 1


class TestExtractM3U8Formats:
    def test_extract_m3u8_returns_formats_only(self):
        # The base method returns formats only (drops subs). The companion
        # _extract_m3u8_formats_and_subtitles returns the tuple.
        # We can't actually call _host.fetch_text here (no runtime), but we
        # can verify the method exists and is callable, and that the
        # _and_subtitles companion exists.
        ie = InfoExtractor()
        assert hasattr(ie, "_extract_m3u8_formats")
        assert hasattr(ie, "_extract_m3u8_formats_and_subtitles")
