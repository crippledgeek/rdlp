"""yt-dlp InfoExtractor compatibility shim for rdlp WASM plugins."""
from rdlp_ytdlp_compat.info_extractor import (
    InfoExtractor,
    int_or_none,
    try_get,
    urljoin,
    unified_timestamp,
)

__all__ = [
    "InfoExtractor",
    "int_or_none",
    "try_get",
    "urljoin",
    "unified_timestamp",
]
