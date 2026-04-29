"""Thin wrappers around the WIT host imports.

These imports are intentionally module-level (componentize-py issue #23 forbids
lazy imports). Outside a componentize-py-built component, `extractor_plugin`
(the renamed wit_world) doesn't exist — we catch `ModuleNotFoundError` so
unit tests can still import this module without crashing.

We catch the narrower `ModuleNotFoundError` rather than the broader
`ImportError` so that real bugs in the bindings module (e.g. partial
componentize-py output, a syntax error in a re-exported module) surface as
errors at module-load time rather than silently flipping us to "outside
runtime" mode and producing a plugin that emits no logs. I/O helpers raise
at call time when not available.
"""
import logging as _logging

_outside_runtime_logger = _logging.getLogger("rdlp_ytdlp_compat")

try:
    from extractor_plugin.imports import host_fetch, host_log
    from extractor_plugin.imports.host_fetch import Request as _Request
    _AVAILABLE = True
except ModuleNotFoundError:
    _AVAILABLE = False


def _check_status(status: int, expected_status, url: str) -> None:
    """Raise `RuntimeError` if `status` is not 2xx and is not the explicitly-
    allowed `expected_status`. Extracted for unit-testability — without
    componentize-py bindings present, the surrounding `fetch_text` cannot be
    called directly, but this pure-Python predicate can be exercised in tests
    to pin the contract.

    Callers that need to inspect a specific non-2xx (e.g. yt-dlp's
    `expected_status=404` for soft 404s) pass that integer; everything else
    in the non-2xx range raises.
    """
    if 200 <= status < 300:
        return
    if expected_status is not None and status == expected_status:
        return
    raise RuntimeError(f"HTTP {status} fetching {url}")


def fetch_text(url: str, headers: list = None, timeout_ms: int = 30000,
               expected_status: int = None) -> str:
    """Fetch a URL and return the body decoded as UTF-8 text.

    Raises `RuntimeError` if the HTTP status is not 2xx (and not equal to
    `expected_status` if supplied) — without this check, 4xx/5xx response
    bodies (login walls, error pages) would be returned as if they were the
    page content, masking auth failures and rate-limits as "extractor returns
    empty results".
    """
    if not _AVAILABLE:
        raise RuntimeError("_host.fetch_text called outside componentize-py runtime")
    req = _Request(
        url=url,
        method="GET",
        headers=headers or [],
        body=None,
        timeout_ms=timeout_ms,
    )
    resp = host_fetch.fetch(req)
    _check_status(resp.status, expected_status, url)
    return bytes(resp.body).decode("utf-8", errors="replace")


def log(level: str, message: str) -> None:
    """Forward a log line to the host. Outside the runtime, route through
    the stdlib `logging` module on the `rdlp_ytdlp_compat` logger so test
    harnesses (pytest's `caplog`) can assert on warning/error lines instead
    of silently swallowing them."""
    if not _AVAILABLE:
        # pytest captures via stdlib logging; route through it so test runs
        # surface extractor diagnostics. Map our level vocabulary to logging.
        level_map_stdlib = {
            "trace": _logging.DEBUG,
            "debug": _logging.DEBUG,
            "info": _logging.INFO,
            "warn": _logging.WARNING,
            "error": _logging.ERROR,
        }
        _outside_runtime_logger.log(
            level_map_stdlib.get(level, _logging.INFO), message
        )
        return
    level_map = {
        "trace": host_log.Level.TRACE,
        "debug": host_log.Level.DEBUG,
        "info": host_log.Level.INFO,
        "warn": host_log.Level.WARN,
        "error": host_log.Level.ERROR,
    }
    host_log.log(level_map.get(level, host_log.Level.INFO), message)
