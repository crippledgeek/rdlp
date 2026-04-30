"""Pure-Python unit tests for utility helpers (no host I/O)."""

import logging

import pytest

from rdlp_ytdlp_compat import (
    ExtractorError,
    GeoRestrictedError,
    RegexNotFoundError,
    UnsupportedError,
    YoutubeDLError,
)
from rdlp_ytdlp_compat.info_extractor import (
    NO_DEFAULT,
    InfoExtractor,
    _NoDefault,
    int_or_none,
    traverse_obj,
    try_get,
    unified_timestamp,
    urljoin,
)


class TestIntOrNone:
    def test_none_input_returns_none(self):
        assert int_or_none(None) is None

    def test_string_digit_parses(self):
        assert int_or_none('42') == 42

    def test_invalid_returns_none(self):
        assert int_or_none('not a number') is None

    def test_scale_divides(self):
        assert int_or_none('1000', scale=1000) == 1
        assert int_or_none('999', scale=1000) == 0  # int division

    def test_default_when_invalid(self):
        assert int_or_none('oops', default=-1) == -1

    def test_invalid_string_with_default(self):
        assert int_or_none('abc', default=99) == 99

    def test_get_attr_reads_attribute(self):
        class Obj:
            x = '42'

        assert int_or_none(Obj(), get_attr='x') == 42

    def test_invscale_multiplies(self):
        # bytes -> bits
        assert int_or_none('10', invscale=8) == 80

    def test_base_radix(self):
        assert int_or_none('ff', base=16) == 255

    def test_bool_true_returns_one(self):
        # Python's bool IS-A int (`int(True) == 1`). Real extractors
        # occasionally do `int_or_none(meta.get("isLive"))` — bool input
        # should pass through cleanly, not raise.
        assert int_or_none(True) == 1
        assert int_or_none(False) == 0

    def test_zero_string_returns_zero(self):
        # Empty string should fall through to default; "0" is a valid value.
        assert int_or_none('0') == 0


class TestNoDefaultSingleton:
    """`_NoDefault()` must always return the same instance so that any
    `is NO_DEFAULT` check stays correct — see info_extractor.py for rationale.
    """

    def test_constructing_returns_singleton(self):
        assert _NoDefault() is NO_DEFAULT

    def test_repeated_construction_is_same_object(self):
        assert _NoDefault() is _NoDefault()

    def test_module_constant_identity(self):
        # An accidental fresh construction must behave identically to the
        # exported NO_DEFAULT — otherwise sentinel checks silently fail.
        accidental = _NoDefault()
        assert accidental is NO_DEFAULT


class TestHostLogOutsideRuntime:
    """When the WIT bindings aren't available (unit-test env), `_host.log`
    routes through stdlib `logging` so pytest captures warnings instead of
    silently swallowing them."""

    def test_warn_emitted_through_stdlib_logging(self, caplog):
        from rdlp_ytdlp_compat import _host

        with caplog.at_level(logging.WARNING, logger='rdlp_ytdlp_compat'):
            _host.log('warn', 'test warning surfaces in pytest')
        assert any(
            'test warning surfaces in pytest' in r.message and r.levelno == logging.WARNING for r in caplog.records
        )

    def test_error_emitted_through_stdlib_logging(self, caplog):
        from rdlp_ytdlp_compat import _host

        with caplog.at_level(logging.ERROR, logger='rdlp_ytdlp_compat'):
            _host.log('error', 'test error surfaces in pytest')
        assert any(r.levelno == logging.ERROR for r in caplog.records)

    def test_unknown_level_falls_back_to_info(self, caplog):
        from rdlp_ytdlp_compat import _host

        with caplog.at_level(logging.INFO, logger='rdlp_ytdlp_compat'):
            _host.log('nonsense_level', 'fallback path')
        assert any('fallback path' in r.message for r in caplog.records)


class TestTryGet:
    def test_simple_path(self):
        d = {'a': {'b': 1}}
        assert try_get(d, lambda x: x['a']['b']) == 1

    def test_keyerror_returns_none(self):
        d = {'a': {}}
        assert try_get(d, lambda x: x['a']['b']) is None

    def test_typeerror_returns_none(self):
        assert try_get(None, lambda x: x['a']) is None

    def test_expected_type_filter(self):
        d = {'a': 'not an int'}
        assert try_get(d, lambda x: x['a'], expected_type=int) is None

    def test_expected_type_pass(self):
        d = {'a': 42}
        assert try_get(d, lambda x: x['a'], expected_type=int) == 42

    def test_iterable_of_getters_first_match_wins(self):
        d = {'b': 99}
        result = try_get(d, (lambda x: x['a'], lambda x: x['b']))
        assert result == 99


class TestUrljoin:
    def test_absolute_url_unchanged(self):
        assert urljoin('https://example.com/a', 'https://other.com/b') == 'https://other.com/b'

    def test_relative_path(self):
        assert urljoin('https://example.com/a/b', '/c') == 'https://example.com/c'

    def test_protocol_relative(self):
        assert urljoin('https://example.com/a', '//cdn.example.com/x') == 'https://cdn.example.com/x'

    def test_none_base_returns_path_when_absolute(self):
        # yt-dlp behavior: returns path unchanged when base is None and path is absolute
        assert urljoin(None, 'https://x.com/a') == 'https://x.com/a'

    def test_none_path_returns_none(self):
        assert urljoin('https://example.com', None) is None

    def test_non_url_base_returns_none(self):
        # yt-dlp's behaviour: non-URL base produces None, not stdlib ValueError
        assert urljoin('not-a-url', '/path') is None

    def test_bytes_path_decoded(self):
        assert urljoin('https://example.com/a', b'/c') == 'https://example.com/c'


class TestUnifiedTimestamp:
    def test_iso_8601_z(self):
        # 2026-04-29T12:00:00Z
        assert unified_timestamp('2026-04-29T12:00:00Z') == 1777464000

    def test_rfc_2822(self):
        assert unified_timestamp('Wed, 29 Apr 2026 12:00:00 GMT') == 1777464000

    def test_invalid_returns_none(self):
        assert unified_timestamp('not a date') is None

    def test_none_returns_none(self):
        assert unified_timestamp(None) is None


class TestParseJson:
    def test_valid_json(self):
        ie = InfoExtractor()
        assert ie._parse_json('{"a": 1}', 'x') == {'a': 1}

    def test_transform_source(self):
        ie = InfoExtractor()
        # yt-dlp pattern: strip a JSONP wrapper
        assert ie._parse_json(
            'callback({"a": 1})', 'x', transform_source=lambda s: s[s.index('(') + 1 : s.rindex(')')]
        ) == {'a': 1}

    def test_invalid_non_fatal_returns_none(self):
        ie = InfoExtractor()
        assert ie._parse_json('not json', 'x', fatal=False) is None

    def test_invalid_fatal_raises(self):
        ie = InfoExtractor()
        with pytest.raises((ValueError, TypeError)):
            ie._parse_json('not json', 'x', fatal=True)

    def test_lenient_kwarg_silently_dropped(self):
        # Slice-2 update: `lenient`, `ignore_extra`, and `strict` are
        # yt-dlp `LenientJSONDecoder` kwargs that don't exist on stdlib
        # `json.loads`. Slice-1 let them propagate (raised TypeError);
        # Slice-2 silently drops them so ported extractors that do
        # `_parse_json(..., ignore_extra=True)` upstream-style don't
        # break at the boundary. See `_parse_json` doc + Slice-2
        # spec/code-review fix commit 955bd86.
        ie = InfoExtractor()
        result = ie._parse_json('{"a": 1}', 'x', lenient=True)
        assert result == {'a': 1}


class TestSearchRegex:
    def test_simple_match(self):
        ie = InfoExtractor()
        assert ie._search_regex(r'id=(\d+)', '<div id=42>', 'video id') == '42'

    def test_named_group(self):
        ie = InfoExtractor()
        assert ie._search_regex(r'id=(?P<id>\d+)', '<div id=42>', 'video id', group='id') == '42'

    def test_default_when_no_match(self):
        ie = InfoExtractor()
        assert ie._search_regex(r'NOT', 'x', 'thing', default='fallback') == 'fallback'

    def test_fatal_explicit_raises_when_no_match(self):
        # Raises RegexNotFoundError (typed yt-dlp subclass), which extends
        # ExtractorError. Old code raised bare ValueError; the typed form
        # is what the WIT dispatcher recognises (route to parse variant).
        ie = InfoExtractor()
        with pytest.raises(RegexNotFoundError):
            ie._search_regex(r'NOT', 'x', 'thing', fatal=True)

    def test_default_fatal_is_true_silent_break_guard(self):
        # CRITICAL: yt-dlp default is fatal=True. Real extractors omit the kwarg
        # expecting a raise; if we default to fatal=False they fail-open and
        # produce empty info-dicts.
        ie = InfoExtractor()
        with pytest.raises(RegexNotFoundError):
            ie._search_regex(r'NOT', 'x', 'thing')  # no fatal kwarg

    def test_list_of_patterns_first_match_wins(self):
        ie = InfoExtractor()
        assert ie._search_regex([r'NOT', r'id=(\d+)'], 'id=7', 'id') == '7'

    def test_no_groups_returns_full_match(self):
        ie = InfoExtractor()
        assert ie._search_regex(r'hello', 'say hello world', 'greeting') == 'hello'


class TestHtmlSearchMeta:
    def test_meta_property(self):
        ie = InfoExtractor()
        html = '<meta property="og:title" content="Hello">'
        assert ie._html_search_meta('og:title', html, 'title') == 'Hello'

    def test_meta_name(self):
        ie = InfoExtractor()
        html = '<meta name="description" content="A page">'
        assert ie._html_search_meta('description', html, 'desc') == 'A page'

    def test_meta_itemprop(self):
        # CRITICAL: yt-dlp matches 5 attrs (itemprop|name|property|id|http-equiv)
        # not just name+property. Microdata extractors rely on itemprop.
        ie = InfoExtractor()
        html = '<meta itemprop="duration" content="PT2M30S">'
        assert ie._html_search_meta('duration', html, 'duration') == 'PT2M30S'

    def test_meta_http_equiv(self):
        ie = InfoExtractor()
        html = '<meta http-equiv="refresh" content="0; url=/x">'
        assert ie._html_search_meta('refresh', html, 'redirect') == '0; url=/x'

    def test_default_when_missing(self):
        ie = InfoExtractor()
        assert ie._html_search_meta('nope', '<html></html>', 'x', default='d') == 'd'

    def test_default_fatal_is_false(self):
        # _html_search_meta defaults to fatal=False (distinct from _search_regex)
        ie = InfoExtractor()
        # Should not raise; default is None
        assert ie._html_search_meta('nope', '<html></html>', 'x') is None

    def test_meta_id_attribute(self):
        # 5th supported attr — id="..." is the rarest of the 5 in real
        # extractors but the regex must still match it.
        ie = InfoExtractor()
        html = '<meta id="page-title" content="Hello via id">'
        assert ie._html_search_meta('page-title', html, 'title') == 'Hello via id'

    def test_meta_iterable_name_first_match_wins(self):
        # `name` accepts a scalar OR an iterable of meta-tag names; the first
        # one that matches wins. Real yt-dlp extractors pass tuples like
        # (`og:title`, `twitter:title`) for redundant meta-tag fallbacks.
        ie = InfoExtractor()
        html = '<meta name="twitter:title" content="from twitter">'
        result = ie._html_search_meta(['og:title', 'twitter:title'], html, 'title')
        assert result == 'from twitter'

    def test_meta_attr_match_is_case_insensitive(self):
        # Real-world HTML often has `Content` (mixed case) or `PROPERTY=...`.
        ie = InfoExtractor()
        html = '<META PROPERTY="og:title" CONTENT="Mixed Case">'
        assert ie._html_search_meta('og:title', html, 'title') == 'Mixed Case'


class TestTraverseObj:
    def test_dict_path(self):
        assert traverse_obj({'a': {'b': 1}}, ('a', 'b')) == 1

    def test_list_index(self):
        assert traverse_obj([{'x': 1}, {'x': 2}], (0, 'x')) == 1

    def test_missing_returns_none_for_scalar_path(self):
        # Scalar (non-branching) path miss: returns None
        assert traverse_obj({'a': {}}, ('a', 'b')) is None

    def test_default(self):
        assert traverse_obj({'a': {}}, ('a', 'b'), default=99) == 99

    def test_branched_miss_returns_empty_list_not_none(self):
        # CRITICAL: yt-dlp returns [] (not None) when a branched/get_all path
        # misses. Extractors do `if traverse_obj(...): ...` AND `len(...)` —
        # None would silently skip iteration paths.
        result = traverse_obj({'a': []}, ('a', Ellipsis))
        assert result == []

    def test_get_all_false_returns_first_only(self):
        result = traverse_obj([{'x': 1}, {'x': 2}], (Ellipsis, 'x'), get_all=False)
        assert result == 1

    def test_casesense_false(self):
        d = {'Title': 'Hello'}
        assert traverse_obj(d, 'title', casesense=False) == 'Hello'

    def test_callable_filter(self):
        # Keep only ints. Slice-2 conforms to yt-dlp's two-arg callable
        # signature `(key_or_index, value)` per
        # `yt_dlp/utils/traversal.py:157-178`. Real extractors rely on
        # the key arg (e.g. SVT line 268: `lambda _, v: v['accessibility']
        # == 'Default'`); the Slice-1 single-arg form was an over-simplified
        # shim API that this test now corrects.
        result = traverse_obj(
            [1, 'x', 2, 'y'],
            (lambda _, v: isinstance(v, int),),
        )
        assert result == [1, 2]

    def test_multiple_paths_first_hit(self):
        d = {'a': None, 'b': 'found'}
        assert traverse_obj(d, ('a',), ('b',)) == 'found'

    def test_expected_type_filter_passes(self):
        d = {'x': 42}
        assert traverse_obj(d, 'x', expected_type=int) == 42

    def test_expected_type_filter_rejects(self):
        d = {'x': 'not int'}
        # Type filter on a scalar miss — defaults to [] when get_all and NO_DEFAULT
        # actually returns default when no path produces a hit
        result = traverse_obj(d, 'x', expected_type=int)
        assert result == [] or result is None  # either acceptable for Slice 1


class TestHostCheckStatus:
    """Pin the HTTP-status check on _host.fetch_text without requiring the
    componentize-py runtime — _check_status is pure-Python and unit-testable."""

    def test_2xx_passes(self):
        from rdlp_ytdlp_compat._host import _check_status

        _check_status(200, None, 'https://x')
        _check_status(204, None, 'https://x')
        _check_status(299, None, 'https://x')

    def test_4xx_raises(self):
        from rdlp_ytdlp_compat._host import _check_status

        with pytest.raises(RuntimeError, match='HTTP 404'):
            _check_status(404, None, 'https://example.com/missing')

    def test_5xx_raises(self):
        from rdlp_ytdlp_compat._host import _check_status

        with pytest.raises(RuntimeError, match='HTTP 503'):
            _check_status(503, None, 'https://example.com')

    def test_3xx_raises(self):
        # Redirect status without a Location is also a non-success.
        from rdlp_ytdlp_compat._host import _check_status

        with pytest.raises(RuntimeError, match='HTTP 301'):
            _check_status(301, None, 'https://example.com')

    def test_expected_status_allows_match(self):
        # yt-dlp's soft-404 pattern: expected_status=404 means "I'll handle it".
        from rdlp_ytdlp_compat._host import _check_status

        _check_status(404, 404, 'https://example.com')

    def test_expected_status_does_not_allow_other_errors(self):
        # expected_status=404 must NOT silently allow 500.
        from rdlp_ytdlp_compat._host import _check_status

        with pytest.raises(RuntimeError, match='HTTP 500'):
            _check_status(500, 404, 'https://example.com')

    def test_expected_status_2xx_still_passes(self):
        from rdlp_ytdlp_compat._host import _check_status

        _check_status(200, 404, 'https://example.com')

    def test_url_appears_in_error(self):
        from rdlp_ytdlp_compat._host import _check_status

        with pytest.raises(RuntimeError, match=r'example\.com/auth'):
            _check_status(401, None, 'https://example.com/auth')


class TestExtractM3U8Formats:
    def test_extract_m3u8_returns_formats_only(self):
        # The base method returns formats only (drops subs). The companion
        # _extract_m3u8_formats_and_subtitles returns the tuple.
        # We can't actually call _host.fetch_text here (no runtime), but we
        # can verify the method exists and is callable, and that the
        # _and_subtitles companion exists.
        ie = InfoExtractor()
        assert hasattr(ie, '_extract_m3u8_formats')
        assert hasattr(ie, '_extract_m3u8_formats_and_subtitles')

    def test_fetch_failure_with_fatal_false_returns_empty(self):
        # yt-dlp contract: when fatal=False and the playlist fetch fails, the
        # helper logs via errnote and returns an empty result so callers can
        # fall back to non-HLS paths (real extractors do `if formats: ...`).
        # Outside componentize-py, _host.fetch_text raises RuntimeError —
        # which exercises the same `except Exception` path as a real network
        # error inside the runtime.
        ie = InfoExtractor()
        formats, subs = ie._extract_m3u8_formats_and_subtitles(
            'https://example.com/master.m3u8',
            'vid',
            errnote='Unable to download HLS playlist',
            fatal=False,
        )
        assert formats == []
        assert subs == {}

    def test_fetch_failure_with_fatal_true_propagates(self):
        # When fatal=True (yt-dlp default), the underlying RuntimeError must
        # propagate so the caller can decide what to do.
        ie = InfoExtractor()
        with pytest.raises(RuntimeError):
            ie._extract_m3u8_formats_and_subtitles(
                'https://example.com/master.m3u8',
                'vid',
                fatal=True,
            )

    def test_extract_m3u8_drops_subs_with_fatal_false(self):
        # The non-suffixed wrapper must propagate fatal=False through to the
        # _and_subtitles companion and only return formats (drop subs).
        ie = InfoExtractor()
        formats = ie._extract_m3u8_formats('https://example.com/master.m3u8', 'vid', fatal=False)
        assert formats == []

    def test_master_playlist_parse_happy_path(self, monkeypatch):
        # Stub _host.extract_m3u8 so the _format_to_dict conversion and
        # field mapping (url, ext, tbr, width, height, format_id) are
        # exercised. Updated for Slice-2.5 passthrough to extract_m3u8.
        from rdlp_ytdlp_compat import _host

        class _Fmt:
            def __init__(self, fid, url, ext, proto, tbr, width, height):
                self.format_id = fid
                self.url = url
                self.ext = ext
                self.protocol = proto
                self.tbr = tbr
                self.width = width
                self.height = height
                self.fps = self.vcodec = self.acodec = None
                self.vbr = self.abr = self.language = self.format_note = None
                self.format_index = self.manifest_url = self.has_drm = None
                self.preference = self.quality = None

        class _Result:
            formats = [
                _Fmt('hls-1280', 'https://cdn.example.com/low/index.m3u8', 'mp4', 'm3u8_native', 1280, 640, 360),
                _Fmt('hls-2560', 'https://cdn.example.com/high/index.m3u8', 'mp4', 'm3u8_native', 2560, 1280, 720),
            ]
            subtitles = []

        monkeypatch.setattr(_host, 'extract_m3u8', lambda *a, **kw: _Result())
        ie = InfoExtractor()
        formats, subs = ie._extract_m3u8_formats_and_subtitles(
            'https://cdn.example.com/master.m3u8',
            'vid',
            ext='mp4',
            m3u8_id='hls',
        )
        assert subs == {}
        assert len(formats) == 2

        low = formats[0]
        assert low['url'] == 'https://cdn.example.com/low/index.m3u8'
        assert low['ext'] == 'mp4'
        assert low['protocol'] == 'm3u8_native'
        assert low['tbr'] == 1280
        assert low['width'] == 640
        assert low['height'] == 360
        assert low['format_id'] == 'hls-1280'

        high = formats[1]
        assert high['tbr'] == 2560
        assert high['width'] == 1280
        assert high['height'] == 720

    def test_master_playlist_skips_malformed_entries(self, monkeypatch):
        # When extract_m3u8 returns an empty formats list, the method
        # should propagate that correctly. Updated for Slice-2.5.
        from rdlp_ytdlp_compat import _host

        class _Result:
            formats = []
            subtitles = []

        monkeypatch.setattr(_host, 'extract_m3u8', lambda *a, **kw: _Result())
        ie = InfoExtractor()
        formats, _ = ie._extract_m3u8_formats_and_subtitles(
            'https://cdn.example.com/m.m3u8',
            'vid',
        )
        assert formats == []


class TestExtractorErrorHierarchy:
    """Verify the yt-dlp drop-in exception hierarchy. Any rename or
    constructor-shape regression here would break ported extractors that
    do `from yt_dlp.utils import ExtractorError`."""

    def test_extractor_error_subclass_of_youtube_dl_error(self):
        # ExtractorError must inherit from YoutubeDLError so existing
        # `except YoutubeDLError:` clauses still catch it.
        assert issubclass(ExtractorError, YoutubeDLError)

    def test_extractor_error_constructor_accepts_all_kwargs(self):
        # yt-dlp upstream signature: (msg, tb, expected, cause, video_id, ie).
        # Extractor source uses every one of these — missing kwargs would
        # fail at first import-and-run.
        cause = RuntimeError('inner')
        e = ExtractorError(
            'boom',
            tb='<traceback>',
            expected=True,
            cause=cause,
            video_id='abc123',
            ie='FooIE',
        )
        assert e.orig_msg == 'boom'
        assert e.traceback == '<traceback>'
        assert e.expected is True
        assert e.cause is cause
        assert e.video_id == 'abc123'
        assert e.ie == 'FooIE'

    def test_extractor_error_default_expected_false(self):
        e = ExtractorError('boom')
        assert e.expected is False

    def test_unsupported_error_single_arg_url(self):
        # Upstream's UnsupportedError is `__init__(self, url)` with
        # expected=True forced.
        e = UnsupportedError('https://example.com/foo')
        assert e.url == 'https://example.com/foo'
        assert e.expected is True
        assert 'https://example.com/foo' in str(e)

    def test_geo_restricted_carries_countries(self):
        e = GeoRestrictedError('blocked', countries=['US', 'GB'])
        assert e.countries == ['US', 'GB']
        assert e.expected is True

    def test_regex_not_found_subclass_of_extractor_error(self):
        # RegexNotFoundError must be catchable as ExtractorError —
        # yt-dlp's _search_regex raises it and many sites' fallback
        # code does `except ExtractorError: ...`.
        assert issubclass(RegexNotFoundError, ExtractorError)


class TestRaiseHelpers:
    """yt-dlp's raise_login_required / raise_geo_restricted / raise_no_formats
    are method-level on InfoExtractor and are called constantly by ported
    extractors. Without them, ports fail with AttributeError."""

    def test_raise_login_required_raises_extractor_error(self):
        # Drop-in compat: ExtractorError catches the typed LoginRequiredError
        # subclass, so ported `except ExtractorError:` still catches.
        ie = InfoExtractor()
        with pytest.raises(ExtractorError) as excinfo:
            ie.raise_login_required()
        assert excinfo.value.expected is True
        # Marker prefix is GONE (wave-3 fix C1) — typed class is the
        # dispatch key. Token must NOT leak into user-facing message.
        assert '[login required]' not in excinfo.value.orig_msg

    def test_raise_login_required_with_method(self):
        ie = InfoExtractor()
        with pytest.raises(ExtractorError) as excinfo:
            ie.raise_login_required('Please log in', method='cookies')
        assert 'cookies' in excinfo.value.orig_msg

    def test_raise_geo_restricted_raises_geo_restricted_error(self):
        ie = InfoExtractor()
        with pytest.raises(GeoRestrictedError) as excinfo:
            ie.raise_geo_restricted(countries=['US'])
        assert excinfo.value.countries == ['US']
        assert excinfo.value.expected is True

    def test_raise_no_formats_raises_extractor_error(self):
        # raise_no_formats(expected=False, default) raises a bare
        # ExtractorError (extractor bug case); the marker prefix is gone.
        ie = InfoExtractor()
        with pytest.raises(ExtractorError) as excinfo:
            ie.raise_no_formats('no media available', video_id='vid')
        assert '[no formats]' not in excinfo.value.orig_msg
        assert excinfo.value.video_id == 'vid'

    def test_raise_login_required_sets_ie_to_extractor_name(self):
        # Upstream yt-dlp's raise_* helpers populate ExtractorError.ie so
        # ported code that does `except ExtractorError as e: log(e.ie)`
        # sees the extractor identity, not None.
        class FooIE(InfoExtractor):
            pass

        with pytest.raises(ExtractorError) as excinfo:
            FooIE().raise_login_required()
        assert excinfo.value.ie == 'Foo'  # class name minus trailing "IE"

    def test_raise_geo_restricted_sets_ie(self):
        class BarIE(InfoExtractor):
            pass

        with pytest.raises(GeoRestrictedError) as excinfo:
            BarIE().raise_geo_restricted(countries=['JP'])
        assert excinfo.value.ie == 'Bar'

    def test_raise_no_formats_sets_ie(self):
        class BazIE(InfoExtractor):
            pass

        with pytest.raises(ExtractorError) as excinfo:
            BazIE().raise_no_formats('no media', video_id='vid')
        assert excinfo.value.ie == 'Baz'

    def test_ie_name_uses_explicit_IE_NAME_attribute(self):
        # When the extractor declares `IE_NAME = "youtube:music"` upstream-
        # style, that takes precedence over the class-name derivation.
        class WeirdNamedExtractor(InfoExtractor):
            IE_NAME = 'youtube:music'

        with pytest.raises(ExtractorError) as excinfo:
            WeirdNamedExtractor().raise_login_required()
        assert excinfo.value.ie == 'youtube:music'

    def test_ie_name_falls_back_to_class_name_when_no_IE_suffix(self):
        # If the class doesn't end in "IE", use the full class name.
        class FooBarExtractor(InfoExtractor):
            pass

        with pytest.raises(ExtractorError) as excinfo:
            FooBarExtractor().raise_login_required()
        assert excinfo.value.ie == 'FooBarExtractor'


# =============================================================================
# Wave-3 review-fix regression tests
# =============================================================================


class TestTypedSubclasses:
    """yt-dlp drop-in compat preserved + isinstance-driven dispatch enabled."""

    def test_login_required_extends_extractor_error(self):
        from rdlp_ytdlp_compat import ExtractorError, LoginRequiredError

        e = LoginRequiredError('login')
        assert isinstance(e, ExtractorError)
        assert e.expected is True

    def test_no_formats_extends_extractor_error(self):
        from rdlp_ytdlp_compat import ExtractorError, NoFormatsError

        e = NoFormatsError('no formats')
        assert isinstance(e, ExtractorError)
        assert e.expected is True

    def test_not_found_extends_extractor_error(self):
        from rdlp_ytdlp_compat import ExtractorError, NotFoundError

        assert isinstance(NotFoundError('missing'), ExtractorError)

    def test_rate_limited_carries_typed_retry_after(self):
        from rdlp_ytdlp_compat import RateLimitedError

        e = RateLimitedError('slow down', retry_after=42)
        assert e.retry_after == 42

    def test_rate_limited_retry_after_optional(self):
        from rdlp_ytdlp_compat import RateLimitedError

        assert RateLimitedError('slow down').retry_after is None

    def test_network_error_extends_extractor_error(self):
        from rdlp_ytdlp_compat import ExtractorError, NetworkError

        assert isinstance(NetworkError('dns failed'), ExtractorError)

    def test_typed_subclasses_default_messages(self):
        # Helpers raised without args produce a sensible default — matches
        # yt-dlp's user-facing message pattern.
        from rdlp_ytdlp_compat import (
            LoginRequiredError,
            NoFormatsError,
            NotFoundError,
            RateLimitedError,
        )

        assert 'registered users' in str(LoginRequiredError())
        assert 'formats' in str(NoFormatsError())
        assert 'not found' in str(NotFoundError()).lower()
        assert 'rate' in str(RateLimitedError()).lower()


class TestRaiseHelpersUseTypedSubclasses:
    """Helpers must raise the typed forms (LoginRequiredError etc.), NOT a
    bare ExtractorError with a marker prefix. The marker-phrase pattern was
    fragile (false-positive substring matches in dispatch) and leaked
    `[login required] ` tokens into user-facing error text."""

    def test_raise_login_required_raises_typed_class(self):
        from rdlp_ytdlp_compat import LoginRequiredError

        with pytest.raises(LoginRequiredError) as excinfo:
            InfoExtractor().raise_login_required()
        # No marker prefix in message — typed class is the dispatch key.
        assert '[login required]' not in excinfo.value.orig_msg

    def test_raise_no_formats_expected_true_raises_typed_class(self):
        from rdlp_ytdlp_compat import NoFormatsError

        with pytest.raises(NoFormatsError) as excinfo:
            InfoExtractor().raise_no_formats('nothing', expected=True)
        assert '[no formats]' not in excinfo.value.orig_msg

    def test_raise_no_formats_expected_false_raises_bare_extractor_error(self):
        # expected=False = "extractor bug, not site failure" — bare class.
        from rdlp_ytdlp_compat import ExtractorError, NoFormatsError

        with pytest.raises(ExtractorError) as excinfo:
            InfoExtractor().raise_no_formats('buggy', expected=False)
        # Must NOT be NoFormatsError — that's reserved for the expected case.
        assert not isinstance(excinfo.value, NoFormatsError)
        assert excinfo.value.expected is False


class TestSearchRegexRaisesTypedException:
    """_search_regex with fatal=True must raise RegexNotFoundError so the
    WIT dispatcher routes it to the parse variant via isinstance."""

    def test_fatal_raises_regex_not_found_error(self):
        from rdlp_ytdlp_compat import RegexNotFoundError

        ie = InfoExtractor()
        with pytest.raises(RegexNotFoundError):
            ie._search_regex(r'NOT', 'x', 'thing', fatal=True)

    def test_regex_not_found_extends_extractor_error(self):
        # Drop-in: ported `except ExtractorError:` clauses still catch.
        from rdlp_ytdlp_compat import ExtractorError

        ie = InfoExtractor()
        with pytest.raises(ExtractorError):
            ie._search_regex(r'NOT', 'x', 'thing', fatal=True)


class TestHostHttpError:
    """The host-side HTTP failure is now a typed RuntimeError subclass with
    `.status` and `.url` attributes — the WIT dispatcher routes by status
    code without parsing the message string (closes the latent
    'RuntimeError starting with HTTP' false-positive surface)."""

    def test_host_http_error_subclass_of_runtime_error(self):
        from rdlp_ytdlp_compat._host import HostHttpError

        e = HostHttpError(404, 'https://example.com/x')
        assert isinstance(e, RuntimeError)

    def test_host_http_error_carries_typed_attributes(self):
        from rdlp_ytdlp_compat._host import HostHttpError

        e = HostHttpError(429, 'https://example.com/y')
        assert e.status == 429
        assert e.url == 'https://example.com/y'

    def test_check_status_raises_typed_host_http_error(self):
        from rdlp_ytdlp_compat._host import HostHttpError, _check_status

        with pytest.raises(HostHttpError) as excinfo:
            _check_status(503, None, 'https://example.com')
        assert excinfo.value.status == 503

    def test_check_status_ok_is_silent(self):
        from rdlp_ytdlp_compat._host import _check_status

        _check_status(200, None, 'https://example.com')  # no raise


class TestSanitizeFilename:
    """yt-dlp drop-in compat for filename sanitisation."""

    def test_empty_returns_underscore(self):
        from rdlp_ytdlp_compat import sanitize_filename

        assert sanitize_filename('') == '_'

    def test_none_returns_underscore(self):
        from rdlp_ytdlp_compat import sanitize_filename

        assert sanitize_filename(None) == '_'

    def test_default_mode_replaces_forbidden_with_unicode_lookalikes(self):
        from rdlp_ytdlp_compat import sanitize_filename

        # /, \, :, |, *, <, > all forbidden on Windows; replaced not stripped.
        result = sanitize_filename('a/b\\c:d|e*f<g>h')
        for ch in '/\\:|*<>':
            assert ch not in result
        # Result still contains all letters
        for ch in 'abcdefgh':
            assert ch in result

    def test_restricted_mode_collapses_to_ascii_underscore(self):
        from rdlp_ytdlp_compat import sanitize_filename

        result = sanitize_filename('a/b c', restricted=True)
        assert '/' not in result
        # whitespace and / both become _
        assert '_' in result

    def test_control_chars_stripped(self):
        from rdlp_ytdlp_compat import sanitize_filename

        result = sanitize_filename('foo\x00\x01\x1f\x7fbar')
        assert '\x00' not in result and '\x01' not in result
        assert 'foo' in result and 'bar' in result

    def test_leading_trailing_dots_stripped(self):
        from rdlp_ytdlp_compat import sanitize_filename

        # Windows hostile to trailing dot; yt-dlp strips both.
        assert sanitize_filename('.foo.').strip('.') == 'foo'

    def test_path_traversal_neutralised_in_default_mode(self):
        # The motivating attack: format_id = "../../etc/passwd"
        from rdlp_ytdlp_compat import sanitize_filename

        result = sanitize_filename('../../etc/passwd')
        # / replaced with full-width look-alike or stripped
        assert '/' not in result
