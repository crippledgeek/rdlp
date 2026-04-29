"""yt-dlp InfoExtractor compatibility — utility helpers (Slice 1).

I/O helpers (_download_webpage, _parse_json, _search_regex, etc.) are added
in Tasks 6-7. This module is import-time pure (no host imports), so its
helpers are fully unit-testable in plain Python.
"""
import collections.abc as _collections_abc
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


def _is_iterable_like(obj):
    """Mirrors yt-dlp's `is_iterable_like`. A value is "iterable-like" if it
    has __iter__ and is NOT a string/bytes/dict/Mapping (which we treat
    as scalars or via different segment kinds)."""
    if isinstance(obj, (str, bytes, dict)):
        return False
    if isinstance(obj, _collections_abc.Mapping):
        return False
    return hasattr(obj, "__iter__")


def _try_call(func, args=()):
    """yt-dlp's `try_call` (`_utils.py:2680-2697`) — call func, swallow
    the documented exception classes and return None. We DO NOT swallow
    `RequiredError` — it MUST propagate so traverse_obj can catch it on
    the final path and re-raise as ExtractorError.
    """
    from rdlp_ytdlp_compat._errors import RequiredError as _RequiredError
    try:
        return func(*args)
    except _RequiredError:
        raise
    except (AttributeError, KeyError, TypeError, IndexError, ValueError,
            ZeroDivisionError):
        return None


def traverse_obj(obj, *paths, default=NO_DEFAULT, expected_type=None,
                 get_all=True, casesense=True, traverse_string=False):
    """Safely traverse nested `dict`s and `Iterable`s — Slice-2 subset of
    yt-dlp's `utils/traversal.py:38-313` @ tag 2026.03.17.

    Each path is wrapped in `variadic`, so `'key'` is the same as
    `('key',)`. The first path producing a non-None result wins; if a
    path branches but produces empty, the next path is tried.

    Supported segment types:

      - `None`              return current object unchanged
      - `str` / `int`       dict key / list index lookup
      - `Ellipsis (...)`    branch over all values of Mapping/Iterable
      - `set` of one type   type filter — keep value iff isinstance
      - `set` of N types    multi-type filter
      - `set` of one func   transformer — apply func, use return value
      - `(branch, branch)`  branch into multiple sub-paths, chain results
      - `callable(k, v)`    predicate filter on Mapping.items() /
                            enumerate(Iterable)
      - `any` builtin       collapse to first non-None, reset branching
      - `all` builtin       collect into list, reset branching
      - `filter` builtin    drop falsy values

    Slice-2 deliberately drops upstream support for `re.Match`,
    `xml.etree.ElementTree`, `http.cookies.Morsel`, `slice`, and
    `traverse_string` — none of these are exercised by SVT or by any
    plausible Slice-2 plugin. Add when a port needs them.

    `default=NO_DEFAULT`: missing paths return None for scalars and `[]`
    for branched paths. `get_all=False` returns only the first match.
    `casesense=False` makes dict-key lookups case-insensitive.
    `expected_type` (a type) keeps the value only if isinstance — applied
    AFTER traversal so it filters branched results too.

    `RequiredError` raised by `{require(...)}` segments is caught here
    on non-final paths (so the next path can be tried) and re-raised as
    `ExtractorError(msg, expected=...)` once the last path is exhausted.
    """
    from rdlp_ytdlp_compat._errors import (
        ExtractorError as _ExtractorError,
        RequiredError as _RequiredError,
    )

    casefold = lambda k: k.casefold() if isinstance(k, str) else k

    if expected_type is None:
        type_test = lambda val: val
    elif isinstance(expected_type, type):
        type_test = lambda val: val if isinstance(val, expected_type) else None
    else:
        type_test = lambda val: _try_call(expected_type, args=(val,))

    def apply_key(key, obj, is_last):
        """Apply ONE path segment to ONE object. Returns
        `(branched: bool, results: iterable)`. When non-branched the
        iterable is a 1-tuple; when branched it's a flat iterable of all
        results."""
        branching = False
        result = None

        if key is None:
            result = obj

        elif isinstance(key, set):
            # Set semantics (`utils/traversal.py:127-134`):
            #   {type}              → type filter (require isinstance)
            #   {type1, type2, ...} → multi-type filter
            #   {callable}          → transformer (apply, use return)
            item = next(iter(key))
            if len(key) > 1 or isinstance(item, type):
                # Type filter — every member must be a type.
                assert all(isinstance(member, type) for member in key), (
                    "set in traverse_obj path with non-type members must "
                    "have exactly one element (the transformer callable)"
                )
                if isinstance(obj, tuple(key)):
                    result = obj
            else:
                # Single-callable transformer. RequiredError propagates.
                result = _try_call(item, args=(obj,))

        elif isinstance(key, (list, tuple)):
            # Sub-path branching — recurse into each branch and chain.
            branching = True
            chained = []
            for branch in key:
                sub_results, _, _ = apply_path(obj, branch, is_last)
                chained.extend(sub_results)
            result = chained

        elif key is Ellipsis:
            branching = True
            if isinstance(obj, _collections_abc.Mapping):
                result = list(obj.values())
            elif _is_iterable_like(obj):
                result = list(obj)
            else:
                result = ()

        elif callable(key):
            # Predicate filter with `(key, value)` signature. Mapping →
            # iterates .items(); Iterable → enumerate(); else empty.
            branching = True
            if isinstance(obj, _collections_abc.Mapping):
                iter_obj = obj.items()
            elif _is_iterable_like(obj):
                iter_obj = enumerate(obj)
            else:
                iter_obj = ()
            result = [v for k, v in iter_obj if _try_call(key, args=(k, v))]

        elif isinstance(obj, _collections_abc.Mapping):
            # Plain key lookup. Case-insensitive when casesense=False.
            if casesense or key in obj:
                result = obj.get(key)
            else:
                result = next(
                    (v for k, v in obj.items() if casefold(k) == key),
                    None,
                )

        elif isinstance(key, int):
            if _is_iterable_like(obj) and isinstance(
                obj, _collections_abc.Sequence,
            ):
                try:
                    result = obj[key]
                except IndexError:
                    result = None

        return branching, (result if branching else (result,))

    def apply_path(start_obj, path, test_type):
        """Walk a single path through current state. Returns
        `(results_iter, has_branched, ends_in_dict)`."""
        from rdlp_ytdlp_compat._utils import variadic as _variadic
        objs = (start_obj,)
        has_branched = False
        seq = list(_variadic(path, (str, bytes, dict, set)))

        last_key = None
        for idx, key in enumerate(seq):
            is_last = idx == len(seq) - 1
            last_key = key

            if not casesense and isinstance(key, str):
                key = key.casefold()

            # `any` / `all` / `filter` are scope resets (do NOT consume an
            # object — they reshape the current `objs` collection).
            if key in (any, all):
                # After `any`/`all`, branching is collapsed. Note that for
                # `any`, `objs` becomes a 1-tuple containing either the
                # first non-None match OR `None` — NOT an empty tuple.
                # That way a subsequent `{require(...)}` segment still gets
                # invoked on `None` and raises `RequiredError` as upstream
                # `yt_dlp/utils/traversal.py:260-267` does.
                has_branched = False
                filtered = (o for o in objs if o not in (None, {}))
                if key is any:
                    objs = (next(filtered, None),)
                else:
                    objs = (list(filtered),)
                continue
            if key is filter:
                objs = tuple(o for o in objs if o)
                continue

            new_objs = []
            for o in objs:
                branched, results = apply_key(key, o, is_last)
                has_branched = has_branched or branched
                new_objs.extend(results)
            objs = new_objs

        return objs, has_branched, False  # Slice-2 doesn't support dict-segment

    sentinel_default = default is NO_DEFAULT
    last_index = len(paths) - 1
    last_has_branched = False  # tracks the terminal path's branching state
    for index, path in enumerate(paths):
        is_last = index == last_index
        try:
            results, has_branched, _ = apply_path(obj, path, True)
        except _RequiredError as e:
            if is_last:
                raise _ExtractorError(e.orig_msg, expected=e.expected) from None
            continue
        last_has_branched = has_branched
        # Drop None / {} — yt-dlp's "unhelpful values".
        cleaned = [r for r in results if r not in (None, {})]
        if expected_type is not None:
            cleaned = [type_test(v) for v in cleaned]
            cleaned = [v for v in cleaned if v is not None]

        if get_all and has_branched:
            if cleaned:
                return cleaned
            continue

        if cleaned:
            return cleaned[0]
    # No path produced a hit. Use the LAST path's runtime branching
    # state — `any`/`all` reset `has_branched` to False mid-path, so a
    # path ending in `any` is treated as scalar even if `...` appeared
    # earlier. This mirrors `yt_dlp/utils/traversal.py:293-298`.
    if sentinel_default:
        if get_all and last_has_branched:
            return []
        return None
    return default


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
    # without these the ports fail with AttributeError.
    #
    # Each helper raises a TYPED ExtractorError subclass (LoginRequiredError /
    # GeoRestrictedError / NoFormatsError) so the `_entry.py` WIT-variant
    # dispatcher can use `isinstance` instead of fragile substring matching
    # on message text. The typed subclasses all inherit from `ExtractorError`,
    # so `except ExtractorError:` in ported code still catches them.

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
        """yt-dlp's raise_login_required. Raises `LoginRequiredError` →
        WIT `auth-required`. Sets `ie=self._ie_name` for upstream parity."""
        from rdlp_ytdlp_compat._errors import LoginRequiredError
        full_msg = msg
        if method is not None:
            full_msg = f"{msg}. Use {method} to log in."
        raise LoginRequiredError(full_msg, ie=self._ie_name)

    def raise_geo_restricted(
        self, msg="This video is not available from your location due to geo restriction",
        countries=None, metadata_available=False,
    ):
        """yt-dlp's raise_geo_restricted. Raises `GeoRestrictedError` →
        WIT `auth-required` (Slice 1 has no dedicated geo-restricted variant;
        a Slice-2 WIT bump can split it out)."""
        from rdlp_ytdlp_compat._errors import GeoRestrictedError
        raise GeoRestrictedError(msg, countries=countries, ie=self._ie_name)

    def raise_no_formats(self, msg, expected=False, video_id=None):
        """yt-dlp's raise_no_formats. Raises `NoFormatsError` (when
        `expected=True`) → WIT `not-found`, or `ExtractorError(expected=False)`
        → WIT `internal` otherwise (yt-dlp convention for unexpected
        no-formats: it's usually an extractor bug)."""
        from rdlp_ytdlp_compat._errors import ExtractorError, NoFormatsError
        if expected:
            raise NoFormatsError(msg, video_id=video_id, ie=self._ie_name)
        raise ExtractorError(
            msg, expected=False, video_id=video_id, ie=self._ie_name,
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
            except RuntimeError as e:
                # _host.fetch_text raises RuntimeError (and HostHttpError
                # which subclasses it) for HTTP failures and "outside
                # runtime". These are the only exception types we expect to
                # retry. TypeError / AttributeError / ImportError from
                # buggy host bindings or buggy extractor code propagate
                # through as bugs — masking them inside this retry loop is
                # the silent-failure pattern review I4 flagged.
                last_err = e
                # Always log on failure so test harnesses (caplog) and the
                # host see the diagnostic, even when the extractor author
                # forgot to pass `errnote`.
                label = errnote or f"failed to fetch {url}"
                _host.log("warn", f"{label}: {e}")
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
                # Raise the typed yt-dlp class so the WIT dispatcher routes
                # to extract-error::parse via isinstance, and ported code
                # using `except RegexNotFoundError:` keeps working.
                from rdlp_ytdlp_compat._errors import RegexNotFoundError
                raise RegexNotFoundError(f"Unable to extract {name}")
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
        except RuntimeError as e:
            # Same narrow catch as _download_webpage (I5): only RuntimeError
            # / HostHttpError subclasses are legitimate retry signals.
            # Always log on failure so the host sees the diagnostic.
            label = errnote or f"failed to fetch HLS playlist {m3u8_url}"
            _host.log("warn", f"{label}: {e}")
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
