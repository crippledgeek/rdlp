"""Utility helpers mirroring yt-dlp's `utils/_utils.py` and
`utils/traversal.py` — filesystem-safe filename / path helpers PLUS the
small pure helpers ported in Slice 2 (`determine_ext`, `dict_get`,
`require`, `variadic`).

Verified against `yt_dlp/utils/_utils.py:631-733` at tag `2026.03.17`.
The implementations don't try to be byte-identical (yt-dlp's full-width-
Unicode replacement is locale-aware in subtle ways), but the contract
matches:

    sanitize_filename(s, restricted=False) -> str

  - Returns a filesystem-safe variant of `s`.
  - Empty / all-stripped input returns `'_'` (so file creation never
    produces a zero-length name).
  - Forbidden-char set is the union of POSIX and Windows reserved bytes:
    `<`, `>`, `:`, `"`, `/`, `\\`, `|`, `?`, `*`, plus control chars
    (\\x00-\\x1f, \\x7f).
  - In the default (`restricted=False`) mode, forbidden chars are mapped
    to visually-similar full-width Unicode look-alikes (matches yt-dlp's
    "default" mode at line 645-647).
  - In `restricted=True` mode, non-ASCII / whitespace / common shell
    metachars collapse to `_` for ASCII-only filesystems.
  - Leading/trailing dots, spaces, and `_` are stripped.
  - Idempotent for typical strings (Unicode look-alikes from a previous
    pass aren't re-substituted because they're not in the forbidden set).

Plus `sanitize_path(s, force=False)` for full path-string sanitisation
(splits on separators, sanitises each segment).
"""
import re
import unicodedata

from rdlp_ytdlp_compat._errors import RequiredError


# Mirrors yt-dlp's MEDIA_EXTENSIONS namespace (_utils.py:5091-5102 @ tag
# 2026.03.17). `determine_ext` falls back to this set when the URL has a
# trailing slash that masks the real extension. Trimmed to the tuples
# yt-dlp populates by default; full namespace also has subtitles + images
# but extractors classifying those typically don't go through `determine_ext`.
_KNOWN_EXTENSIONS = frozenset((
    # video
    "3g2", "3gp", "avi", "divx", "f4v", "flv", "m4v", "mk3d", "mkv",
    "mov", "mp4", "mpg", "ogv", "webm", "wmv",
    # audio
    "aac", "aiff", "alac", "ape", "asf", "f4a", "f4b", "flac", "m4a",
    "m4b", "m4r", "mka", "mp3", "oga", "ogg", "ogx", "opus", "spx",
    "vorbis", "wav", "weba", "wma",
    # streaming manifests
    "f4f", "f4m", "m3u8", "mpd", "smil",
))


def variadic(x, allowed_types=(str, bytes, dict)):
    """yt-dlp's `variadic` (`_utils.py:2673-2677` @ tag 2026.03.17): wrap a
    scalar in a 1-tuple; pass iterables through unchanged.

    `allowed_types` are the types that should NEVER be unwrapped (so a
    string `key` doesn't get iterated character-by-character). Real
    yt-dlp uses a NO_DEFAULT sentinel and emits a deprecation warning;
    we hardcode the (str, bytes, dict) default since every call site in
    the shim relies on it.
    """
    if isinstance(x, allowed_types):
        return (x,)
    if hasattr(x, "__iter__"):
        return x
    return (x,)


def determine_ext(url, default_ext="unknown_video"):
    """yt-dlp's `determine_ext` (`_utils.py:1304-1314` @ tag 2026.03.17).

    Returns the extension parsed from a URL's path: strips the query
    string at `?`, takes the suffix after the last `.`. Returns
    `default_ext` when:
      - URL is None, or
      - URL contains no `.`, or
      - The suffix is non-alphanumeric AND not in `_KNOWN_EXTENSIONS`
        (handles trailing-slash URLs like `…/foo.mp4/?download`).
    """
    if url is None or "." not in url:
        return default_ext
    guess = url.partition("?")[0].rpartition(".")[2]
    if re.match(r"^[A-Za-z0-9]+$", guess):
        return guess
    if guess.rstrip("/") in _KNOWN_EXTENSIONS:
        return guess.rstrip("/")
    return default_ext


def dict_get(d, key_or_keys, default=None, skip_false_values=True):
    """yt-dlp's `dict_get` (`utils/traversal.py:473-477` @ tag 2026.03.17).

    Iterate `key_or_keys` (single key or iterable of keys), returning
    the first dict value that is non-None. When `skip_false_values=True`
    (the default), also skip falsy-but-not-None values (empty string, 0,
    empty list). Pass `skip_false_values=False` when the caller needs to
    distinguish "key absent" from "key present with value 0/'' " — SVT
    `_extract_video` does this for the `inappropriateForChildren` /
    `blockedForChildren` fields where `False` is a meaningful signal.

    `None` is ALWAYS skipped regardless of `skip_false_values` — that's
    the key invariant separating "absent" from "explicitly None".
    """
    for val in map(d.get, variadic(key_or_keys)):
        if val is not None and (val or not skip_false_values):
            return val
    return default


def clean_html(html):
    """yt-dlp's `clean_html` (`_utils.py:527-540` @ tag 2026.03.17).
    Strip HTML tags, collapse whitespace, convert `<br>` to newlines,
    convert `</p><p>` to newlines, unescape entities. `None` passes
    through (callers pass `_search_regex(..., default=None)` results)."""
    import html as _stdlib_html
    if html is None:
        return html
    html = re.sub(r"\s+", " ", html)
    html = re.sub(r"(?u)\s?<\s?br\s?/?\s?>\s?", "\n", html)
    html = re.sub(r"(?u)<\s?/\s?p\s?>\s?<\s?p[^>]*>", "\n", html)
    html = re.sub(r"<.*?>", "", html)
    html = _stdlib_html.unescape(html)
    return html.strip()


def parse_duration(s):
    """yt-dlp's `parse_duration` (`_utils.py:2082-2136` @ tag 2026.03.17).
    Three-tier matcher: colon-separated (DD:HH:MM:SS / HH:MM:SS / MM:SS
    / SS), letter-suffixed (`1h2m3s` / ISO-8601 PT-form), and verbose
    English (`1 hour 2 mins`). Returns float seconds or `None`."""
    if not isinstance(s, str):
        return None
    s = s.strip()
    if not s:
        return None

    days = hours = mins = secs = ms = None
    m = re.match(
        r"""(?x)
            (?P<before_secs>
                (?:(?:(?P<days>[0-9]+):)?(?P<hours>[0-9]+):)?(?P<mins>[0-9]+):)?
            (?P<secs>(?(before_secs)[0-9]{1,2}|[0-9]+))
            (?P<ms>[.:][0-9]+)?Z?$
        """,
        s,
    )
    if m:
        days, hours, mins, secs, ms = m.group(
            "days", "hours", "mins", "secs", "ms",
        )
    else:
        m = re.match(
            r"""(?ix)(?:P?
                (?:[0-9]+\s*y(?:ears?)?,?\s*)?
                (?:[0-9]+\s*m(?:onths?)?,?\s*)?
                (?:[0-9]+\s*w(?:eeks?)?,?\s*)?
                (?:(?P<days>[0-9]+)\s*d(?:ays?)?,?\s*)?
                T)?
                (?:(?P<hours>[0-9]+)\s*h(?:(?:ou)?rs?)?,?\s*)?
                (?:(?P<mins>[0-9]+)\s*m(?:in(?:ute)?s?)?,?\s*)?
                (?:(?P<secs>[0-9]+)(?P<ms>\.[0-9]+)?\s*s(?:ec(?:ond)?s?)?\s*)?Z?$
            """,
            s,
        )
        if m:
            days, hours, mins, secs, ms = m.groups()
        else:
            m = re.match(
                r"(?i)(?:(?P<hours>[0-9.]+)\s*(?:hours?)|"
                r"(?P<mins>[0-9.]+)\s*(?:mins?\.?|minutes?)\s*)Z?$",
                s,
            )
            if m:
                hours, mins = m.groups()
            else:
                return None

    if ms:
        ms = ms.replace(":", ".")
    return sum(
        float(part or 0) * mult
        for part, mult in (
            (days, 86400), (hours, 3600), (mins, 60), (secs, 1), (ms, 1),
        )
    )


def require(name, *, expected=False):
    """yt-dlp's `require` (`utils/traversal.py:320-327` @ tag 2026.03.17).

    Returns a callable that raises `RequiredError` when its input is
    None, otherwise passes the input through unchanged. Designed for use
    as a transformer inside `traverse_obj` paths:

        traverse_obj(data, ('video', 'svtId', {str}, {require('SVT ID')}))

    The traversal engine catches `RequiredError` thrown by intermediate
    paths so the next path can be tried, and re-raises as a plain
    `ExtractorError(expected=...)` once the last path is exhausted. The
    `expected` flag rides through to the surfaced ExtractorError.
    """
    def _check(value):
        if value is None:
            raise RequiredError(f"Unable to extract {name}", expected=expected)
        return value
    return _check



# Mirrors yt-dlp's `ACCENT_CHARS` (`_utils.py:580-628` at tag 2026.03.17),
# trimmed to the subset that actually occurs in real-world video metadata.
# Used in `restricted=True` mode to collapse accented characters to ASCII
# rather than dropping them.
_ACCENT_CHARS = dict(
    zip(
        "ÂÃÄÀÁÅÆÇÈÉÊËÌÍÎÏÐÑÒÓÔÕÖØÙÚÛÜÝÞßàáâãäåæçèéêëìíîïðñòóôõöøùúûüýþÿ",
        (
            "A", "A", "A", "A", "A", "A", "AE", "C", "E", "E", "E", "E",
            "I", "I", "I", "I", "D", "N", "O", "O", "O", "O", "O", "O",
            "U", "U", "U", "U", "Y", "TH", "ss",
            "a", "a", "a", "a", "a", "a", "ae", "c", "e", "e", "e", "e",
            "i", "i", "i", "i", "d", "n", "o", "o", "o", "o", "o", "o",
            "u", "u", "u", "u", "y", "th", "y",
        ),
    )
)


def _replace_default(c):
    """Default-mode replacement: full-width Unicode look-alikes for forbidden
    chars (`/` → U+29F8, `\\` → U+29F9, others via U+FEE0 offset). Mirrors
    yt-dlp `_utils.py:645-647`."""
    if c == "/":
        return "⧸"
    if c == "\\":
        return "⧹"
    return chr(ord(c) + 0xFEE0)


def sanitize_filename(s: str, restricted: bool = False) -> str:
    """Return a filesystem-safe variant of `s`. Mirrors yt-dlp's
    `sanitize_filename` (`_utils.py:631-683`).

    - `restricted=False` (default): forbidden chars → full-width Unicode
      look-alikes (preserves visual appearance).
    - `restricted=True`: ASCII-only output; non-ASCII / whitespace /
      shell metachars collapse to `_`. Use for environments that don't
      handle Unicode filenames cleanly (FAT32, some sync tools).

    Empty or all-stripped input returns `'_'` so file creation never
    produces a zero-length name.
    """
    if s is None:
        return "_"
    if not isinstance(s, str):
        s = str(s)
    if not s:
        return "_"

    if restricted:
        # NFKC-normalize then strip accents (matches yt-dlp's restricted mode).
        s = unicodedata.normalize("NFKC", s)
        s = "".join(_ACCENT_CHARS.get(c, c) for c in s)
        # Forbidden = whitespace + common shell metachars + non-ASCII.
        s = re.sub(r"[^\x20-\x7E]", "_", s)
        s = re.sub(r"[!&'()\[\]{}$;`^,#\s/\\<>:\"|?*]", "_", s)
    else:
        # Default mode — full-width replacement for forbidden chars.
        s = re.sub(r'[<>:"/\\|*]', lambda m: _replace_default(m.group(0)), s)
        # Control chars and `?` deleted (yt-dlp `_utils.py:648`).
        s = re.sub(r"[\x00-\x1f\x7f?]", "", s)

    # Collapse leading/trailing whitespace, dots (Windows-hostile), and
    # underscores. Strip after replacement to handle "  foo  " and ".foo".
    s = s.strip(". _")
    # Collapse runs of underscores to a single underscore (yt-dlp pattern).
    s = re.sub(r"__+", "_", s)
    # Empty after cleanup → "_" so the filename is at least a valid path.
    if not s:
        return "_"
    return s


def str_to_int(s):
    """yt-dlp's `str_to_int` (utils/_utils.py). Parses 'human' integer
    strings: '1,234' / '1.5K' / '2M' / '3B' / plain digits. Truncates
    decimals (yt-dlp's behaviour). Returns None on parse failure."""
    if s is None:
        return None
    if not isinstance(s, str):
        try:
            return int(s)
        except (TypeError, ValueError):
            return None
    cleaned = s.replace(",", "").strip()
    if not cleaned:
        return None
    multiplier = 1
    if cleaned[-1].upper() in ("K", "M", "B"):
        multiplier = {"K": 1_000, "M": 1_000_000, "B": 1_000_000_000}[cleaned[-1].upper()]
        cleaned = cleaned[:-1]
    try:
        return int(float(cleaned) * multiplier)
    except (ValueError, TypeError):
        return None


def url_or_none(s):
    """yt-dlp's `url_or_none` (utils/_utils.py). Returns the input if
    it's a usable URL (http/https/protocol-relative/data:), else None.
    Rejects relative paths and javascript: URIs."""
    if not isinstance(s, str):
        return None
    if s.startswith(("http://", "https://", "//", "data:")):
        return s
    return None


# Path-segment separator — POSIX uses `/`; Windows uses both. We split on
# either to mirror yt-dlp's `_sanitize_path_parts` behaviour.
_PATH_SEP_RE = re.compile(r"[/\\]")


def sanitize_path(s: str, force: bool = False) -> str:
    """Sanitise a path string by splitting on separators and applying
    `sanitize_filename` to each segment. Mirrors yt-dlp's `sanitize_path`
    (`_utils.py:706-733`).

    `force=True` enables Windows-style sanitisation regardless of platform
    (matches yt-dlp's `--windows-filenames` behaviour). Slice-1 implementation
    treats both modes identically — Windows-specific reserved-name handling
    (CON/NUL/AUX/COM*/LPT*) is not yet implemented and would be a Slice-2
    addition if a Windows-targeted plugin needs it.
    """
    if s is None:
        return ""
    if not isinstance(s, str):
        s = str(s)
    if not s:
        return ""
    parts = _PATH_SEP_RE.split(s)
    sanitised = [sanitize_filename(p) for p in parts if p]
    return "/".join(sanitised)
