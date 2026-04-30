"""Slice-2 follow-up helpers for xxxymovies plugin port.

Mirrors:
  - clean_html — yt_dlp/utils/_utils.py:527-540
  - parse_duration — yt_dlp/utils/_utils.py:2082-2136
  - _html_search_regex — yt_dlp/extractor/common.py:1379-1386
  - _rta_search — yt_dlp/extractor/common.py:1525-1543

Each helper kept verbatim to upstream behaviour for the inputs the
xxxymovies extractor actually feeds it (HTML stripping, MM:SS-format
duration, RTA-5042 meta tag detection).
"""

from rdlp_ytdlp_compat import InfoExtractor
from rdlp_ytdlp_compat._utils import clean_html, parse_duration


class TestCleanHtml:
    def test_none_passes_through(self):
        assert clean_html(None) is None

    def test_strips_simple_tags(self):
        assert clean_html("<p>hello</p>") == "hello"

    def test_collapses_whitespace(self):
        # Upstream `_utils.py:533` collapses any whitespace run to a
        # single space BEFORE stripping tags.
        assert clean_html("  <p>foo   bar</p>  ") == "foo bar"

    def test_br_becomes_newline(self):
        # `<br/>` and `<br>` and `< br />` all → '\n'.
        assert clean_html("a<br/>b") == "a\nb"
        assert clean_html("a<br>b") == "a\nb"
        assert clean_html("a< br />b") == "a\nb"

    def test_p_to_p_becomes_newline(self):
        assert clean_html("<p>a</p><p>b</p>") == "a\nb"

    def test_unescapes_html_entities(self):
        assert clean_html("Tom &amp; Jerry") == "Tom & Jerry"
        assert clean_html("&lt;tag&gt;") == "<tag>"

    def test_strips_attributes_inside_tags(self):
        assert (
            clean_html('<a href="x" class="y">link</a>')
            == "link"
        )


class TestParseDuration:
    def test_none_returns_none(self):
        assert parse_duration(None) is None

    def test_non_string_returns_none(self):
        assert parse_duration(42) is None

    def test_empty_returns_none(self):
        assert parse_duration("") is None
        assert parse_duration("   ") is None

    def test_mm_ss(self):
        # xxxymovies's "Duration:" markup is "MM:SS"
        assert parse_duration("15:31") == 15 * 60 + 31
        assert parse_duration("00:42") == 42

    def test_hh_mm_ss(self):
        assert parse_duration("1:02:03") == 3600 + 2 * 60 + 3

    def test_dd_hh_mm_ss(self):
        assert parse_duration("2:01:02:03") == 2 * 86400 + 3600 + 2 * 60 + 3

    def test_seconds_only(self):
        assert parse_duration("42") == 42

    def test_milliseconds_dot(self):
        assert parse_duration("1:30.5") == 90.5

    def test_iso8601_h_m_s(self):
        # yt-dlp accepts "PT1H2M3S" and bare "1h2m3s" forms.
        assert parse_duration("1h2m3s") == 3600 + 2 * 60 + 3

    def test_unparseable_returns_none(self):
        assert parse_duration("not a duration") is None


class _XxxIE(InfoExtractor):
    """Subclass for testing helper methods that need an instance."""
    _VALID_URL = r"https?://example\.com/(?P<id>\w+)"


class TestHtmlSearchRegex:
    """`_html_search_regex` is `_search_regex` + clean_html on result."""

    def test_strips_tags_from_match(self):
        ie = _XxxIE()
        html = '<div class="title"><span>Foo</span> Bar</div>'
        result = ie._html_search_regex(
            r'<div class="title">(.+?)</div>', html, "title",
        )
        assert result == "Foo Bar"

    def test_unescapes_entities(self):
        ie = _XxxIE()
        html = '<title>Tom &amp; Jerry - Cartoons</title>'
        result = ie._html_search_regex(
            r'<title>(.+?) - ', html, "title",
        )
        assert result == "Tom & Jerry"

    def test_passes_through_default_on_miss(self):
        ie = _XxxIE()
        result = ie._html_search_regex(
            r'no-match', "<html></html>", "x",
            default=None, fatal=False,
        )
        assert result is None

    def test_handles_pattern_list(self):
        # xxxymovies passes a list of two regexes as `pattern`. First
        # match wins, then result is HTML-cleaned.
        ie = _XxxIE()
        html = "<title>Real Title - XXXYMovies.com</title>"
        result = ie._html_search_regex(
            [r"<div class=block_header>([^<]+)</div>",
             r"<title>(.+?) - XXXYMovies\.com</title>"],
            html, "title",
        )
        assert result == "Real Title"


class TestRtaSearch:
    """`_rta_search(html)` returns 18 if any RTA / age-restriction
    marker is found, else None."""

    def test_official_rta_meta_returns_18(self):
        html = '<meta name="rating" content="RTA-5042-1996-1400-1577-RTA">'
        assert _XxxIE._rta_search(html) == 18

    def test_proudly_labeled_marker_returns_18(self):
        # The fallback uses the "Proudly Labeled <a href...>" pattern.
        full = (
            'Proudly Labeled <a href="http://www.rtalabel.org/" '
            'title="Restricted to Adults">RTA</a>'
        )
        assert _XxxIE._rta_search(full) == 18

    def test_acknowledge_18_marker_returns_18(self):
        html = ">you acknowledge you are at least 18 years old"
        assert _XxxIE._rta_search(html) == 18

    def test_2257_marker_returns_18(self):
        html = "> 18 U.S.C. § 2257 statement"
        assert _XxxIE._rta_search(html) == 18

    def test_no_marker_returns_none(self):
        assert _XxxIE._rta_search("<html><body>nothing</body></html>") is None
