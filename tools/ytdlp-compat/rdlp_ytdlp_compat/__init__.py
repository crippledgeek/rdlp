"""yt-dlp InfoExtractor compatibility shim for rdlp WASM plugins."""
from rdlp_ytdlp_compat._errors import (
    ContentTooShortError,
    DownloadCancelled,
    DownloadError,
    ExtractorError,
    GeoRestrictedError,
    PostProcessingError,
    RegexNotFoundError,
    UnavailableVideoError,
    UnsupportedError,
    UserNotLive,
    YoutubeDLError,
    network_exceptions,
)
from rdlp_ytdlp_compat.info_extractor import (
    NO_DEFAULT,
    InfoExtractor,
    int_or_none,
    traverse_obj,
    try_get,
    unified_timestamp,
    urljoin,
)

__all__ = [
    "InfoExtractor",
    "NO_DEFAULT",
    "int_or_none",
    "try_get",
    "urljoin",
    "unified_timestamp",
    "traverse_obj",
    # Exception hierarchy mirroring yt-dlp upstream — see _errors.py for
    # the full mapping table. Drop-in compatible: `from rdlp_ytdlp_compat
    # import ExtractorError` substitutes for `from yt_dlp.utils import ...`
    # in ported extractor source.
    "YoutubeDLError",
    "ExtractorError",
    "UnsupportedError",
    "RegexNotFoundError",
    "GeoRestrictedError",
    "UserNotLive",
    "DownloadError",
    "UnavailableVideoError",
    "ContentTooShortError",
    "PostProcessingError",
    "DownloadCancelled",
    "network_exceptions",
]
