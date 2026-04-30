"""Multi-class plugin dispatch.

A yt-dlp source file like `svt.py` can ship a base IE plus multiple
concrete subclasses (`SVTPlayIE`, `SVTSeriesIE`, `SVTPageIE`). At
extract time the plugin host needs to pick the right one for a given
URL. Dispatch rules mirror yt-dlp's `gen_extractors` + per-class
`suitable()` walk — the first class whose `suitable(url)` returns True
wins. Sibling-overriding (one class deferring to another via
`super().suitable()`) is part of the public contract used by SVT.
"""
import pytest

from rdlp_ytdlp_compat import InfoExtractor
from rdlp_ytdlp_compat._dispatch import (
    DispatchError,
    discover_ie_classes,
    dispatch_url,
)

# -----------------------------------------------------------------------------
# Module fixtures — simulate user_plugin modules with various class shapes.
# -----------------------------------------------------------------------------

class _FakeModule:
    """Stand-in for `user_plugin` — a namespace object exposing classes
    via attribute access."""
    def __init__(self, **classes):
        for name, cls in classes.items():
            setattr(self, name, cls)


class _AlphaIE(InfoExtractor):
    _VALID_URL = r"https?://alpha\.example/(?P<id>\w+)"


class _BetaIE(InfoExtractor):
    _VALID_URL = r"https?://beta\.example/(?P<id>\w+)"


class _BaseIE(InfoExtractor):
    """Abstract base — no `_VALID_URL`, must be excluded from dispatch
    candidates."""


class _LeafIE(_BaseIE):
    _VALID_URL = r"https?://leaf\.example/(?P<id>\w+)"


class _SiblingPrimaryIE(_BaseIE):
    _VALID_URL = r"https?://shared\.example/play/(?P<id>\w+)"


class _SiblingSeriesIE(_BaseIE):
    """SVT-style sibling override: `_VALID_URL` matches the same prefix
    as SiblingPrimaryIE, but `suitable()` defers when the more-specific
    sibling claims the URL first."""
    _VALID_URL = r"https?://shared\.example/(?P<id>\w+)"

    @classmethod
    def suitable(cls, url):
        if _SiblingPrimaryIE.suitable(url):
            return False
        return super().suitable(url)


# -----------------------------------------------------------------------------
# discover_ie_classes
# -----------------------------------------------------------------------------

class TestDiscoverIeClasses:
    def test_finds_single_concrete(self):
        mod = _FakeModule(AlphaIE=_AlphaIE)
        result = discover_ie_classes(mod)
        assert result == [_AlphaIE]

    def test_finds_multiple_concrete(self):
        mod = _FakeModule(AlphaIE=_AlphaIE, BetaIE=_BetaIE)
        result = discover_ie_classes(mod)
        assert set(result) == {_AlphaIE, _BetaIE}

    def test_excludes_abstract_base(self):
        # Base class with `_VALID_URL = None` (inherited from
        # InfoExtractor) must not appear as a candidate.
        mod = _FakeModule(BaseIE=_BaseIE, LeafIE=_LeafIE)
        assert discover_ie_classes(mod) == [_LeafIE]

    def test_excludes_info_extractor_itself(self):
        # Module exposing only the base class itself counts as "no
        # concrete IE" → DispatchError. The base must never be dispatched.
        mod = _FakeModule(InfoExtractor=InfoExtractor)
        with pytest.raises(DispatchError):
            discover_ie_classes(mod)

    def test_excludes_non_class_attributes(self):
        # Functions, constants, etc. must not be picked up.
        mod = _FakeModule(
            AlphaIE=_AlphaIE,
            some_helper=lambda x: x,
            CONSTANT=42,
            DESCRIPTION="not a class",
        )
        assert discover_ie_classes(mod) == [_AlphaIE]

    def test_raises_when_no_classes_found(self):
        mod = _FakeModule()
        with pytest.raises(DispatchError) as excinfo:
            discover_ie_classes(mod)
        assert "no InfoExtractor subclass" in str(excinfo.value)


# -----------------------------------------------------------------------------
# dispatch_url
# -----------------------------------------------------------------------------

class TestDispatchUrl:
    def test_single_class_match(self):
        cls = dispatch_url([_AlphaIE], "https://alpha.example/foo")
        assert cls is _AlphaIE

    def test_single_class_no_match(self):
        cls = dispatch_url([_AlphaIE], "https://other.example/foo")
        assert cls is None

    def test_first_match_wins_among_disjoint(self):
        cls = dispatch_url(
            [_AlphaIE, _BetaIE], "https://beta.example/x",
        )
        assert cls is _BetaIE

    def test_sibling_override_primary_wins(self):
        # SiblingPrimaryIE has the more-specific path `/play/...`;
        # SiblingSeriesIE.suitable() yields to it.
        url = "https://shared.example/play/abc"
        cls = dispatch_url(
            [_SiblingSeriesIE, _SiblingPrimaryIE], url,
        )
        assert cls is _SiblingPrimaryIE

    def test_sibling_override_series_claims_remainder(self):
        # When the URL doesn't match the primary, the sibling base
        # pattern wins.
        url = "https://shared.example/some-slug"
        cls = dispatch_url(
            [_SiblingSeriesIE, _SiblingPrimaryIE], url,
        )
        # SiblingPrimaryIE's regex requires `/play/`, so it won't
        # match. SiblingSeriesIE then claims via its broader regex.
        assert cls is _SiblingSeriesIE

    def test_dispatch_order_independent(self):
        # Same outcome regardless of input order — dispatch should
        # respect each class's `suitable()` not the iteration order.
        url = "https://shared.example/play/xyz"
        # Reverse the list
        cls = dispatch_url(
            [_SiblingPrimaryIE, _SiblingSeriesIE], url,
        )
        assert cls is _SiblingPrimaryIE

    def test_buggy_suitable_does_not_poison_dispatch(self):
        """A `suitable()` override raising an exception MUST NOT prevent
        sibling classes from being tried. The exception is logged via
        `_host.log` so the plugin author can find broken overrides."""
        class _BrokenIE(InfoExtractor):
            _VALID_URL = r"https?://broken\.example/(?P<id>\w+)"

            @classmethod
            def suitable(cls, url):
                raise RuntimeError("intentionally buggy override")

        # Dispatch must skip BrokenIE (logging the error) and find AlphaIE.
        cls = dispatch_url(
            [_BrokenIE, _AlphaIE], "https://alpha.example/foo",
        )
        assert cls is _AlphaIE

    def test_buggy_suitable_logs_via_host(self, monkeypatch):
        """Pin the warn-log call so a future refactor that silently
        swallows the exception triggers a regression."""
        logged = []

        from rdlp_ytdlp_compat import _host

        def fake_log(level, msg):
            logged.append((level, msg))

        monkeypatch.setattr(_host, "log", fake_log)

        class _BrokenIE2(InfoExtractor):
            _VALID_URL = r"https?://broken2\.example/(?P<id>\w+)"

            @classmethod
            def suitable(cls, url):
                raise RuntimeError("boom")

        dispatch_url([_BrokenIE2], "https://broken2.example/x")
        assert any(
            level == "warn" and "_BrokenIE2" in msg and "boom" in msg
            for level, msg in logged
        ), f"expected warn-level log mentioning class + error, got {logged!r}"
