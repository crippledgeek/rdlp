"""Cross-check our `dispatch_url` against EVERY upstream SVT `_TEST` URL.

Loads `examples/plugins/svt/svt.py` directly (using `importlib`, no
componentize-py needed) and asserts that for each of the 23 URLs from
upstream svt.py's `_TESTS` blocks, our `dispatch_url` picks the
correct `*IE` class. Catches regressions in:

  - the `_VALID_URL` regex compile path (no `re.VERBOSE` corruption)
  - sibling-`suitable()` overrides (SVTSeriesIE / SVTPageIE yielding
    when SVTPlayIE.suitable matches)
  - the dispatch order independence claim in test_dispatch.py

Each entry is sourced verbatim from
`/tmp/ytdlp-slice2/yt_dlp/extractor/svt.py` `_TESTS` blocks at upstream
tag 2026.03.17. URLs marked `'only_matching': True` upstream still
need to dispatch correctly even if their info_dict isn't asserted.
"""
import importlib.util
import sys
from pathlib import Path

import pytest


def _load_svt_module():
    """Import examples/plugins/svt/svt.py as a Python module without
    going through componentize-py. The shim's `_dispatch.py` works
    against any module exposing the IE classes."""
    repo_root = Path(__file__).resolve().parents[3]
    svt_path = repo_root / "examples/plugins/svt/svt.py"
    assert svt_path.exists(), f"missing source: {svt_path}"
    spec = importlib.util.spec_from_file_location("svt_under_test", svt_path)
    module = importlib.util.module_from_spec(spec)
    sys.modules["svt_under_test"] = module
    spec.loader.exec_module(module)
    return module


SVT = _load_svt_module()
from rdlp_ytdlp_compat._dispatch import (  # noqa: E402
    discover_ie_classes,
    dispatch_url,
)


CANDIDATES = discover_ie_classes(SVT)


# Each entry: (URL, expected IE class name). Order-independent —
# `dispatch_url` walks every candidate's `suitable()`. Comments mark
# the upstream test type (full info_dict assertion vs. only_matching).
SVT_PLAY_URLS = [
    # Full info_dict tests
    ("https://www.svtplay.se/video/eXYgwZb/sverige-och-kriget/1-utbrottet", "SVTPlayIE"),
    ("https://www.svtplay.se/video/30479064", "SVTPlayIE"),
    ("https://www.svtplay.se/video/emBxBQj", "SVTPlayIE"),
    ("https://www.svtplay.se/video/jz2rYz7/anders-hansen-moter/james-fallon?info=visa", "SVTPlayIE"),
    # only_matching tests
    ("https://www.svtplay.se/video/30479064/husdrommar/husdrommar-sasong-8-designdrommar-i-stenungsund?modalId=8zVbDPA", "SVTPlayIE"),
    ("https://www.svtplay.se/video/30684086/rapport/rapport-24-apr-18-00-7?id=e72gVpa", "SVTPlayIE"),
    ("http://www.oppetarkiv.se/video/5219710/trollflojten", "SVTPlayIE"),
    ("http://www.svtplay.se/klipp/9023742/stopptid-om-bjorn-borg", "SVTPlayIE"),
    ("https://www.svtplay.se/kanaler/svt1", "SVTPlayIE"),
    ("svt:1376446-003A", "SVTPlayIE"),
    ("svt:14278044", "SVTPlayIE"),
    ("https://www.svt.se/barnkanalen/barnplay/kar/eWv5MLX/", "SVTPlayIE"),
    ("svt:eWv5MLX", "SVTPlayIE"),
]

SVT_SERIES_URLS = [
    ("https://www.svtplay.se/rederiet", "SVTSeriesIE"),
    ("https://www.svtplay.se/rederiet?tab=season-2-jpmQYgn", "SVTSeriesIE"),
]

SVT_PAGE_URLS = [
    ("https://www.svt.se/nyheter/lokalt/skane/viktor-18-forlorade-armar-och-ben-i-sepsis-vill-ateruppta-karaten-och-bli-svetsare", "SVTPageIE"),
    ("https://www.svt.se/nyheter/lokalt/skane/forsvarsmakten-om-trafikkaoset-pa-e22-kunde-inte-varit-dar-snabbare", "SVTPageIE"),
    ("https://www.svt.se/nyheter/svtforum/2023-tungt-ar-for-svensk-media", "SVTPageIE"),
    ("https://www.svt.se/sport/ishockey/bakom-masken-lehners-kamp-mot-mental-ohalsa", "SVTPageIE"),
    ("https://www.svt.se/nyheter/utrikes/svenska-andrea-ar-en-mil-fran-branderna-i-kalifornien", "SVTPageIE"),
    ("http://www.svt.se/sport/ishockey/jagr-tacklar-giroux-under-intervjun", "SVTPageIE"),
    ("https://www.svt.se/nyheter/lokalt/vast/svt-testar-tar-nagon-upp-skrapet-1", "SVTPageIE"),
    ("https://www.svt.se/vader/manadskronikor/maj2018", "SVTPageIE"),
]

ALL_URLS = SVT_PLAY_URLS + SVT_SERIES_URLS + SVT_PAGE_URLS


class TestSvtClassDiscovery:
    def test_three_concrete_classes(self):
        names = sorted(c.__name__ for c in CANDIDATES)
        assert names == ["SVTPageIE", "SVTPlayIE", "SVTSeriesIE"], (
            f"expected SVT's 3 concrete IEs, got {names}"
        )

    def test_svt_base_excluded(self):
        # `SVTBaseIE` ships in svt.py but lacks `_VALID_URL` (`None`
        # inherited from InfoExtractor) — must NOT appear as a
        # dispatch candidate.
        for cls in CANDIDATES:
            assert cls.__name__ != "SVTBaseIE"


class TestSvtUrlDispatch:
    @pytest.mark.parametrize("url,expected", ALL_URLS,
                             ids=[u for u, _ in ALL_URLS])
    def test_dispatches_to_correct_ie(self, url, expected):
        cls = dispatch_url(CANDIDATES, url)
        assert cls is not None, (
            f"no IE claimed {url!r} — every upstream _TEST URL must dispatch"
        )
        assert cls.__name__ == expected, (
            f"{url!r}: dispatched to {cls.__name__}, expected {expected}"
        )

    def test_unrelated_url_returns_none(self):
        # Sanity: a URL outside SVT's domain space must not match any IE.
        assert dispatch_url(CANDIDATES, "https://example.com/foo") is None

    def test_play_takes_precedence_over_series(self):
        # SVTSeriesIE.suitable explicitly yields when SVTPlayIE.suitable
        # matches. Pin this contract directly with a URL that BOTH
        # regexes match shape-wise (a /video/ path) — SVTPlayIE's regex
        # has the more specific `/video/...` requirement, but
        # SVTSeriesIE's broader regex `/(?P<id>[^/?&#]+)` would also
        # capture `video` as the id. Without the override, dispatch
        # ordering would matter.
        url = "https://www.svtplay.se/video/eXYgwZb/sverige-och-kriget/1-utbrottet"
        cls = dispatch_url(CANDIDATES, url)
        assert cls.__name__ == "SVTPlayIE"

    def test_play_takes_precedence_over_page(self):
        # SVTPageIE.suitable yields when SVTPlayIE matches. Pin similarly.
        url = "https://www.svt.se/barnkanalen/barnplay/kar/eWv5MLX/"
        cls = dispatch_url(CANDIDATES, url)
        assert cls.__name__ == "SVTPlayIE"
