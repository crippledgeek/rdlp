"""Pure-Python unit tests for Slice-2 utility helpers.

Mirrors yt-dlp upstream behaviour:
  - `determine_ext` — `yt_dlp/utils/_utils.py:1304-1314` @ tag 2026.03.17
  - `dict_get` — `yt_dlp/utils/traversal.py:473-477`
  - `require` — `yt_dlp/utils/traversal.py:320-327`
  - `variadic` — `yt_dlp/utils/_utils.py:2673-2677`
  - `RequiredError` — `yt_dlp/utils/traversal.py:330-331` (`_RequiredError`)
"""
import pytest

from rdlp_ytdlp_compat import ExtractorError
from rdlp_ytdlp_compat._errors import RequiredError
from rdlp_ytdlp_compat._utils import (
    determine_ext,
    dict_get,
    require,
    variadic,
)


class TestDetermineExt:
    """`determine_ext(url, default_ext='unknown_video')` parses extension
    from URL path. Strips query string, splits on last dot."""

    def test_none_returns_default(self):
        assert determine_ext(None) == "unknown_video"

    def test_no_dot_returns_default(self):
        assert determine_ext("https://example.com/foo") == "unknown_video"

    def test_simple_mp4(self):
        assert determine_ext("https://example.com/foo.mp4") == "mp4"

    def test_strips_query_string(self):
        assert determine_ext("https://example.com/foo.mp4?token=abc") == "mp4"

    def test_strips_query_before_extension_check(self):
        # `.com/foo?bar.baz` — the part after the last `.` BEFORE the `?`
        # is the extension; query string is irrelevant.
        assert determine_ext("https://x.com/foo.m3u8?bar.baz") == "m3u8"

    def test_alphanumeric_ext_accepted(self):
        # `mpd` — pure alphanumeric, accepted via the regex branch.
        assert determine_ext("https://x.com/v.mpd") == "mpd"

    def test_non_alphanumeric_ext_rejects_unless_known(self):
        # `mp4!` — not alphanumeric, not in KNOWN_EXTENSIONS → default.
        assert determine_ext("https://x.com/v.mp4!") == "unknown_video"

    def test_trailing_slash_with_known_extension(self):
        # `foo.mp4/` — yt-dlp's known-extension fallback strips trailing
        # slash and looks up in KNOWN_EXTENSIONS.
        assert determine_ext("http://x.com/foo.mp4/?download") == "mp4"

    def test_trailing_slash_unknown_returns_default(self):
        assert determine_ext("http://x.com/foo.unknown/?x") == "unknown_video"

    def test_custom_default(self):
        assert determine_ext(None, default_ext="bin") == "bin"

    def test_f4m_known(self):
        # SVT exercises `.f4m` — Adobe HDS manifest.
        assert determine_ext("https://x.com/stream.f4m") == "f4m"


class TestDictGet:
    """`dict_get(d, key_or_keys, default=None, skip_false_values=True)`."""

    def test_single_key_present(self):
        assert dict_get({"a": 1}, "a") == 1

    def test_single_key_absent(self):
        assert dict_get({"a": 1}, "b") is None

    def test_single_key_absent_custom_default(self):
        assert dict_get({"a": 1}, "b", default="x") == "x"

    def test_iterable_keys_first_wins(self):
        assert dict_get({"a": 1, "b": 2}, ("a", "b")) == 1

    def test_iterable_keys_first_missing_falls_back(self):
        assert dict_get({"b": 2}, ("a", "b")) == 2

    def test_iterable_keys_all_missing(self):
        assert dict_get({"c": 3}, ("a", "b")) is None

    def test_skip_false_values_default_skips_empty_string(self):
        # Default `skip_false_values=True`: empty string is falsy → skipped.
        assert dict_get({"a": "", "b": "x"}, ("a", "b")) == "x"

    def test_skip_false_values_default_skips_zero(self):
        assert dict_get({"a": 0, "b": 5}, ("a", "b")) == 5

    def test_skip_false_values_default_skips_empty_list(self):
        assert dict_get({"a": [], "b": [1]}, ("a", "b")) == [1]

    def test_skip_false_values_false_returns_zero(self):
        # SVT `_extract_video` calls `dict_get(...,
        # skip_false_values=False)` to distinguish absent from `False`.
        assert dict_get({"a": 0}, "a", skip_false_values=False) == 0

    def test_skip_false_values_false_returns_empty_string(self):
        assert dict_get({"a": ""}, "a", skip_false_values=False) == ""

    def test_skip_false_values_false_still_skips_none(self):
        # `None` is always skipped — distinguishes "absent" from "explicitly None".
        assert dict_get({"a": None, "b": 0}, ("a", "b"),
                        skip_false_values=False) == 0


class TestRequire:
    """`require(name, *, expected=False)` returns a callable that raises
    `RequiredError` on None input, else passes through."""

    def test_passes_through_non_none(self):
        check = require("foo")
        assert check("bar") == "bar"

    def test_passes_through_zero(self):
        # 0 is not None — should pass.
        check = require("foo")
        assert check(0) == 0

    def test_passes_through_empty_string(self):
        # Empty string is not None — should pass. (yt-dlp's require checks
        # `value is None`, NOT falsy.)
        check = require("foo")
        assert check("") == ""

    def test_raises_on_none(self):
        check = require("video data")
        with pytest.raises(RequiredError) as excinfo:
            check(None)
        assert "Unable to extract video data" in str(excinfo.value)

    def test_expected_flag_propagates(self):
        check = require("SVT ID", expected=True)
        with pytest.raises(RequiredError) as excinfo:
            check(None)
        assert excinfo.value.expected is True

    def test_default_expected_is_false(self):
        check = require("foo")
        with pytest.raises(RequiredError) as excinfo:
            check(None)
        assert excinfo.value.expected is False


class TestVariadic:
    """`variadic(x)` wraps scalar in tuple; passes iterables through.
    Strings/bytes/dicts are NEVER unwrapped (they're 'block' types)."""

    def test_string_wrapped(self):
        assert variadic("foo") == ("foo",)

    def test_bytes_wrapped(self):
        assert variadic(b"foo") == (b"foo",)

    def test_dict_wrapped(self):
        d = {"a": 1}
        assert variadic(d) == (d,)

    def test_int_wrapped(self):
        assert variadic(42) == (42,)

    def test_none_wrapped(self):
        assert variadic(None) == (None,)

    def test_tuple_passes_through(self):
        assert variadic(("a", "b")) == ("a", "b")

    def test_list_passes_through(self):
        assert variadic(["a", "b"]) == ["a", "b"]


class TestRequiredError:
    """`RequiredError` subclasses `ExtractorError` so existing
    `except ExtractorError:` clauses catch it."""

    def test_subclasses_extractor_error(self):
        err = RequiredError("missing", expected=True)
        assert isinstance(err, ExtractorError)

    def test_caught_as_extractor_error(self):
        with pytest.raises(ExtractorError):
            raise RequiredError("missing", expected=True)

    def test_expected_attribute(self):
        err = RequiredError("missing", expected=True)
        assert err.expected is True

    def test_default_expected_false(self):
        err = RequiredError("missing")
        assert err.expected is False

    def test_orig_msg_preserved(self):
        err = RequiredError("Unable to extract foo")
        assert err.orig_msg == "Unable to extract foo"
