"""yt-dlp exception hierarchy mirror — drop-in compatible with `from
yt_dlp.utils import ExtractorError`, plus rdlp-specific typed subclasses
that map 1:1 to WIT `extract-error` variants.

## Two-layer design

**Upstream-mirrored (drop-in):** `YoutubeDLError`, `ExtractorError`,
`UnsupportedError`, `RegexNotFoundError`, `GeoRestrictedError`,
`UserNotLive`, `DownloadError`, `UnavailableVideoError`,
`ContentTooShortError`, `PostProcessingError`, `DownloadCancelled`. Class
shapes verified against yt-dlp tag `2026.03.17`, file
`yt_dlp/utils/_utils.py:966-1162`.

**rdlp-specific (typed dispatch):** `LoginRequiredError`, `NoFormatsError`,
`NotFoundError`, `RateLimitedError`, `NetworkError`. These don't exist
upstream — yt-dlp uses `ExtractorError(msg, expected=True)` + helper
methods. We add typed subclasses on top so the WIT-variant dispatcher
can use `isinstance` (canonical Python idiom — same shape as
`httpx.exceptions.HTTPError` family) instead of fragile substring
matching on message text. All five subclass `ExtractorError`, so
`except ExtractorError:` clauses in ported code still catch them and
`from yt_dlp.utils import ExtractorError` substitution still works.

## Cause-chain handling

componentize-py 0.17.2 only marshals `Err.value` (the variant payload)
across the WIT boundary — `__cause__`, `__context__`, `__traceback__`,
and `args` are all dropped. The `_entry.py` dispatcher therefore
flattens one level of `e.__cause__` (and yt-dlp's own `e.cause`
attribute) into the variant payload string at the boundary. Use the
modern PEP-3134 form when re-raising (`raise X() from e`) — the
dispatcher reads both `__cause__` and the legacy `cause` attribute.
"""


class YoutubeDLError(Exception):
    """Root of yt-dlp's exception hierarchy.

    yt-dlp's `__init__` mirrors `Exception.__init__`; we follow the same
    pattern so `str(e)` and `e.msg` both work.
    """

    def __init__(self, msg=None):
        if msg is not None:
            super().__init__(msg)
            self.msg = msg
        else:
            super().__init__()
            self.msg = None


class ExtractorError(YoutubeDLError):
    """Raised when an extractor cannot produce a usable info-dict.

    Constructor signature matches upstream exactly so ported code using
    `raise ExtractorError("foo", cause=e, video_id=vid, expected=True)`
    works without modification.

    `expected=True` distinguishes "the site told us no" (auth, geo, not
    found, rate-limited — caller should not file a bug report) from
    "the extractor probably has a bug" (`expected=False`, the default).
    The `_entry.py` WIT mapping uses `expected` plus message-text
    heuristics to choose between the `auth-required`, `not-found`,
    `rate-limited`, `parse`, and `internal` variants.
    """

    def __init__(
        self, msg, tb=None, expected=False, cause=None, video_id=None, ie=None
    ):
        super().__init__(msg)
        self.orig_msg = str(msg)
        self.traceback = tb
        self.expected = expected
        self.cause = cause
        self.video_id = video_id
        self.ie = ie
        self.exc_info = None  # set by yt-dlp's __init__ via sys.exc_info();
        # we leave it None — components don't have a useful traceback object
        # to capture across the WIT boundary anyway.


class UnsupportedError(ExtractorError):
    """The given URL doesn't match this extractor's `_VALID_URL`.

    Single-arg constructor matching upstream (`_utils.py:1022-1026`).
    `expected=True` is forced; raising this is never a bug report.
    """

    def __init__(self, url):
        super().__init__("Unsupported URL: %s" % url, expected=True)
        self.url = url


class RegexNotFoundError(ExtractorError):
    """Raised by `_search_regex` when nothing matched and `fatal=True`.

    Subclass marker only — no extra fields. Maps to `extract-error::parse`.
    """


class GeoRestrictedError(ExtractorError):
    """Content is geo-restricted; `countries` is the allowed-country list.

    Constructor matches upstream (`_utils.py:1034-1044`). `expected=True`
    is forced. The host's WIT mapping treats this as `auth-required` for
    Slice 1 (no dedicated `geo-restricted` variant in the current WIT);
    a Slice-2 WIT bump can add the variant if needed.
    """

    def __init__(self, msg, countries=None, **kwargs):
        kwargs.setdefault("expected", True)
        super().__init__(msg, **kwargs)
        self.countries = countries


class UserNotLive(ExtractorError):
    """The channel is not currently live (matches upstream
    `_utils.py:1047-1052`)."""

    def __init__(self, msg=None, **kwargs):
        kwargs.setdefault("expected", True)
        super().__init__(msg or "The channel is not currently live", **kwargs)


class DownloadError(YoutubeDLError):
    """Sibling of `ExtractorError`, NOT a subclass. Raised by the downloader
    layer (`_utils.py:1055-1066`); extractors normally don't raise this but
    some defensive code catches it.
    """

    def __init__(self, msg, exc_info=None):
        super().__init__(msg)
        self.exc_info = exc_info


class UnavailableVideoError(YoutubeDLError):
    """Sibling, raised by `YoutubeDL.process_info` when a format is
    unavailable (`_utils.py:1136-1147`). Not raised from `_real_extract`."""

    def __init__(self, err=None):
        super().__init__("Unable to download video" if err is None else str(err))


class ContentTooShortError(YoutubeDLError):
    """Sibling, raised by the downloader on truncated downloads
    (`_utils.py:1150-1162`).
    """

    def __init__(self, downloaded, expected):
        super().__init__(
            "Downloaded %d bytes, expected %d bytes" % (downloaded, expected)
        )
        self.downloaded = downloaded
        self.expected = expected


class PostProcessingError(YoutubeDLError):
    """Stub. yt-dlp postprocessing isn't run inside the plugin sandbox; the
    class exists so `from yt_dlp.utils import PostProcessingError` resolves
    in ported code that imports-and-passes-through."""


class DownloadCancelled(YoutubeDLError):
    """Cancellation signal (epoch deadline, user abort). Maps to WIT
    `extract-error::cancelled`. Upstream subclasses (`ExistingVideoReached`,
    `RejectedVideoReached`, `MaxDownloadsReached`) are not ported — they're
    YoutubeDL-process-control signals, not extractor outputs.
    """


# =============================================================================
# rdlp-specific typed subclasses (NOT upstream — see module docstring)
# =============================================================================
#
# These exist so the WIT-variant dispatcher in `_entry.py` can use
# `isinstance` instead of substring matching on `e.orig_msg`. All five
# inherit from `ExtractorError` so:
#   - `except ExtractorError:` in ported code still catches them
#   - `expected=True` is forced (these are all "site told us no", not bugs)
#   - the WIT mapping is 1:1 by class identity, not by message-text inspection
#
# Pattern (httpx / grpc.RpcError / urllib3.MaxRetryError) verified against
# Python idiomatic exception design. Drop-in compat preserved.


class LoginRequiredError(ExtractorError):
    """The video requires authentication. Maps to WIT `auth-required`.

    Raised by `InfoExtractor.raise_login_required`. Default message
    matches yt-dlp upstream so ported code that catches and re-raises
    sees identical text.
    """

    def __init__(self, msg=None, **kwargs):
        kwargs.setdefault("expected", True)
        super().__init__(
            msg or "This video is only available for registered users",
            **kwargs,
        )


class NoFormatsError(ExtractorError):
    """No playable formats found. Maps to WIT `not-found`.

    Raised by `InfoExtractor.raise_no_formats`. yt-dlp upstream uses
    `ExtractorError(msg, expected=expected)` with no typed subclass; we
    add the typed form so the dispatcher can route by class.
    """

    def __init__(self, msg=None, **kwargs):
        kwargs.setdefault("expected", True)
        super().__init__(msg or "No video formats available", **kwargs)


class NotFoundError(ExtractorError):
    """The video / item identified by the URL doesn't exist (HTTP 404,
    explicit "removed" / "deleted" page). Maps to WIT `not-found`.

    No upstream equivalent; rdlp adds this for plugin-author convenience.
    Ported extractors using `raise ExtractorError("not found", expected=True)`
    fall through to the dispatcher's `expected=True → parse` default —
    explicit `raise NotFoundError(...)` is preferred for new ports.
    """

    def __init__(self, msg=None, **kwargs):
        kwargs.setdefault("expected", True)
        super().__init__(msg or "Resource not found", **kwargs)


class RateLimitedError(ExtractorError):
    """The site rate-limited this request. Maps to WIT
    `rate-limited(option<u32>)`.

    `retry_after` is the optional retry-after duration in seconds (matches
    HTTP 429 `Retry-After: <delta-seconds>`). The dispatcher reads the
    typed attribute and passes it as the variant's `option<u32>` payload,
    so retry hints survive the WIT boundary cleanly.
    """

    def __init__(self, msg=None, retry_after=None, **kwargs):
        kwargs.setdefault("expected", True)
        super().__init__(msg or "Rate limited by site", **kwargs)
        self.retry_after = retry_after


class NetworkError(ExtractorError):
    """Network-layer failure surfaced into the extractor. Maps to WIT
    `network`.

    The host's `_host._check_status` raises `_HostHttpError` (a
    `RuntimeError` subclass with a typed `.status` attribute) on non-2xx
    responses; the dispatcher routes those by status code without going
    through `NetworkError`. Use `NetworkError` for application-level
    network failures the extractor classifies itself (e.g. an empty body
    where one was expected).
    """

    def __init__(self, msg, **kwargs):
        kwargs.setdefault("expected", True)
        super().__init__(msg, **kwargs)


class RequiredError(ExtractorError):
    """Raised by `require(name)` when a `traverse_obj` path produces None
    where a value is required. Mirrors yt-dlp's private `_RequiredError`
    (`utils/traversal.py:330-331` @ tag 2026.03.17).

    `traverse_obj` catches `RequiredError` thrown by intermediate paths
    (so the next path can be tried) and re-raises as
    `ExtractorError(msg, expected=...)` from the LAST path. Subclassing
    `ExtractorError` keeps `except ExtractorError:` clauses in ported
    code working identically.
    """

    def __init__(self, msg, *, expected=False):
        super().__init__(msg, expected=expected)


# Network exceptions yt-dlp's ExtractorError auto-wraps (`_utils.py:986`).
# We don't ship the actual urllib/http.client classes, but exposing the
# tuple lets caller code do `isinstance(e, network_exceptions)` symmetrically.
# Slice-1 implementation: empty tuple. The host's `_HostHttpError` is the
# typed network-failure surface inside the runtime; the `_entry.py` template
# routes it by typed class rather than message-prefix matching.
network_exceptions = ()
