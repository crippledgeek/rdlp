"""Issue #240 acceptance bullet: a Python plugin extracts DASH formats from
a fixture-shaped MpdExtraction payload via _extract_mpd_formats_and_subtitles.

The MpdExtraction payload below mirrors what the Rust host produces against
crates/rdlp-downloader/tests/fixtures/dash/segment_template.mpd. The Rust host
test in crates/rdlp-plugin/src/host/extract_helpers.rs is the authoritative
check that the Rust side actually produces this payload from the real MPD.
This test is the Python-plugin-level check that the shim consumes such a
payload correctly.
"""

from dataclasses import dataclass, field
from typing import Optional

from rdlp_ytdlp_compat.info_extractor import InfoExtractor
from rdlp_ytdlp_compat import _host


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


class FixtureExtractor(InfoExtractor):
    """Minimal Python plugin under test."""

    _VALID_URL = r'.*'

    def extract_dash(self, mpd_url, video_id):
        return self._extract_mpd_formats_and_subtitles(mpd_url, video_id)


def _segment_template_payload(manifest_url):
    """Mirrors expand_dash_representations(segment_template.mpd, manifest_url).

    Values approximate what the real Rust expansion produces — Rust test
    `extract_mpd_returns_formats_via_fixture` is the authoritative check that
    the real expansion shape matches what the Python conversion expects.
    """
    base = "https://cdn.example.com/v720/"
    return FakeMpdExtraction(formats=[
        FakeMpdFormat(
            format_id="dash_v_0_0",
            url=manifest_url,
            ext="mp4",
            vcodec="avc1.4d401f",
            acodec=None,
            tbr=1500.0,
            width=1280,
            height=720,
            fps=25.0,
            container="mp4_dash",
            manifest_url=manifest_url,
            fragment_base_url=base,
            fragments=[FakeMpdFragment(url="seg-0.m4s", duration=4.0)],
        ),
        FakeMpdFormat(
            format_id="dash_a_0_0",
            url=manifest_url,
            ext="m4a",
            vcodec=None,
            acodec="mp4a.40.2",
            tbr=128.0,
            asr=48000,
            container="m4a_dash",
            manifest_url=manifest_url,
            fragment_base_url=base,
            fragments=[FakeMpdFragment(url="seg-0.m4s", duration=4.0)],
        ),
    ])


def test_fixture_plugin_extracts_dash_formats(monkeypatch):
    manifest_url = "https://example.com/manifest.mpd"
    monkeypatch.setattr(
        _host, "extract_mpd",
        lambda *a, **k: _segment_template_payload(manifest_url),
    )

    ie = FixtureExtractor()
    fmts, subs = ie.extract_dash(manifest_url, "v123")

    # Structural invariants — issue #240 acceptance.
    assert len(fmts) >= 2, fmts
    video_fmts = [f for f in fmts if f.get("vcodec") and not f.get("acodec")]
    audio_fmts = [f for f in fmts if f.get("acodec") and not f.get("vcodec")]
    assert len(video_fmts) >= 1, fmts
    assert len(audio_fmts) >= 1, fmts

    for f in fmts:
        assert f["protocol"] == "http_dash_segments"
        assert f["fragments"], f
        assert f["fragment_base_url"]
        assert f["manifest_url"] == manifest_url
        assert f["container"] in ("mp4_dash", "m4a_dash")
    assert subs == {}
