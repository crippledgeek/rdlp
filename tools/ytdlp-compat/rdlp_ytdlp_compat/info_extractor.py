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
#
# `__new__` enforces singleton: every `_NoDefault()` call returns the same
# instance, so any `is NO_DEFAULT` check stays correct even when a caller
# accidentally constructs a fresh instance. Without this guard,
# `default=_NoDefault()` would be `is not NO_DEFAULT` and silently take the
# "real default supplied" branch — wrong return path for `_search_regex` /
# `traverse_obj`. Same pattern as `inspect.Parameter.empty` /
# `dataclasses.MISSING` in the stdlib.
class _NoDefault:
    _instance = None

    def __new__(cls):
        if cls._instance is None:
            cls._instance = super().__new__(cls)
        return cls._instance

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


def traverse_obj(obj, *paths, default=NO_DEFAULT, expected_type=None,
                 get_all=True, casesense=True, traverse_string=False):
    """yt-dlp's traverse_obj — Slice-1 subset.

    Each path is a tuple of segments. Supported segment types in Slice 1:
      - str: dict key (case-sensitive unless `casesense=False`)
      - int: list index
      - Ellipsis (...): iterate all values of a Mapping/Iterable
      - callable: filter (keep elements where callable(item) is truthy)

    Default behaviour:
      - default=NO_DEFAULT: returns [] on branched miss (when get_all=True),
        None on scalar miss. Real extractors rely on this distinction —
        if traverse_obj(...) AND len(...) both work on branched paths.
      - get_all=True: branched paths return a list of all matches.
      - get_all=False: return only the first match, no branching.
      - casesense=False: dict-key lookup is case-insensitive.

    Multiple paths: first non-empty result wins.
    """
    sentinel_default = default is NO_DEFAULT
    any_branched = False
    for path in paths:
        if not isinstance(path, tuple):
            path = (path,)
        is_branched = any(seg is Ellipsis or callable(seg) for seg in path)
        any_branched = any_branched or is_branched
        result = _traverse_one(obj, path, get_all=get_all, casesense=casesense)
        if result is None or (isinstance(result, list) and not result):
            continue
        if expected_type is not None:
            if isinstance(result, list):
                result = [v for v in result if isinstance(v, expected_type)]
                if not result:
                    continue
            elif not isinstance(result, expected_type):
                continue
        if get_all is False and isinstance(result, list):
            result = result[0] if result else None
            if result is None:
                continue
        return result
    # No path produced a hit
    if sentinel_default:
        # yt-dlp default: [] for branched/get_all paths, None for scalar paths.
        return [] if (any_branched and get_all) else None
    return default


def _traverse_one(obj, path, get_all=True, casesense=True):
    if not path:
        return obj
    head, *rest = path
    if head is Ellipsis:
        if isinstance(obj, dict):
            items = list(obj.values())
        elif isinstance(obj, (list, tuple)):
            items = list(obj)
        else:
            return None
        out = []
        for item in items:
            v = _traverse_one(item, rest, get_all=get_all, casesense=casesense) if rest else item
            if v is None:
                continue
            if isinstance(v, list):
                out.extend(v)
            else:
                out.append(v)
        return out if out else []
    if callable(head):
        if not isinstance(obj, (list, tuple)):
            return None
        return [item for item in obj if head(item)]
    if isinstance(head, str) and isinstance(obj, dict) and not casesense:
        for k, v in obj.items():
            if isinstance(k, str) and k.lower() == head.lower():
                return _traverse_one(v, rest, get_all=get_all, casesense=casesense) if rest else v
        return None
    try:
        nxt = obj[head]
    except (KeyError, IndexError, TypeError):
        return None
    return _traverse_one(nxt, rest, get_all=get_all, casesense=casesense) if rest else nxt


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

    # yt-dlp's standard "raise" helpers (see common.py:1251-1280). Real
    # extractors invoke `self.raise_login_required(...)` / etc. very often;
    # without these the ports fail with AttributeError. Each helper raises
    # an `ExtractorError(expected=True)` tagged with a marker phrase so the
    # _entry.py WIT mapping can pick the right variant.

    @property
    def _ie_name(self):
        """The extractor's own name, for ExtractorError(ie=...) parity with
        upstream. yt-dlp uses `IE_NAME` if the extractor declares it,
        falling back to the class name minus the trailing `IE`."""
        ie_name = getattr(self, "IE_NAME", None)
        if ie_name is None:
            cls_name = type(self).__name__
            ie_name = cls_name[:-2] if cls_name.endswith("IE") else cls_name
        return ie_name

    def raise_login_required(
        self, msg="This video is only available for registered users",
        metadata_available=False, method=None,
    ):
        """yt-dlp's raise_login_required. Maps to WIT auth-required.

        Sets `ie=self._ie_name` on the underlying ExtractorError so ported
        code that does `except ExtractorError as e: log(e.ie)` sees the
        extractor identity (matches upstream behaviour)."""
        from rdlp_ytdlp_compat._errors import ExtractorError
        full_msg = msg
        if method is not None:
            full_msg = f"{msg}. Use {method} to log in."
        # Marker phrase keys the _entry.py message-text dispatch.
        raise ExtractorError(
            f"[login required] {full_msg}", expected=True, ie=self._ie_name,
        )

    def raise_geo_restricted(
        self, msg="This video is not available from your location due to geo restriction",
        countries=None, metadata_available=False,
    ):
        """yt-dlp's raise_geo_restricted. Maps to WIT auth-required (Slice 1
        has no dedicated geo-restricted variant)."""
        from rdlp_ytdlp_compat._errors import GeoRestrictedError
        raise GeoRestrictedError(msg, countries=countries, ie=self._ie_name)

    def raise_no_formats(self, msg, expected=False, video_id=None):
        """yt-dlp's raise_no_formats. Maps to WIT not-found when expected
        (the user-facing "no formats available"); to internal otherwise.

        Sets `ie=self._ie_name` for upstream parity."""
        from rdlp_ytdlp_compat._errors import ExtractorError
        # Marker phrase keys the _entry.py dispatch.
        prefix = "[no formats] "
        raise ExtractorError(
            f"{prefix}{msg}", expected=expected, video_id=video_id,
            ie=self._ie_name,
        )

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
                return _host.fetch_text(
                    url, headers=headers or [], timeout_ms=30000,
                    expected_status=expected_status,
                )
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

    def _search_regex(self, pattern, string, name, default=NO_DEFAULT,
                      fatal=True, flags=0, group=None):
        """yt-dlp's _search_regex. CRITICAL: real default is fatal=True (extractors
        omit the kwarg expecting raise-on-miss). Accepts a single pattern OR an
        iterable of patterns/compiled regexes; first match wins.

        - group=None: return first non-None group, or group(0) if no groups.
        - group=int/str: return that named/numbered group.
        - group=list/tuple: return tuple of groups.
        """
        patterns = pattern if isinstance(pattern, (list, tuple)) else [pattern]
        m = None
        for pat in patterns:
            m = _re.search(pat, string, flags) if isinstance(pat, str) else pat.search(string)
            if m is not None:
                break
        if m is not None:
            if group is None:
                groups = m.groups()
                if groups:
                    for g in groups:
                        if g is not None:
                            return g
                return m.group(0)
            if isinstance(group, (list, tuple)):
                return tuple(m.group(g) for g in group)
            return m.group(group)
        # No match
        if default is NO_DEFAULT:
            if fatal:
                raise ValueError(f"Unable to extract {name}")
            _host.log("warn", f"unable to extract {name}; returning None")
            return None
        return default

    def _html_search_meta(self, name, html, display_name=None, fatal=False, **kwargs):
        """yt-dlp's _html_search_meta. `name` accepts scalar or iterable of meta-tag
        names. Real yt-dlp matches FIVE attributes: itemprop|name|property|id|http-equiv
        — limiting to property+name breaks any extractor that uses og: tags via
        http-equiv or itemprop microdata. The default fatal is False (distinct
        from _search_regex)."""
        names = name if isinstance(name, (list, tuple)) else [name]
        default = kwargs.get("default", NO_DEFAULT)
        for n in names:
            attrs = "(?:itemprop|name|property|id|http-equiv)"
            # content first
            pat = rf'<meta[^>]+(?:{attrs})=["\']{_re.escape(n)}["\'][^>]*content=["\']([^"\']*)["\']'
            m = _re.search(pat, html, _re.IGNORECASE)
            if m is None:
                # content second
                pat = rf'<meta[^>]+content=["\']([^"\']*)["\'][^>]*(?:{attrs})=["\']{_re.escape(n)}["\']'
                m = _re.search(pat, html, _re.IGNORECASE)
            if m is not None:
                return m.group(1)
        if default is not NO_DEFAULT:
            return default
        if fatal:
            raise ValueError(f"Unable to extract {display_name or names[0]}")
        return None

    def _extract_m3u8_formats(self, *args, **kwargs):
        """yt-dlp's _extract_m3u8_formats. Returns formats only (drops subs).
        For (formats, subs) tuple, use _extract_m3u8_formats_and_subtitles."""
        formats, _subs = self._extract_m3u8_formats_and_subtitles(*args, **kwargs)
        return formats

    def _extract_m3u8_formats_and_subtitles(
            self, m3u8_url, video_id, ext=None, entry_protocol="m3u8_native",
            preference=None, quality=None, m3u8_id=None, note=None, errnote=None,
            fatal=True, live=False, data=None, headers=None, query=None):
        """yt-dlp's _extract_m3u8_formats_and_subtitles. Slice-1 scope: master
        playlist only — does NOT recurse into media playlists. Returns
        (formats, subs={}).

        Honors yt-dlp's `note` / `errnote` / `fatal` logging contract: a
        host-fetch failure raises only when `fatal=True` (the yt-dlp default);
        otherwise the error is logged via `errnote` and an empty
        (formats=[], subs={}) result is returned. Real extractors test
        `if formats:` and fall back to a non-HLS path when the playlist is
        unreachable — silently re-raising would break that pattern."""
        if note is not None:
            _host.log("info", f"{note}: {m3u8_url}")
        try:
            body = _host.fetch_text(m3u8_url, headers=headers)
        except Exception as e:
            if errnote is not None:
                _host.log("warn", f"{errnote}: {e}")
            if fatal:
                raise
            return [], {}
        formats = []
        lines = body.splitlines()
        for i, line in enumerate(lines):
            if not line.startswith("#EXT-X-STREAM-INF:"):
                continue
            attrs = self._parse_m3u8_attrs(line[len("#EXT-X-STREAM-INF:"):])
            if i + 1 >= len(lines):
                continue
            url = lines[i + 1].strip()
            if not url or url.startswith("#"):
                continue
            url = urljoin(m3u8_url, url)
            bandwidth = int_or_none(attrs.get("BANDWIDTH"), scale=1000)
            resolution = attrs.get("RESOLUTION", "")
            width = height = None
            if "x" in resolution:
                w, h = resolution.split("x", 1)
                width = int_or_none(w)
                height = int_or_none(h)
            formats.append({
                "format_id": f"{m3u8_id}-{bandwidth}" if m3u8_id else str(bandwidth or len(formats)),
                "url": url,
                "ext": ext or "mp4",
                "protocol": entry_protocol,
                "tbr": bandwidth,
                "width": width,
                "height": height,
                "preference": preference,
                "quality": quality,
            })
        return formats, {}

    @staticmethod
    def _parse_m3u8_attrs(s):
        """Parse `KEY1=VALUE1,KEY2="VALUE 2"` into a dict."""
        out = {}
        for m in _re.finditer(r'([A-Z0-9-]+)=(?:"([^"]*)"|([^,]*))', s):
            out[m.group(1)] = m.group(2) if m.group(2) is not None else m.group(3)
        return out
