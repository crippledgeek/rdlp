"""Verify Python InfoExtractor methods correctly delegate to _host."""

import pytest

from rdlp_ytdlp_compat import InfoExtractor


@pytest.fixture
def ie():
    class _Concrete(InfoExtractor):
        _VALID_URL = r'.*'

    return _Concrete()


def test_search_regex_calls_host(monkeypatch, ie):
    captured = {}

    def fake(pat, s, flags):
        captured['pat'] = pat
        captured['s'] = s
        captured['flags'] = flags
        return 'matched'

    from rdlp_ytdlp_compat import _host

    monkeypatch.setattr(_host, 'search_regex', fake)

    result = ie._search_regex(r'foo', 'string foo bar', 'name')

    assert result == 'matched'
    assert captured['pat'] == r'foo'
    assert captured['s'] == 'string foo bar'


def test_search_regex_returns_default_on_miss(monkeypatch, ie):
    from rdlp_ytdlp_compat import _host

    monkeypatch.setattr(_host, 'search_regex', lambda *a, **k: None)
    assert ie._search_regex(r'x', 'y', 'name', default='fallback', fatal=False) == 'fallback'


def test_search_regex_raises_on_miss_fatal(monkeypatch, ie):
    from rdlp_ytdlp_compat import RegexNotFoundError, _host

    monkeypatch.setattr(_host, 'search_regex', lambda *a, **k: None)
    with pytest.raises(RegexNotFoundError):
        ie._search_regex(r'x', 'y', 'name')


def test_html_search_regex_calls_host(monkeypatch, ie):
    from rdlp_ytdlp_compat import _host

    monkeypatch.setattr(_host, 'html_search_regex', lambda pat, s, f: 'Hello World')
    result = ie._html_search_regex(r'<title>(.+?)</title>', '<title>Hello <b>World</b></title>', 'title')
    assert result == 'Hello World'


def test_html_search_meta_calls_host(monkeypatch, ie):
    from rdlp_ytdlp_compat import _host

    monkeypatch.setattr(_host, 'html_search_meta', lambda name, html: 'value')
    result = ie._html_search_meta('og:title', '<html/>')
    assert result == 'value'


def test_og_search_property_calls_host(monkeypatch, ie):
    from rdlp_ytdlp_compat import _host

    monkeypatch.setattr(_host, 'og_search_property', lambda prop, html: 'Title')
    assert ie._og_search_title('<html/>') == 'Title'
    assert ie._og_search_thumbnail('<html/>') == 'Title'


def test_rta_search_calls_host(monkeypatch):
    from rdlp_ytdlp_compat import InfoExtractor, _host

    monkeypatch.setattr(_host, 'rta_search', lambda html: 18)
    assert InfoExtractor._rta_search('anything') == 18


def test_rta_search_none(monkeypatch):
    from rdlp_ytdlp_compat import InfoExtractor, _host

    monkeypatch.setattr(_host, 'rta_search', lambda html: None)
    assert InfoExtractor._rta_search('clean') is None


def test_search_json_calls_host(monkeypatch, ie):
    from rdlp_ytdlp_compat import _host

    monkeypatch.setattr(_host, 'search_json', lambda start, end, s: '{"x": 1}')
    result = ie._search_json(r'start', 'page', 'name', 'vid')
    assert result == {'x': 1}


def test_set_cookie_calls_host_cookie_jar(monkeypatch, ie):
    captured = {}

    def fake_set_cookie(url, cookie):
        captured['url'] = url
        captured['cookie'] = cookie

    from rdlp_ytdlp_compat import _host

    fake_jar = type('J', (), {'set_cookie': staticmethod(fake_set_cookie)})()
    monkeypatch.setattr(_host, 'cookie_jar', fake_jar, raising=False)

    ie._set_cookie('example.com', 'sessionid', 'abc123')

    assert captured['url'] == 'https://example.com/'
    assert captured['cookie'].name == 'sessionid'
    assert captured['cookie'].value == 'abc123'


def test_search_json_ld_calls_host(monkeypatch, ie):
    from rdlp_ytdlp_compat import _host

    class FakeJsonLd:
        title = 'T'
        description = 'd'
        thumbnail = 'https://x/t.jpg'
        thumbnails = []
        upload_date = '20240101'
        duration = 300
        view_count = 100
        like_count = 5
        tags = []
        categories = []

    monkeypatch.setattr(_host, 'extract_json_ld', lambda html: FakeJsonLd())
    result = ie._search_json_ld('<html/>', 'vid')
    assert result['title'] == 'T'
    assert result['duration'] == 300


def test_extract_m3u8_calls_host(monkeypatch, ie):
    from rdlp_ytdlp_compat import _host

    class FakeFormat:
        format_id = '0'
        url = 'https://x/hi.m3u8'
        ext = 'mp4'
        protocol = 'm3u8_native'
        tbr = 5000.0
        width = 1920
        height = 1080
        fps = None
        vcodec = 'avc1'
        acodec = 'mp4a'
        vbr = None
        abr = None
        language = None
        format_note = None
        format_index = 0
        manifest_url = None
        has_drm = None
        preference = None
        quality = None

    class FakeResult:
        formats = [FakeFormat()]
        subtitles = []

    monkeypatch.setattr(_host, 'extract_m3u8', lambda *a, **k: FakeResult())
    fmts, subs = ie._extract_m3u8_formats_and_subtitles('https://x.com/m.m3u8', 'v')
    assert len(fmts) == 1
    assert fmts[0]['url'] == 'https://x/hi.m3u8'
    assert fmts[0]['width'] == 1920
    assert subs == {}
