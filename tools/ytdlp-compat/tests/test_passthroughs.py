"""Verify Python InfoExtractor methods correctly delegate to _host."""

import pytest
from dataclasses import dataclass, field
from typing import Optional

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


# ---- DASH (extract_mpd) fakes & tests --------------------------------


@dataclass
class FakeMpdFragment:
    url: str
    duration: Optional[float] = None


@dataclass
class FakeMpdFormat:
    format_id: str = "dash_v_0_0"
    url: str = "https://example.com/manifest.mpd"
    ext: str = "mp4"
    vcodec: Optional[str] = "avc1.4d401f"
    acodec: Optional[str] = None
    tbr: Optional[float] = 1500.0
    width: Optional[int] = 1280
    height: Optional[int] = 720
    fps: Optional[float] = 25.0
    asr: Optional[int] = None
    language: Optional[str] = None
    container: Optional[str] = "mp4_dash"
    manifest_url: Optional[str] = "https://example.com/manifest.mpd"
    fragment_base_url: Optional[str] = "https://cdn.example.com/v720/"
    fragments: list = field(default_factory=list)


@dataclass
class FakeMpdExtraction:
    formats: list = field(default_factory=list)
    subtitles: list = field(default_factory=list)


def _ie():
    """Construct a bare InfoExtractor for shim-method exercise."""
    from rdlp_ytdlp_compat.info_extractor import InfoExtractor

    class _Concrete(InfoExtractor):
        _VALID_URL = r'.*'

    return _Concrete()


def test_extract_mpd_calls_host(monkeypatch):
    """#8 (positive) — shim invokes _host.extract_mpd and returns converted dicts."""
    from rdlp_ytdlp_compat import _host
    fake = FakeMpdExtraction(formats=[
        FakeMpdFormat(fragments=[FakeMpdFragment(url="seg-0.m4s", duration=4.0)]),
    ])
    monkeypatch.setattr(_host, "extract_mpd", lambda *a, **k: fake)
    fmts, subs = _ie()._extract_mpd_formats_and_subtitles(
        "https://example.com/manifest.mpd", "v123",
    )
    assert len(fmts) == 1
    assert fmts[0]["format_id"] == "dash_v_0_0"
    assert fmts[0]["url"] == "https://example.com/manifest.mpd"
    assert fmts[0]["manifest_url"] == "https://example.com/manifest.mpd"


def test_extract_mpd_relative_fragment_uses_path_key(monkeypatch):
    """#9 — relative fragment URLs use 'path', per yt-dlp location_key."""
    from rdlp_ytdlp_compat import _host
    fake = FakeMpdExtraction(formats=[
        FakeMpdFormat(fragments=[FakeMpdFragment(url="seg-0.m4s", duration=4.0)]),
    ])
    monkeypatch.setattr(_host, "extract_mpd", lambda *a, **k: fake)
    fmts, _ = _ie()._extract_mpd_formats_and_subtitles("https://x/m.mpd", "v")
    assert fmts[0]["fragments"][0] == {"path": "seg-0.m4s", "duration": 4.0}
    assert "url" not in fmts[0]["fragments"][0]


def test_extract_mpd_absolute_fragment_uses_url_key(monkeypatch):
    """#10 — absolute fragment URLs use 'url'."""
    from rdlp_ytdlp_compat import _host
    abs_url = "https://cdn.example.com/seg-0.m4s"
    fake = FakeMpdExtraction(formats=[
        FakeMpdFormat(fragments=[FakeMpdFragment(url=abs_url, duration=4.0)]),
    ])
    monkeypatch.setattr(_host, "extract_mpd", lambda *a, **k: fake)
    fmts, _ = _ie()._extract_mpd_formats_and_subtitles("https://x/m.mpd", "v")
    assert fmts[0]["fragments"][0] == {"url": abs_url, "duration": 4.0}
    assert "path" not in fmts[0]["fragments"][0]


def test_extract_mpd_protocol_is_http_dash_segments(monkeypatch):
    """#11 — every emitted format dict carries protocol='http_dash_segments'."""
    from rdlp_ytdlp_compat import _host
    fake = FakeMpdExtraction(formats=[
        FakeMpdFormat(fragments=[FakeMpdFragment(url="a.m4s")]),
        FakeMpdFormat(format_id="dash_a_0_0", vcodec=None, acodec="mp4a.40.2",
                      fragments=[FakeMpdFragment(url="b.m4s")]),
    ])
    monkeypatch.setattr(_host, "extract_mpd", lambda *a, **k: fake)
    fmts, _ = _ie()._extract_mpd_formats_and_subtitles("https://x/m.mpd", "v")
    assert len(fmts) == 2  # stub returns [] so this guards against vacuous all()
    assert all(f["protocol"] == "http_dash_segments" for f in fmts)


def test_extract_mpd_drops_unsupported_kwargs_data():
    """#12 — data= kwarg raises NotImplementedError."""
    with pytest.raises(NotImplementedError, match="data/headers/query"):
        _ie()._extract_mpd_formats_and_subtitles(
            "https://x/m.mpd", "v", data=b"x",
        )


def test_extract_mpd_drops_unsupported_kwargs_headers():
    """#13 — headers= kwarg raises NotImplementedError."""
    with pytest.raises(NotImplementedError, match="data/headers/query"):
        _ie()._extract_mpd_formats_and_subtitles(
            "https://x/m.mpd", "v", headers={"X": "y"},
        )


def test_extract_mpd_drops_unsupported_kwargs_query():
    """#14 — query= kwarg raises NotImplementedError."""
    with pytest.raises(NotImplementedError, match="data/headers/query"):
        _ie()._extract_mpd_formats_and_subtitles(
            "https://x/m.mpd", "v", query={"k": "v"},
        )


def test_extract_mpd_fatal_false_returns_empty_on_runtime_error(monkeypatch):
    """#15 — fatal=False swallows RuntimeError and returns ([], {}).

    The guard `called` ensures the host was actually invoked (not bypassed by
    the stub), so the test fails against the current no-op stub.
    """
    from rdlp_ytdlp_compat import _host

    called = []

    def boom(*a, **k):
        called.append(True)
        raise RuntimeError("boom")

    monkeypatch.setattr(_host, "extract_mpd", boom)
    fmts, subs = _ie()._extract_mpd_formats_and_subtitles(
        "https://x/m.mpd", "v", fatal=False,
    )
    assert called, "_host.extract_mpd was never invoked — stub bypassed the host"
    assert fmts == []
    assert subs == {}


def test_extract_mpd_fatal_true_propagates_runtime_error(monkeypatch):
    """#16 — fatal=True re-raises RuntimeError."""
    from rdlp_ytdlp_compat import _host

    def boom(*a, **k):
        raise RuntimeError("boom")

    monkeypatch.setattr(_host, "extract_mpd", boom)
    with pytest.raises(RuntimeError, match="boom"):
        _ie()._extract_mpd_formats_and_subtitles(
            "https://x/m.mpd", "v", fatal=True,
        )


def test_extract_mpd_formats_drops_subtitles(monkeypatch):
    """#17 — _extract_mpd_formats returns list, not tuple."""
    from rdlp_ytdlp_compat import _host
    fake = FakeMpdExtraction(formats=[
        FakeMpdFormat(fragments=[FakeMpdFragment(url="a.m4s")]),
    ])
    monkeypatch.setattr(_host, "extract_mpd", lambda *a, **k: fake)
    result = _ie()._extract_mpd_formats("https://x/m.mpd", "v")
    assert isinstance(result, list)
    assert len(result) == 1


def test_extract_mpd_fragment_duration_optional_passthrough(monkeypatch):
    """#18 — fragment.duration None is omitted; non-None is passed through."""
    from rdlp_ytdlp_compat import _host
    fake = FakeMpdExtraction(formats=[
        FakeMpdFormat(fragments=[
            FakeMpdFragment(url="a.m4s", duration=4.5),
            FakeMpdFragment(url="b.m4s", duration=None),
        ]),
    ])
    monkeypatch.setattr(_host, "extract_mpd", lambda *a, **k: fake)
    fmts, _ = _ie()._extract_mpd_formats_and_subtitles("https://x/m.mpd", "v")
    assert fmts[0]["fragments"][0]["duration"] == 4.5
    assert "duration" not in fmts[0]["fragments"][1]


def test_extract_mpd_subtitles_returned_as_empty_dict_in_v1(monkeypatch):
    """#19 — subtitles return slot is a dict (empty in v1).

    Also asserts formats came back non-empty so the stub (which never calls
    the host) fails this test — format conversion is a prerequisite.
    """
    from rdlp_ytdlp_compat import _host
    fake = FakeMpdExtraction(formats=[
        FakeMpdFormat(fragments=[FakeMpdFragment(url="a.m4s")]),
    ])
    monkeypatch.setattr(_host, "extract_mpd", lambda *a, **k: fake)
    fmts, subs = _ie()._extract_mpd_formats_and_subtitles("https://x/m.mpd", "v")
    assert len(fmts) == 1  # host must have been called and conversion must work
    assert isinstance(subs, dict)
    assert subs == {}
