"""yt-dlp InfoExtractor compatibility shim for rdlp WASM plugins."""
from rdlp_ytdlp_compat.info_extractor import (
    InfoExtractor,
    NO_DEFAULT,
    int_or_none, try_get, urljoin, unified_timestamp, traverse_obj,
)

__all__ = [
    "InfoExtractor",
    "NO_DEFAULT",
    "int_or_none", "try_get", "urljoin", "unified_timestamp", "traverse_obj",
]
