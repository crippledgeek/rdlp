"""Multi-class plugin dispatch.

A yt-dlp source file like `svt.py` ships a base `InfoExtractor`
subclass plus several concrete subclasses (`SVTPlayIE`, `SVTSeriesIE`,
`SVTPageIE`). At extract time the host needs to pick the right class
for an incoming URL. The contract:

  - `discover_ie_classes(module)` returns every CONCRETE
    `InfoExtractor` subclass exposed at module scope. Concrete = has a
    non-None `_VALID_URL`. Abstract bases (e.g. `SVTBaseIE`) are
    filtered out so they aren't tried for dispatch.

  - `dispatch_url(classes, url)` walks the candidates in order, calling
    each class's `suitable(url)`. The first True wins. `suitable()` is
    the public yt-dlp idiom for sibling-override (e.g. SVTSeriesIE
    yields when SVTPlayIE.suitable(url) — see svt.py:307).

This module is import-time pure so it's unit-testable in plain Python.
The auto-generated `_entry.py` template imports both functions and uses
them inside the `extract()` body.
"""
from rdlp_ytdlp_compat.info_extractor import InfoExtractor


class DispatchError(RuntimeError):
    """Raised when a plugin module exposes zero `InfoExtractor`
    subclasses. `_entry.py` translates this to an
    `ExtractError_Internal` variant before crossing the WIT boundary."""


def discover_ie_classes(module):
    """Return every concrete `InfoExtractor` subclass exposed by
    `module`. Concrete = `_VALID_URL` is set (a string or a non-empty
    iterable). Abstract bases like `SVTBaseIE` (which inherit
    `_VALID_URL = None` from the base class) are excluded so they
    aren't tried for dispatch.

    Order is preserved as `dir(module)` returns names — typically
    alphabetical, which matches yt-dlp's `gen_extractors` ordering.
    """
    candidates = []
    for name in dir(module):
        if name.startswith("_"):
            continue
        value = getattr(module, name)
        if not isinstance(value, type):
            continue
        if value is InfoExtractor:
            continue
        if not issubclass(value, InfoExtractor):
            continue
        if getattr(value, "_VALID_URL", None) is None:
            continue
        candidates.append(value)
    if not candidates:
        raise DispatchError(
            "no InfoExtractor subclass found in plugin — declare "
            "`class FooIE(InfoExtractor):` with a `_VALID_URL` regex "
            "at module top level"
        )
    return candidates


def dispatch_url(classes, url):
    """Walk `classes` in order; return the first whose `suitable(url)`
    is True. Returns None when no class claims the URL."""
    for cls in classes:
        try:
            if cls.suitable(url):
                return cls
        except Exception:  # noqa: BLE001
            # `suitable()` is user code — a buggy override shouldn't
            # poison dispatch for subsequent candidates. Continue.
            continue
    return None
