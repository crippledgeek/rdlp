"""Auto-generated entry point for rdlp plugin build-from-ytdlp.

See the Rust doc-comment on ENTRY_TEMPLATE in build_from_ytdlp.rs for the
load-bearing invariants this file maintains. Editing this file directly is
pointless — it is regenerated on every build-from-ytdlp invocation.
"""
from extractor_plugin import ExtractorPlugin as _ExtractorPluginProtocol
from extractor_plugin.types import Err
from extractor_plugin.imports.types import (
    InfoDict, Format, PluginInfo, SearchPage,
    ExtractError_UnsupportedUrl, ExtractError_NotFound,
    ExtractError_RateLimited, ExtractError_AuthRequired,
    ExtractError_Network, ExtractError_Parse, ExtractError_Cancelled,
    ExtractError_Internal, SearchError_Unsupported,
)

# User plugin imports — must be top-level (componentize-py #23).
# Plugin source is staged at yt_dlp/extractor/{{PLUGIN_MODULE}}.py
# (Slice-2.5 fake-package layout for byte-identical drop-in).
from yt_dlp.extractor.{{PLUGIN_MODULE}} import *  # noqa: F401,F403

import yt_dlp.extractor.{{PLUGIN_MODULE}} as user_plugin
from rdlp_ytdlp_compat import (
    InfoExtractor as _CompatInfoExtractor,
    ExtractorError as _ExtractorError,
    UnsupportedError as _UnsupportedError,
    GeoRestrictedError as _GeoRestrictedError,
    UserNotLive as _UserNotLive,
    RegexNotFoundError as _RegexNotFoundError,
    DownloadCancelled as _DownloadCancelled,
    LoginRequiredError as _LoginRequiredError,
    NoFormatsError as _NoFormatsError,
    NotFoundError as _NotFoundError,
    RateLimitedError as _RateLimitedError,
    NetworkError as _NetworkError,
)
from rdlp_ytdlp_compat._host import HostHttpError as _HostHttpError
from rdlp_ytdlp_compat._dispatch import (
    DispatchError as _DispatchError,
    discover_ie_classes as _discover_ie_classes,
    dispatch_url as _dispatch_url,
)


# Discover all concrete InfoExtractor subclasses at module load. Exactly
# zero candidates = plugin-authoring error (raised loudly at first call
# below). Multi-class plugins (e.g. SVT with Play/Series/Page IEs in one
# file) keep all classes — extract-time dispatch picks the right one
# per URL via cls.suitable().
try:
    _IE_CLASSES = _discover_ie_classes(user_plugin)
except _DispatchError as _e:
    # Surface at extract() / metadata() rather than here — componentize-py
    # init errors crash the whole instance.
    _IE_CLASSES = []
    _DISCOVERY_ERROR = _e
else:
    _DISCOVERY_ERROR = None

# Pre-instantiate one IE per class. Slice-1 used a single `_IE` instance;
# Slice-2 keeps a dict so dispatch can route URLs to the right object.
_IE_INSTANCES = {cls: cls() for cls in _IE_CLASSES}


def _format_payload(e):
    """Flatten a Python exception into a WIT-payload string.

    componentize-py 0.17.2 only marshals `Err.value` (the variant payload)
    across the canonical-ABI boundary — `__cause__`, `__context__`,
    `__traceback__`, and `args` are all dropped. So debugging info (cause
    chain, video_id, extractor identity) MUST be flattened into the payload
    string at the dispatch site, otherwise the host sees `"plugin returned
    None"` with zero context.

    Reads BOTH Python's `__cause__` (set by `raise X() from e`, the modern
    PEP-3134 form) AND yt-dlp's legacy `cause` attribute (its own
    convention, set via `ExtractorError(msg, cause=e)`). Walks one level
    deep — deeper chains rarely add signal, and the host stderr captures
    the full traceback via the `log` capability if anyone needs it.
    """
    msg = str(getattr(e, "orig_msg", e) or "")
    # __cause__ first (PEP 3134), fall back to yt-dlp's `cause` attribute.
    cause = getattr(e, "__cause__", None)
    if cause is None:
        cause = getattr(e, "cause", None)
    if cause is not None:
        msg = f"{msg} (cause: {type(cause).__name__}: {cause})"
    vid = getattr(e, "video_id", None)
    if vid:
        msg = f"{msg} [video_id={vid}]"
    ie = getattr(e, "ie", None)
    if ie:
        msg = f"{msg} [ie={ie}]"
    return msg


def _extractor_error_to_variant(e):
    """Map a Python exception to the appropriate WIT extract-error variant.

    Pure `isinstance` ladder — leaf-first (most-specific subclasses checked
    before their parents) so e.g. `LoginRequiredError` (subclass of
    `ExtractorError`) routes to `auth-required` instead of falling through
    to the parent `ExtractorError` arm. Substring matching on message text
    is intentionally absent; relying on it produced false-positive routing
    of real bugs to "expected" variants, masking them from bug reports.

    Variants are listed in WIT-declaration order (see
    `crates/rdlp-plugin/wit/types.wit::extract-error`):

      unsupported-url(string)  — UnsupportedError
      not-found(string)        — UserNotLive, NoFormatsError, NotFoundError
      rate-limited(option<u32>) — RateLimitedError, HTTP 429
      auth-required(string)    — LoginRequiredError, GeoRestrictedError, HTTP 401/403
      network(string)          — NetworkError, HostHttpError (other 4xx/5xx)
      parse(string)            — RegexNotFoundError, bare ExtractorError(expected=True)
      cancelled                — DownloadCancelled
      internal(string)         — ExtractorError(expected=False), unknown exceptions
    """
    # === Typed subclasses — most-specific first =============================
    if isinstance(e, _UnsupportedError):
        return ExtractError_UnsupportedUrl(getattr(e, "url", _format_payload(e)))
    if isinstance(e, _LoginRequiredError):
        return ExtractError_AuthRequired(_format_payload(e))
    if isinstance(e, _GeoRestrictedError):
        # No dedicated geo variant in Slice-1 WIT; auth-required is the
        # closest match (both mean "you can't access this content").
        return ExtractError_AuthRequired(_format_payload(e))
    if isinstance(e, _NoFormatsError):
        return ExtractError_NotFound(_format_payload(e))
    if isinstance(e, _NotFoundError):
        return ExtractError_NotFound(_format_payload(e))
    if isinstance(e, _UserNotLive):
        return ExtractError_NotFound(_format_payload(e))
    if isinstance(e, _RateLimitedError):
        # `retry_after` carried as a typed attribute, NOT parsed from
        # message text — survives the WIT boundary as `option<u32>`.
        return ExtractError_RateLimited(e.retry_after)
    if isinstance(e, _NetworkError):
        return ExtractError_Network(_format_payload(e))
    if isinstance(e, _RegexNotFoundError):
        return ExtractError_Parse(_format_payload(e))
    if isinstance(e, _DownloadCancelled):
        return ExtractError_Cancelled()

    # === Bare ExtractorError parent ========================================
    # Default for expected=True: parse (yt-dlp semantics — "site told us no,
    # not our bug" maps closest to "we couldn't extract", which is parse).
    # Default for expected=False: internal (the extractor likely has a bug).
    if isinstance(e, _ExtractorError):
        msg = _format_payload(e)
        if getattr(e, "expected", False):
            return ExtractError_Parse(msg)
        return ExtractError_Internal(msg)

    # === Typed host HTTP error =============================================
    # _host.HostHttpError carries `.status` as a typed int attribute, so we
    # route by status code without parsing message strings (closes the
    # pre-fix "any RuntimeError starting with 'HTTP ' would mis-dispatch"
    # latent bug).
    if isinstance(e, _HostHttpError):
        status = e.status
        msg = str(e)
        if status == 429:
            return ExtractError_RateLimited(None)
        if status in (401, 403):
            return ExtractError_AuthRequired(msg)
        if status == 404:
            return ExtractError_NotFound(msg)
        if 400 <= status < 600:
            return ExtractError_Network(msg)
        # Status outside 4xx/5xx falling through here means caller passed
        # an `expected_status` that masked it — surface as Internal.
        return ExtractError_Internal(msg)

    # === Catch-all — extractor bug ========================================
    # Reuse `_format_payload` so the legacy `cause` attribute (yt-dlp's
    # own convention via `ExtractorError(msg, cause=e)`) is also
    # flattened, not just `__cause__`. Earlier this branch duplicated
    # the cause-flattening inline and dropped the legacy attr — found
    # in code review as a debug-info regression for unknown exception
    # types.
    return ExtractError_Internal(_format_payload(e))


def _validate_id(d):
    """yt-dlp's info_dict contract requires `id` and `title` as strs and
    either `formats` (list[dict]) OR `url` (str)
    (yt_dlp/extractor/common.py:122-129 @ tag 2026.03.17). Validate at the
    boundary so a buggy plugin returning {"id": None} surfaces a clear
    `ExtractError_Internal` instead of writing the literal string "None"
    into the archive.

    Errors are tagged "[validate]" so a debugger reading host logs can
    distinguish "plugin returned bad shape" from "plugin's _real_extract
    raised". Routes to ExtractError_Internal via the bare-ExtractorError
    arm of _extractor_error_to_variant.
    """
    if not isinstance(d, dict):
        raise _ExtractorError(
            f"[validate] plugin _real_extract returned {type(d).__name__}, "
            f"expected dict",
            expected=False,
        )
    vid = d.get("id")
    if not isinstance(vid, str) or not vid:
        raise _ExtractorError(
            f"[validate] info_dict 'id' must be a non-empty str, "
            f"got {type(vid).__name__}",
            expected=False,
        )
    title = d.get("title")
    if not isinstance(title, str):
        raise _ExtractorError(
            f"[validate] info_dict 'title' must be a str, "
            f"got {type(title).__name__}",
            expected=False,
        )
    formats = d.get("formats")
    url = d.get("url")
    has_formats = isinstance(formats, list) and len(formats) > 0
    has_url = isinstance(url, str) and url
    if not (has_formats or has_url):
        raise _ExtractorError(
            "[validate] info_dict has neither non-empty 'formats' nor 'url'",
            expected=False,
        )


# CRITICAL: class name must be `ExtractorPlugin` (matches --world-module
# PascalCase) for componentize-py-pin@0.17.2 to discover and instantiate it.
class ExtractorPlugin(_ExtractorPluginProtocol):
    def metadata(self) -> PluginInfo:
        if _DISCOVERY_ERROR is not None:
            raise Err(ExtractError_Internal(str(_DISCOVERY_ERROR)))
        # When multiple IE classes ship in one plugin, the manifest's
        # plugin name is determined by the `.py` filename (kebab-cased
        # at build time). For metadata reporting we use the first class
        # purely for the `url_regex` hint shown in `rdlp plugin info`;
        # actual matching still walks every class via `suitable()`.
        primary = _IE_CLASSES[0]
        return PluginInfo(
            name=primary.__name__.lower(),
            version="0.1.0",
            wit_version="0.1.0",
            matches=[],  # populated from manifest at install time
            url_regex=getattr(primary, "_VALID_URL", None),
            priority=150,
            claims_override=[],
            supports_search=False,
        )

    def extract(self, url: str) -> InfoDict:
        if _DISCOVERY_ERROR is not None:
            raise Err(ExtractError_Internal(str(_DISCOVERY_ERROR)))
        # Walk every IE class in the plugin and pick the first whose
        # `suitable(url)` is True. Sibling-override classes (e.g.
        # SVTSeriesIE.suitable yields when SVTPlayIE.suitable matches)
        # are honoured because dispatch_url respects each class's own
        # suitable() implementation.
        ie_class = _dispatch_url(_IE_CLASSES, url)
        if ie_class is None:
            raise Err(ExtractError_UnsupportedUrl(url))
        ie = _IE_INSTANCES[ie_class]
        # All extraction logic — including _dict_to_info_dict's _opt_str
        # type-check on each format field — runs INSIDE the try. Otherwise
        # _opt_str's _ExtractorError on a bad format dict would propagate
        # past the variant dispatcher as an uncaught Python exception,
        # which componentize-py surfaces as a wasm trap (instance killed,
        # epoch fuel lost) instead of the graceful WIT variant.
        try:
            d = ie._real_extract(url)
            _validate_id(d)
            return _dict_to_info_dict(d)
        except Exception as e:
            raise Err(_extractor_error_to_variant(e))

    def search(self, query) -> SearchPage:
        raise Err(SearchError_Unsupported())


def _opt_str(v, where=""):
    """Coerce optional str fields. None → None (not 'None'); int/float are
    converted only if the field's WIT type is `option<string>`. Anything
    else (list, dict) is rejected as a plugin bug.

    `where` is a human-readable location hint ("formats[3].format_id") that
    surfaces in the validation-error message — without it, debugging a
    deeply-nested type mismatch in an info-dict means trial-and-error.
    """
    if v is None:
        return None
    if isinstance(v, str):
        return v
    if isinstance(v, (int, float)):
        return str(v)
    locus = f" at {where}" if where else ""
    raise _ExtractorError(
        f"[validate] info-dict field{locus} has invalid type: "
        f"expected str/None, got {type(v).__name__}",
        expected=False,
    )


def _opt_uint(v, where=""):
    """Coerce optional unsigned-int fields to Python int. yt-dlp helpers
    (`parse_duration`, `int_or_none`) often return floats — passing a
    float to a WIT `option<u32>` / `option<u64>` field crashes the
    componentize-py canonical-ABI marshaller (`ToCanonU32`). Round-to-int
    here so plugin authors don't have to remember which fields require
    integer values; bool guarded explicitly because Python `bool` is an
    `int` subclass and silently coercing `True`→`1` is surprising."""
    if v is None:
        return None
    if isinstance(v, bool):
        locus = f" at {where}" if where else ""
        raise _ExtractorError(
            f"[validate] info-dict field{locus} is bool, "
            f"expected number or None",
            expected=False,
        )
    if isinstance(v, int):
        return max(0, v)
    if isinstance(v, float):
        return max(0, int(round(v)))
    locus = f" at {where}" if where else ""
    raise _ExtractorError(
        f"[validate] info-dict field{locus} has invalid type: "
        f"expected number/None, got {type(v).__name__}",
        expected=False,
    )


def _dict_to_info_dict(d: dict) -> InfoDict:
    # Build formats with index-aware error messages so a malformed
    # `formats[4].format_id` surfaces with that exact path, not just
    # `"info-dict field has invalid type: list"`.
    formats = []
    for i, f in enumerate(d.get("formats") or []):
        if not isinstance(f, dict):
            raise _ExtractorError(
                f"[validate] formats[{i}] must be a dict, "
                f"got {type(f).__name__}",
                expected=False,
            )
        formats.append(Format(
            format_id=_opt_str(f.get("format_id"), f"formats[{i}].format_id") or "",
            url=_opt_str(f.get("url"), f"formats[{i}].url") or "",
            ext=_opt_str(f.get("ext"), f"formats[{i}].ext") or "mp4",
            protocol=_opt_str(f.get("protocol"), f"formats[{i}].protocol") or "https",
            width=f.get("width"), height=f.get("height"), fps=f.get("fps"),
            tbr=f.get("tbr"), vbr=f.get("vbr"), abr=f.get("abr"),
            vcodec=_opt_str(f.get("vcodec"), f"formats[{i}].vcodec"),
            acodec=_opt_str(f.get("acodec"), f"formats[{i}].acodec"),
            container=_opt_str(f.get("container"), f"formats[{i}].container"),
            filesize=f.get("filesize"),
            format_note=_opt_str(f.get("format_note"), f"formats[{i}].format_note"),
        ))
    # Single-format-via-`url` canonicalisation. yt-dlp extractors
    # frequently return `{'id': ..., 'url': video_url, ...}` with no
    # explicit `formats[]` (xxxymovies, alphaporno, hellporno, many
    # others). Without this synthesis the WIT info-dict ships an empty
    # `formats[]` list and the URL is silently lost on the host side
    # (`adapter.rs::convert_info_dict` consumes it as `webpage_url` only).
    # Mirrors yt-dlp's internal `_check_formats` canonicalisation.
    if not formats and isinstance(d.get("url"), str) and d["url"]:
        formats.append(Format(
            format_id=_opt_str(d.get("format_id"), "format_id") or "0",
            url=d["url"],
            ext=_opt_str(d.get("ext"), "ext") or "mp4",
            protocol=_opt_str(d.get("protocol"), "protocol") or "https",
            width=d.get("width"), height=d.get("height"), fps=d.get("fps"),
            tbr=d.get("tbr"), vbr=d.get("vbr"), abr=d.get("abr"),
            vcodec=_opt_str(d.get("vcodec"), "vcodec"),
            acodec=_opt_str(d.get("acodec"), "acodec"),
            container=_opt_str(d.get("container"), "container"),
            filesize=d.get("filesize"),
            format_note=_opt_str(d.get("format_note"), "format_note"),
        ))
    # Use defensive .get with explicit default ("") — _validate_id already
    # ran and rejected non-str id/title, but keeping the .get makes the
    # coupling resilient to future refactors that might bypass the
    # validator.
    return InfoDict(
        id=d.get("id") or "",
        title=d.get("title") or "",
        url=_opt_str(d.get("url"), "url"),
        formats=formats, subtitles=[],
        thumbnail=_opt_str(d.get("thumbnail"), "thumbnail"),
        description=_opt_str(d.get("description"), "description"),
        uploader=_opt_str(d.get("uploader"), "uploader"),
        uploader_id=_opt_str(d.get("uploader_id"), "uploader_id"),
        upload_date=_opt_str(d.get("upload_date"), "upload_date"),
        duration=_opt_uint(d.get("duration"), "duration"),
        view_count=_opt_uint(d.get("view_count"), "view_count"),
        like_count=_opt_uint(d.get("like_count"), "like_count"),
        tags=list(d.get("tags") or []),
        categories=list(d.get("categories") or []),
    )
