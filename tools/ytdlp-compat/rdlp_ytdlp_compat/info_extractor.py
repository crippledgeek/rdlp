"""yt-dlp InfoExtractor compatibility — utility helpers (Slice 1).

I/O helpers (_download_webpage, _parse_json, _search_regex, etc.) are added
in Tasks 6-7. This module is import-time pure (no host imports), so its
helpers are fully unit-testable in plain Python.
"""
import datetime
import email.utils
import json as _json
import re as _re
from urllib.parse import urljoin as _stdlib_urljoin

from rdlp_ytdlp_compat import _host


# yt-dlp uses a NO_DEFAULT sentinel to distinguish "caller passed default=None"
# from "no default at all". We mirror that convention.
class _NoDefault:
    def __repr__(self):
        return "NO_DEFAULT"


NO_DEFAULT = _NoDefault()


def int_or_none(v, scale=1, default=None, get_attr=None, invscale=1, base=None):
    """yt-dlp's int_or_none. Real signature includes get_attr (call getattr first),
    invscale (multiplicative inverse — `int_or_none(x, invscale=8)` for bytes→bits),
    and base (radix for `int(s, base=...)`)."""
    if get_attr is not None:
        v = getattr(v, get_attr, None)
    if v is None or v == "":
        return default
    try:
        if base is not None:
            v = int(v, base)
        else:
            v = int(v)
        return v * invscale // scale
    except (ValueError, TypeError):
        return default


def try_get(src, getter, expected_type=None):
    """yt-dlp's try_get. `getter` accepts a single callable OR an iterable of
    callables; first non-exception, type-matching, non-None result wins."""
    getters = getter if isinstance(getter, (list, tuple)) else (getter,)
    for g in getters:
        try:
            v = g(src)
        except (KeyError, IndexError, TypeError, AttributeError):
            continue
        if v is None:
            continue
        if expected_type is not None and not isinstance(v, expected_type):
            continue
        return v
    return None


def urljoin(base, path):
    """yt-dlp's urljoin. Decodes bytes; returns None for non-`https?://`/`//` base;
    returns path unchanged if already absolute. Differs from stdlib urljoin."""
    if isinstance(path, bytes):
        path = path.decode("utf-8", errors="replace")
    if not isinstance(path, str) or not path:
        return None
    # Already absolute
    if path.startswith(("http://", "https://")):
        return path
    if path.startswith("//"):
        return "https:" + path
    if isinstance(base, bytes):
        base = base.decode("utf-8", errors="replace")
    if not isinstance(base, str):
        return None
    # yt-dlp's urljoin returns None unless base looks like a URL.
    if not _re.match(r"^(?:https?:)?//", base):
        return None
    return _stdlib_urljoin(base, path)


def unified_timestamp(date_str, day_first=True, tz_offset=0):
    """yt-dlp's unified_timestamp. day_first=True is yt-dlp's default — controls
    DD/MM vs MM/DD ambiguity. tz_offset shifts the parsed timestamp.

    Slice-1 implementation handles ISO 8601 and RFC 2822. Full yt-dlp date_formats
    table is deferred to Slice 2.
    """
    if date_str is None or not isinstance(date_str, str):
        return None
    s = date_str.strip()
    # ISO 8601 (Z or +HH:MM)
    try:
        normalized = s.replace("Z", "+00:00") if s.endswith("Z") else s
        dt = datetime.datetime.fromisoformat(normalized)
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=datetime.timezone.utc)
        return int(dt.timestamp()) + tz_offset
    except (ValueError, TypeError):
        pass
    # RFC 2822
    try:
        dt = email.utils.parsedate_to_datetime(s)
        if dt is not None:
            return int(dt.timestamp()) + tz_offset
    except (TypeError, ValueError):
        pass
    return None


class InfoExtractor:
    """Base class for yt-dlp-style extractors. I/O helpers added in Tasks 6-7.

    Real extractors subclass this and override `_real_extract(url)` to return a
    dict with keys: id, title, formats (list), and optional uploader, duration,
    thumbnail, description, etc.
    """

    _VALID_URL = None  # subclass overrides

    def __init__(self):
        pass

    def _real_extract(self, url):
        raise NotImplementedError

    # Re-export utility helpers as instance methods for yt-dlp compatibility
    # (real yt-dlp extractors do `self._int_or_none(...)`).
    def _int_or_none(self, *args, **kwargs):
        return int_or_none(*args, **kwargs)

    def _try_get(self, *args, **kwargs):
        return try_get(*args, **kwargs)

    def _download_webpage(self, url_or_request, video_id, note=None, errnote=None,
                          fatal=True, tries=1, timeout=NO_DEFAULT, *,
                          encoding=None, data=None, headers=None, query=None,
                          expected_status=None, impersonate=None,
                          require_impersonation=False):
        """yt-dlp's _download_webpage. `timeout` here is the inter-retry SLEEP
        (NOT a request timeout — that's controlled host-side). `headers` and
        `query` default to None (instead of yt-dlp's mutable {} default).
        impersonate/require_impersonation are accepted for compatibility but
        ignored — TLS impersonation is host-side via wreq."""
        url = url_or_request if isinstance(url_or_request, str) else url_or_request.url
        if note is not None:
            _host.log("info", f"{note}: {url}")
        last_err = None
        for _ in range(max(1, tries)):
            try:
                return _host.fetch_text(url, headers=headers or [], timeout_ms=30000)
            except Exception as e:
                last_err = e
                if errnote is not None:
                    _host.log("warn", f"{errnote}: {e}")
        if fatal and last_err is not None:
            raise last_err
        return None

    def _parse_json(self, json_string, video_id, transform_source=None,
                    fatal=True, errnote=None, **parser_kwargs):
        """yt-dlp's _parse_json. NOTE: yt-dlp has no `lenient` kwarg — it always
        uses LenientJSONDecoder with strict=False. **parser_kwargs lets callers
        pass `ignore_extra=True` etc. to the underlying decoder."""
        if transform_source is not None:
            json_string = transform_source(json_string)
        try:
            return _json.loads(json_string, **parser_kwargs)
        except (TypeError, ValueError) as e:
            if fatal:
                raise
            if errnote is not None:
                _host.log("warn", f"{errnote} for {video_id}: {e}")
            return None
