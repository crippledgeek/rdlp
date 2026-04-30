"""Regression tests for bugs H7, H8, H9.

These tests were written FIRST and verified to FAIL against the unpatched code
before the fixes were applied:

  H7  _extract_m3u8_formats_and_subtitles drops data/headers/query silently
  H8  unified_timestamp only handled ISO 8601 + RFC 2822 (missed DATE_FORMATS)
  H9  urljoin hardcoded 'https:' for protocol-relative paths (ignored base scheme)
"""

import pytest

from rdlp_ytdlp_compat.info_extractor import (
    InfoExtractor,
    unified_timestamp,
    urljoin,
)

# =============================================================================
# H9 — urljoin protocol-relative scheme inheritance
# =============================================================================


class TestUrljoinH9ProtocolRelative:
    """H9: protocol-relative paths must inherit the scheme from `base`,
    not unconditionally prepend 'https:'."""

    def test_http_base_propagates_http_scheme(self):
        # NEGATIVE: before fix, returned "https://cdn/y"
        assert urljoin('http://x.com/', '//cdn/y') == 'http://cdn/y'

    def test_https_base_propagates_https_scheme(self):
        assert urljoin('https://x.com/', '//cdn/y') == 'https://cdn/y'

    def test_none_base_defaults_to_https(self):
        # When base is absent (None), fall back to https (safe default).
        assert urljoin(None, '//cdn/y') == 'https://cdn/y'

    def test_missing_base_string_defaults_to_https(self):
        # Empty-ish base that has no parseable scheme defaults to https.
        assert urljoin('', '//cdn/y') == 'https://cdn/y'

    def test_ftp_base_propagates_ftp_scheme(self):
        # Unusual but yt-dlp is scheme-agnostic in urljoin.
        # Our implementation reads the scheme from urllib.parse.urlparse.
        assert urljoin('ftp://files.example.com/', '//cdn/y') == 'ftp://cdn/y'

    def test_protocol_relative_with_path_under_http_base(self):
        assert urljoin('http://example.com/a/b', '//cdn.example.com/asset.m3u8') == 'http://cdn.example.com/asset.m3u8'

    # ---- Existing passing behaviours must still hold ----

    def test_absolute_https_path_unchanged(self):
        assert urljoin('http://x.com/', 'https://other.com/y') == 'https://other.com/y'

    def test_absolute_http_path_unchanged(self):
        assert urljoin('https://x.com/', 'http://other.com/y') == 'http://other.com/y'

    def test_relative_path_resolved_against_base(self):
        assert urljoin('https://example.com/a/b', '/c') == 'https://example.com/c'

    def test_none_path_returns_none(self):
        assert urljoin('https://example.com', None) is None

    def test_bytes_path_protocol_relative(self):
        result = urljoin('http://x.com/', b'//cdn/y')
        assert result == 'http://cdn/y'


# =============================================================================
# H8 — unified_timestamp DATE_FORMATS coverage
# =============================================================================


class TestUnifiedTimestampH8DateFormats:
    """H8: unified_timestamp must parse a broad set of realistic date strings,
    not just ISO 8601 and RFC 2822."""

    # ---- Formats that FAILED before the DATE_FORMATS list was added ----

    def test_yyyy_mm_dd_space_time(self):
        # "2024-01-15 12:30:45" — common in database exports / API responses.
        result = unified_timestamp('2024-01-15 12:30:45')
        assert result is not None
        assert isinstance(result, int)

    def test_dd_slash_mm_slash_yyyy_day_first(self):
        # "15/01/2024" is day-first by default.
        result = unified_timestamp('15/01/2024', day_first=True)
        assert result is not None
        assert isinstance(result, int)
        # Sanity: January 15, 2024 in UTC
        import datetime

        dt = datetime.datetime(2024, 1, 15, tzinfo=datetime.UTC)
        assert result == int(dt.timestamp())

    def test_dd_slash_mm_slash_yyyy_month_first(self):
        # With day_first=False, "01/15/2024" should parse as Jan 15.
        result = unified_timestamp('01/15/2024', day_first=False)
        assert result is not None
        import datetime

        dt = datetime.datetime(2024, 1, 15, tzinfo=datetime.UTC)
        assert result == int(dt.timestamp())

    def test_jan_15_2024_long_month(self):
        # "Jan 15, 2024"
        result = unified_timestamp('Jan 15, 2024')
        assert result is not None
        assert isinstance(result, int)

    def test_january_15_2024_full_month(self):
        # "January 15, 2024"
        result = unified_timestamp('January 15, 2024')
        assert result is not None
        assert isinstance(result, int)

    def test_15_jan_2024_day_abbrev(self):
        # "15 Jan 2024"
        result = unified_timestamp('15 Jan 2024')
        assert result is not None

    def test_15_january_2024_day_full_month(self):
        # "15 January 2024"
        result = unified_timestamp('15 January 2024')
        assert result is not None

    def test_yyyy_slash_mm_slash_dd(self):
        # "2024/01/15"
        result = unified_timestamp('2024/01/15')
        assert result is not None

    def test_yyyymmdd_compact(self):
        # "20240115" — used in some streaming APIs.
        result = unified_timestamp('20240115')
        assert result is not None

    def test_dd_dot_mm_dot_yyyy(self):
        # "15.01.2024" — common in European sites.
        result = unified_timestamp('15.01.2024')
        assert result is not None

    def test_iso_8601_z_still_works(self):
        # Existing passing format must remain unaffected.
        assert unified_timestamp('2026-04-29T12:00:00Z') == 1777464000

    def test_rfc_2822_still_works(self):
        assert unified_timestamp('Wed, 29 Apr 2026 12:00:00 GMT') == 1777464000

    def test_invalid_still_returns_none(self):
        assert unified_timestamp('not a date') is None

    def test_none_still_returns_none(self):
        assert unified_timestamp(None) is None

    def test_all_return_integers(self):
        """Spot-check 10 realistic date strings — all must return an int."""
        cases = [
            '2024-01-15 12:30:45',
            '15/01/2024',
            'Jan 15, 2024',
            'January 15, 2024',
            '15 Jan 2024',
            '2024/01/15',
            '20240115',
            '15.01.2024',
            '2024-01-15',
            '2024-01-15T12:30:45Z',
        ]
        for s in cases:
            result = unified_timestamp(s)
            assert result is not None and isinstance(result, int), (
                f'unified_timestamp({s!r}) returned {result!r}, expected int'
            )

    def test_day_first_ambiguous_date_parsed_correctly(self):
        # "02/03/2024": day_first=True → 2 March 2024; day_first=False → 3 Feb 2024.
        import datetime

        ts_day_first = unified_timestamp('02/03/2024', day_first=True)
        ts_month_first = unified_timestamp('02/03/2024', day_first=False)
        march_2 = int(datetime.datetime(2024, 3, 2, tzinfo=datetime.UTC).timestamp())
        feb_3 = int(datetime.datetime(2024, 2, 3, tzinfo=datetime.UTC).timestamp())
        assert ts_day_first == march_2, f'day_first=True: expected March 2 ({march_2}), got {ts_day_first}'
        assert ts_month_first == feb_3, f'day_first=False: expected Feb 3 ({feb_3}), got {ts_month_first}'


# =============================================================================
# H7 — _extract_m3u8_formats_and_subtitles fails loud on unsupported params
# =============================================================================


class TestExtractM3u8H7UnsupportedParams:
    """H7: passing data/headers/query must raise NotImplementedError (loud
    failure) rather than silently fetching unauthenticated."""

    def test_headers_raises_not_implemented(self):
        # NEGATIVE: before fix, headers= was silently ignored and the host
        # was called without authentication.
        ie = InfoExtractor()
        with pytest.raises(NotImplementedError, match='headers'):
            ie._extract_m3u8_formats_and_subtitles(
                'https://example.com/master.m3u8',
                'vid',
                headers={'Authorization': 'Bearer token123'},
            )

    def test_data_raises_not_implemented(self):
        ie = InfoExtractor()
        with pytest.raises(NotImplementedError, match='data'):
            ie._extract_m3u8_formats_and_subtitles(
                'https://example.com/master.m3u8',
                'vid',
                data=b'body=payload',
            )

    def test_query_raises_not_implemented(self):
        ie = InfoExtractor()
        with pytest.raises(NotImplementedError, match='query'):
            ie._extract_m3u8_formats_and_subtitles(
                'https://example.com/master.m3u8',
                'vid',
                query={'token': 'abc'},
            )

    def test_live_logs_warning_proceeds_not_raises(self, caplog):
        """live=True should log a warning but NOT raise — it's an acceptable
        degradation (proceed as non-live). The actual fetch will fail with
        RuntimeError outside the componentize-py runtime, exercising the
        fatal=False path."""
        import logging

        ie = InfoExtractor()
        with caplog.at_level(logging.WARNING, logger='rdlp_ytdlp_compat'):
            # fatal=False so the RuntimeError from outside-runtime is swallowed.
            formats, subs = ie._extract_m3u8_formats_and_subtitles(
                'https://example.com/master.m3u8',
                'vid',
                live=True,
                fatal=False,
            )
        assert formats == []
        assert subs == {}
        assert any('live' in r.message.lower() for r in caplog.records), (
            'Expected a warning about live=True, got: ' + str([r.message for r in caplog.records])
        )

    def test_no_extra_params_still_calls_host(self):
        """Baseline: calling without data/headers/query must NOT raise
        NotImplementedError — it proceeds to the host call (which raises
        RuntimeError outside the runtime)."""
        ie = InfoExtractor()
        with pytest.raises(RuntimeError):
            # fatal=True (default) so RuntimeError from no-runtime propagates.
            ie._extract_m3u8_formats_and_subtitles(
                'https://example.com/master.m3u8',
                'vid',
            )

    def test_headers_error_message_mentions_wit(self):
        """Error message must reference WIT/Slice 2.5 so plugin authors know
        what to do."""
        ie = InfoExtractor()
        with pytest.raises(NotImplementedError) as exc_info:
            ie._extract_m3u8_formats_and_subtitles(
                'https://example.com/master.m3u8',
                'vid',
                headers={'X-Token': 'y'},
            )
        msg = str(exc_info.value)
        assert 'WIT' in msg or 'Slice 2.5' in msg or 'v0.1.0' in msg

    def test_headers_raised_before_host_call(self, monkeypatch):
        """The NotImplementedError must be raised before ANY host call —
        not after a potentially-unauthenticated fetch attempt."""
        from rdlp_ytdlp_compat import _host

        called = []
        monkeypatch.setattr(_host, 'extract_m3u8', lambda *a, **kw: called.append(1))

        ie = InfoExtractor()
        with pytest.raises(NotImplementedError):
            ie._extract_m3u8_formats_and_subtitles(
                'https://example.com/master.m3u8',
                'vid',
                headers={'Authorization': 'Bearer token123'},
            )
        assert called == [], 'Host must NOT be called when headers are passed'
