"""Tests for the InfoExtractor methods added in Slice 2 to support SVT.

Method contracts verified against `yt_dlp/extractor/common.py` @ tag
2026.03.17. Behaviour tests below should pass against either the upstream
class OR the rdlp shim.
"""
import pytest

from rdlp_ytdlp_compat import (
    ExtractorError,
    InfoExtractor,
    NO_DEFAULT,
)


# -----------------------------------------------------------------------------
# Fixtures: subclass IE shapes for the dispatch / classmethod tests.
# -----------------------------------------------------------------------------

class _ExampleIE(InfoExtractor):
    _VALID_URL = r"https?://example\.com/video/(?P<id>\w+)"


class _NoIdIE(InfoExtractor):
    """IE with `_VALID_URL` lacking a named `id` group — `_match_id` should
    raise IndexError/AttributeError when called."""
    _VALID_URL = r"https?://noid\.example/v/\w+"


# -----------------------------------------------------------------------------
# classmethods
# -----------------------------------------------------------------------------

class TestMatchValidUrl:
    def test_match_returns_match_object(self):
        m = _ExampleIE._match_valid_url("https://example.com/video/abc123")
        assert m is not None
        assert m.group("id") == "abc123"

    def test_no_match_returns_none(self):
        assert _ExampleIE._match_valid_url("https://other.com/foo") is None

    def test_caches_compiled_regex_per_class(self):
        # Calling twice should produce identical Match objects' classes
        # and not error. Implementation detail tested loosely.
        m1 = _ExampleIE._match_valid_url("https://example.com/video/x")
        m2 = _ExampleIE._match_valid_url("https://example.com/video/y")
        assert m1.group("id") == "x"
        assert m2.group("id") == "y"

    def test_compiled_without_verbose_flag(self):
        """Upstream `_match_valid_url` (`extractor/common.py:617-626`)
        compiles `_VALID_URL` with NO flags. Compiling with `re.VERBOSE`
        unconditionally would silently strip whitespace and `#` from
        single-line patterns that don't carry the `(?x)` inline flag.

        Direct assertion against compiled pattern flags is the only
        reliable way to pin this contract — string-match tests fail to
        detect VERBOSE because `re.match` is prefix-anchored and most
        patterns still produce SOME match even after VERBOSE corruption.
        """
        import re as _re

        class _PinFlagsIE(InfoExtractor):
            _VALID_URL = r"https?://flags\.example/(?P<id>\w+)"

        # Force compilation by dispatching once.
        _PinFlagsIE._match_valid_url("https://flags.example/x")
        assert hasattr(_PinFlagsIE, "_VALID_URL_RE")
        for compiled in _PinFlagsIE._VALID_URL_RE:
            assert not (compiled.flags & _re.VERBOSE), (
                f"_VALID_URL pattern was compiled with re.VERBOSE "
                f"(flags={compiled.flags}); upstream uses zero flags. "
                f"Inline `(?x)` is the correct mechanism for verbose "
                f"patterns."
            )

    def test_hash_literal_preserved_in_pattern(self):
        """Behavioural sibling of `test_compiled_without_verbose_flag`:
        a single-line pattern carrying a `#` literal must match URLs
        with that fragment, AND must NOT match URLs lacking it. Without
        VERBOSE the `#frag` literal is required; under VERBOSE it is
        stripped and the pattern degrades to "match anything before #".
        """
        class _FragIE(InfoExtractor):
            _VALID_URL = r"https?://frag\.example/(?P<id>\w+)#frag$"

        # The `#frag$` anchor MUST be enforced by the regex.
        assert _FragIE._match_valid_url(
            "https://frag.example/abc#frag",
        ) is not None
        # If VERBOSE strips `#frag$`, this URL would still match. It
        # MUST NOT under correct (no-flag) compilation.
        assert _FragIE._match_valid_url(
            "https://frag.example/abc#otherfragment",
        ) is None, (
            "pattern with `#frag$` anchor matched a URL ending in "
            "`#otherfragment` — VERBOSE flag stripped the literal `#frag`"
        )


class TestMatchId:
    def test_returns_id_group(self):
        assert _ExampleIE._match_id("https://example.com/video/foo42") == "foo42"

    def test_raises_when_no_id_group(self):
        # yt-dlp's `_match_id` raises IndexError on a regex without
        # `(?P<id>...)`. Caller expectation: catch via `get_temp_id`.
        with pytest.raises((IndexError, AttributeError)):
            _NoIdIE._match_id("https://noid.example/v/abc")


class TestIeKey:
    def test_strips_IE_suffix(self):
        assert _ExampleIE.ie_key() == "_Example"

    def test_real_class(self):
        class FooIE(InfoExtractor):
            _VALID_URL = r".*"
        assert FooIE.ie_key() == "Foo"

    def test_classmethod_callable_via_instance(self):
        # yt-dlp pattern: `extractor.ie_key()` from instance.
        ie = _ExampleIE()
        assert ie.ie_key() == "_Example"


# -----------------------------------------------------------------------------
# url_result / playlist_result
# -----------------------------------------------------------------------------

class TestUrlResult:
    def test_basic(self):
        r = InfoExtractor.url_result("https://example.com/x")
        assert r["_type"] == "url"
        assert r["url"] == "https://example.com/x"

    def test_with_ie_key_string(self):
        r = InfoExtractor.url_result("https://x.com/y", ie="SVTPlay", video_id="abc")
        assert r["ie_key"] == "SVTPlay"
        assert r["id"] == "abc"

    def test_with_ie_class(self):
        # SVT pattern: `self.url_result('svt:' + content_id, SVTPlayIE.ie_key(), content_id)`
        # — actually passes `ie_key()` result, not the class. Test both.
        r = InfoExtractor.url_result("https://x.com/y", ie=_ExampleIE)
        assert r["ie_key"] == "_Example"

    def test_with_video_title(self):
        r = InfoExtractor.url_result("https://x.com/y", video_title="My Video")
        assert r["title"] == "My Video"

    def test_url_transparent(self):
        r = InfoExtractor.url_result("https://x.com/y", url_transparent=True)
        assert r["_type"] == "url_transparent"


class TestPlaylistResult:
    def test_basic(self):
        entries = [{"id": "1"}, {"id": "2"}]
        r = InfoExtractor.playlist_result(entries, "pl-1", "My Playlist")
        assert r["_type"] == "playlist"
        assert r["id"] == "pl-1"
        assert r["title"] == "My Playlist"
        assert list(r["entries"]) == entries

    def test_with_description(self):
        r = InfoExtractor.playlist_result([], "pl", "T", "desc text")
        assert r["description"] == "desc text"

    def test_omits_empty_id_and_title(self):
        # Falsy `playlist_id` / `playlist_title` skipped in upstream
        # (`if playlist_id:`, `if playlist_title:`).
        r = InfoExtractor.playlist_result([])
        assert "id" not in r
        assert "title" not in r

    def test_multi_video(self):
        r = InfoExtractor.playlist_result([], multi_video=True)
        assert r["_type"] == "multi_video"


# -----------------------------------------------------------------------------
# OpenGraph helpers
# -----------------------------------------------------------------------------

class TestOgSearchTitle:
    def test_extracts_property_form(self):
        ie = _ExampleIE()
        html = '<meta property="og:title" content="My Video Title">'
        assert ie._og_search_title(html) == "My Video Title"

    def test_extracts_name_form(self):
        ie = _ExampleIE()
        html = '<meta name="og:title" content="Alt Form">'
        assert ie._og_search_title(html) == "Alt Form"

    def test_single_quotes(self):
        ie = _ExampleIE()
        html = "<meta property='og:title' content='Single Quoted'>"
        assert ie._og_search_title(html) == "Single Quoted"

    def test_missing_returns_none_default_fatal_false(self):
        # `_og_search_title` defaults to fatal=False — missing returns None.
        ie = _ExampleIE()
        assert ie._og_search_title("<html><head></head></html>") is None

    def test_unescapes_html_entities(self):
        ie = _ExampleIE()
        html = '<meta property="og:title" content="A &amp; B">'
        assert ie._og_search_title(html) == "A & B"


class TestOgSearchThumbnail:
    def test_extracts_image(self):
        ie = _ExampleIE()
        html = '<meta property="og:image" content="https://cdn.example/x.jpg">'
        assert ie._og_search_thumbnail(html) == "https://cdn.example/x.jpg"

    def test_missing_returns_none(self):
        ie = _ExampleIE()
        assert ie._og_search_thumbnail("<html></html>") is None


# -----------------------------------------------------------------------------
# JSON search
# -----------------------------------------------------------------------------

class TestSearchJson:
    def test_basic(self):
        ie = _ExampleIE()
        html = 'var data = {"key": "value", "n": 42};'
        result = ie._search_json(r"var data\s*=", html, "data", "vid1")
        assert result == {"key": "value", "n": 42}

    def test_with_end_pattern(self):
        ie = _ExampleIE()
        html = '<script id="x">{"a": 1}</script>'
        result = ie._search_json(
            r'<script[^>]+id="x"[^>]*>',
            html, "x-json", "vid", end_pattern="</script>",
        )
        assert result == {"a": 1}

    def test_default_returned_on_miss(self):
        ie = _ExampleIE()
        result = ie._search_json(
            r"NOT_PRESENT", "irrelevant", "x", "vid", default={"fallback": True},
        )
        assert result == {"fallback": True}

    def test_fatal_true_raises_on_miss(self):
        ie = _ExampleIE()
        with pytest.raises(ExtractorError):
            ie._search_json(r"NOT_PRESENT", "irrelevant", "x", "vid")

    def test_handles_nested_json(self):
        ie = _ExampleIE()
        html = 'var x = {"a": {"b": [1, 2, 3]}, "c": "d"};'
        result = ie._search_json(r"var x\s*=", html, "x", "vid")
        assert result == {"a": {"b": [1, 2, 3]}, "c": "d"}


class TestSearchNextjsData:
    def test_extracts_next_data(self):
        ie = _ExampleIE()
        html = (
            '<html><body>'
            '<script id="__NEXT_DATA__" type="application/json">'
            '{"props": {"page": "home"}}'
            '</script>'
            '</body></html>'
        )
        result = ie._search_nextjs_data(html, "vid1")
        assert result == {"props": {"page": "home"}}

    def test_default_on_miss(self):
        ie = _ExampleIE()
        result = ie._search_nextjs_data("<html></html>", "vid1", default={})
        assert result == {}

    def test_fatal_default_raises(self):
        ie = _ExampleIE()
        with pytest.raises(ExtractorError):
            ie._search_nextjs_data("<html></html>", "vid1")


# -----------------------------------------------------------------------------
# Subtitle merging
# -----------------------------------------------------------------------------

class TestMergeSubtitles:
    def test_merge_disjoint_languages(self):
        target = {"en": [{"url": "u-en"}]}
        new = {"sv": [{"url": "u-sv"}]}
        result = InfoExtractor._merge_subtitles(new, target=target)
        assert set(result.keys()) == {"en", "sv"}
        assert result["sv"] == [{"url": "u-sv"}]

    def test_merge_same_language_appends(self):
        target = {"sv": [{"url": "u1"}]}
        new = {"sv": [{"url": "u2"}]}
        result = InfoExtractor._merge_subtitles(new, target=target)
        # Both entries should appear — same lang merges by extending.
        urls = [item["url"] for item in result["sv"]]
        assert set(urls) == {"u1", "u2"}

    def test_merge_dedupes_identical_url(self):
        # yt-dlp's `_merge_subtitle_items` dedupes by (url, data) pair.
        target = {"sv": [{"url": "u1"}]}
        new = {"sv": [{"url": "u1"}]}
        result = InfoExtractor._merge_subtitles(new, target=target)
        assert result["sv"] == [{"url": "u1"}]

    def test_target_none_creates_dict(self):
        new = {"en": [{"url": "u"}]}
        result = InfoExtractor._merge_subtitles(new)
        assert result == {"en": [{"url": "u"}]}

    def test_merge_returns_target(self):
        target = {}
        result = InfoExtractor._merge_subtitles({"en": [{"url": "u"}]}, target=target)
        assert result is target  # mutates in place AND returns


# -----------------------------------------------------------------------------
# Geo verification headers
# -----------------------------------------------------------------------------

class TestGeoVerificationHeaders:
    def test_returns_dict(self):
        ie = _ExampleIE()
        # Without `geo_verification_proxy` param, returns empty dict —
        # we don't ship YoutubeDL params, so always empty in shim.
        result = ie.geo_verification_headers()
        assert isinstance(result, dict)


class TestDownloadWebpageQuery:
    """SVT's `SVTSeriesIE._real_extract` (line 313) calls
    `_download_json(url, slug, 'note', query={'query': '{...GraphQL...}'})`.
    The `query=` kwarg MUST serialise into the URL as `?key=value` before
    fetching, otherwise the GraphQL endpoint receives no payload."""

    def test_query_dict_appended_to_url(self, monkeypatch):
        # Capture the URL that `_host.fetch_text` is asked to fetch.
        captured = {}

        def fake_fetch_text(url, headers=None, timeout_ms=None,
                            expected_status=None):
            captured["url"] = url
            return "fake body"

        from rdlp_ytdlp_compat import _host
        monkeypatch.setattr(_host, "fetch_text", fake_fetch_text)

        ie = _ExampleIE()
        ie._download_webpage(
            "https://api.example.com/graphql",
            "vid1",
            query={"query": "{ user { id } }", "variables": "null"},
        )

        # URL must contain both query parameters URL-encoded.
        assert "?" in captured["url"], f"got: {captured['url']!r}"
        assert "query=" in captured["url"]
        assert "variables=" in captured["url"]
        # Special chars (braces, spaces) must be percent-encoded.
        assert " " not in captured["url"]
        assert "{" not in captured["url"]

    def test_query_none_leaves_url_unchanged(self, monkeypatch):
        captured = {}

        def fake_fetch_text(url, headers=None, timeout_ms=None,
                            expected_status=None):
            captured["url"] = url
            return ""

        from rdlp_ytdlp_compat import _host
        monkeypatch.setattr(_host, "fetch_text", fake_fetch_text)

        ie = _ExampleIE()
        ie._download_webpage("https://api.example.com/graphql", "vid1", query=None)
        assert captured["url"] == "https://api.example.com/graphql"

    def test_query_appended_to_existing_querystring(self, monkeypatch):
        captured = {}

        def fake_fetch_text(url, headers=None, timeout_ms=None,
                            expected_status=None):
            captured["url"] = url
            return ""

        from rdlp_ytdlp_compat import _host
        monkeypatch.setattr(_host, "fetch_text", fake_fetch_text)

        ie = _ExampleIE()
        ie._download_webpage(
            "https://api.example.com/path?existing=1",
            "vid1",
            query={"new": "value"},
        )
        # Both existing and new params must appear; separator is `&`.
        assert "existing=1" in captured["url"]
        assert "new=value" in captured["url"]


# -----------------------------------------------------------------------------
# DASH/F4M stubs (skipped per project_dash-protocol-missing memory)
# -----------------------------------------------------------------------------

class TestDeadFormatStubs:
    """F4M is a dead format; DASH (MPD) is unimplemented in
    rdlp-downloader. Both helpers exist for compatibility but return
    empty results without making network calls."""

    def test_f4m_formats_returns_empty(self):
        ie = _ExampleIE()
        result = ie._extract_f4m_formats(
            "https://example.com/x.f4m", "vid", fatal=False,
        )
        assert result == []

    def test_mpd_returns_empty_pair(self):
        ie = _ExampleIE()
        formats, subs = ie._extract_mpd_formats_and_subtitles(
            "https://example.com/x.mpd", "vid", fatal=False,
        )
        assert formats == []
        assert subs == {}
