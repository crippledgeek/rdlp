"""Thin wrappers around the WIT host imports.

These imports are intentionally module-level (componentize-py issue #23 forbids
lazy imports). Outside a componentize-py-built component, `extractor_plugin`
(the renamed wit_world) doesn't exist — we catch ImportError so unit tests
can still import this module without crashing. I/O helpers raise at call time
when not available.
"""
try:
    from extractor_plugin.imports import host_fetch, host_log
    from extractor_plugin.imports.host_fetch import Request as _Request
    _AVAILABLE = True
except ImportError:
    _AVAILABLE = False


def fetch_text(url: str, headers: list = None, timeout_ms: int = 30000,
               expected_status: int = None) -> str:
    """Fetch a URL and return the body decoded as UTF-8 text.

    Raises `RuntimeError` if the HTTP status is not 2xx (and not equal to
    `expected_status` if supplied) — without this check, 4xx/5xx response
    bodies (login walls, error pages) would be returned as if they were the
    page content, masking auth failures and rate-limits as "extractor returns
    empty results".

    Callers that need to inspect non-2xx responses (e.g. yt-dlp's
    `expected_status=404` for soft 404s) pass an explicit allowed status.
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
    status = resp.status
    if not (200 <= status < 300) and status != expected_status:
        raise RuntimeError(f"HTTP {status} fetching {url}")
    return bytes(resp.body).decode("utf-8", errors="replace")


def log(level: str, message: str) -> None:
    """Forward a log line to the host. Silent no-op outside the runtime."""
    if not _AVAILABLE:
        return
    level_map = {
        "trace": host_log.Level.TRACE,
        "debug": host_log.Level.DEBUG,
        "info": host_log.Level.INFO,
        "warn": host_log.Level.WARN,
        "error": host_log.Level.ERROR,
    }
    host_log.log(level_map.get(level, host_log.Level.INFO), message)
