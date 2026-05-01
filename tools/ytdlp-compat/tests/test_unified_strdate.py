"""yt-dlp's `unified_strdate` — date string parser → 'YYYYMMDD'."""

from rdlp_ytdlp_compat import unified_strdate


def test_none_returns_none():
    assert unified_strdate(None) is None


def test_iso_8601():
    assert unified_strdate('2024-01-15') == '20240115'


def test_iso_8601_with_time():
    assert unified_strdate('2024-01-15T10:30:00Z') == '20240115'


def test_unparseable_returns_none():
    assert unified_strdate('not a date') is None


def test_already_yyyymmdd():
    assert unified_strdate('20240115') == '20240115'
