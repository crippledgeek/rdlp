"""traverse_obj — Slice-2 feature additions.

Slice-1 supported: str/int dict-key, Ellipsis, callable-as-list-filter
(single-arg). Slice-2 adds the segment types SVT exercises:

  - `{type}`               type filter (single-element set with a type)
  - `{type, type, ...}`    multi-type filter
  - `{callable}`           transformer (apply func to current value)
  - `any` builtin          first non-None, reset branching
  - `(branch, branch)`     sub-path tuple (branch into multiple sub-paths,
                           chain results)
  - `lambda k, v: ...`     two-arg predicate filter on Mapping items()
  - `RequiredError` propagation across paths

Verified against `yt_dlp/utils/traversal.py:38-313` @ tag 2026.03.17.
"""
import json

import pytest

from rdlp_ytdlp_compat import ExtractorError
from rdlp_ytdlp_compat._utils import require
from rdlp_ytdlp_compat.info_extractor import traverse_obj


class TestSetTypeFilter:
    """`{type}` keeps current value iff it is an instance of the type."""

    def test_str_type_keeps_string(self):
        assert traverse_obj("hello", {str}) == "hello"

    def test_str_type_drops_int(self):
        assert traverse_obj(42, {str}) is None

    def test_dict_type_keeps_dict(self):
        d = {"a": 1}
        assert traverse_obj(d, {dict}) == d

    def test_dict_type_drops_list(self):
        assert traverse_obj([1, 2], {dict}) is None

    def test_multi_type_set(self):
        # {str, int} keeps either string or int.
        assert traverse_obj("x", {str, int}) == "x"
        assert traverse_obj(7, {str, int}) == 7
        assert traverse_obj(1.5, {str, int}) is None

    def test_chained_after_dict_key(self):
        assert traverse_obj({"a": "x"}, ("a", {str})) == "x"
        assert traverse_obj({"a": 7}, ("a", {str})) is None


class TestSetCallableTransformer:
    """`{callable}` (single-element set with a non-type callable) applies
    the function to the current value."""

    def test_json_loads_transformer(self):
        # SVT pattern: `{json.loads}` parses JSON-encoded dict values.
        d = {"raw": '{"key": "value"}'}
        assert traverse_obj(d, ("raw", {json.loads})) == {"key": "value"}

    def test_callable_returns_value(self):
        assert traverse_obj("hello", {str.upper}) == "HELLO"

    def test_callable_returning_none_drops(self):
        # A transformer that returns None should drop the value (so the
        # next path can be tried).
        def always_none(_):
            return None
        assert traverse_obj({"a": 1}, ("a", {always_none})) is None

    def test_callable_raising_drops(self):
        # Transformer that raises (other than RequiredError) is caught and
        # treated as a miss — yt-dlp uses try_call for callable application.
        def bad(_):
            return 1 / 0
        assert traverse_obj("x", {bad}) is None


class TestRequireInPath:
    """`{require(name)}` raises RequiredError when value is None;
    traverse_obj catches on non-final paths and re-raises ExtractorError
    on the final path."""

    def test_require_passes_through(self):
        d = {"video": {"id": "abc123"}}
        assert traverse_obj(d, ("video", "id", {require("video id")})) == "abc123"

    def test_require_swallowed_on_non_final_path(self):
        # Two paths: first triggers require(None), second succeeds.
        # Should return second path's result without raising.
        d = {"fallback": "ok"}
        result = traverse_obj(
            d,
            ("missing", {require("video id")}),
            ("fallback",),
        )
        assert result == "ok"

    def test_require_raises_on_final_path(self):
        d = {"other": 1}
        with pytest.raises(ExtractorError) as excinfo:
            traverse_obj(d, ("missing", {require("video id")}))
        assert "Unable to extract video id" in str(excinfo.value)

    def test_require_expected_flag_propagates(self):
        with pytest.raises(ExtractorError) as excinfo:
            traverse_obj(
                {}, ("x", {require("foo", expected=True)}),
            )
        assert excinfo.value.expected is True


class TestAnyTerminator:
    """`any` builtin: take first non-None / non-empty value, reset
    branching state."""

    def test_any_after_branched_returns_first(self):
        # Branched path produces a list; `any` collapses to first element.
        d = {"items": [{"id": "a"}, {"id": "b"}, {"id": "c"}]}
        assert traverse_obj(d, ("items", ..., "id", any)) == "a"

    def test_any_skips_none(self):
        d = {"items": [{}, {"id": "first"}, {"id": "second"}]}
        # `...` over the list yields each item; "id" lookup returns None
        # for the first, "first" for the second. `any` picks first non-None.
        assert traverse_obj(d, ("items", ..., "id", any)) == "first"

    def test_any_no_match_returns_none(self):
        assert traverse_obj({"items": [{}, {}]}, ("items", ..., "id", any)) is None


class TestTupleSubpath:
    """`(branch_a, branch_b)` segment branches into multiple sub-paths
    and chains results."""

    def test_tuple_branches_chained(self):
        # SVT pattern from svt.py:463-465:
        #   ('page', (('topMedia', 'svtId'), ('body', ..., 'video', 'svtId')), {str}, ...)
        d = {
            "page": {
                "topMedia": {"svtId": "id-from-top"},
                "body": [
                    {"video": {"svtId": "id-from-body"}},
                ],
            }
        }
        result = traverse_obj(
            d,
            ("page",
             (("topMedia", "svtId"),
              ("body", ..., "video", "svtId")),
             {str}),
        )
        # Both branches resolve; result is a list of both ids.
        assert isinstance(result, list)
        assert set(result) == {"id-from-top", "id-from-body"}

    def test_tuple_branch_with_index_segment(self):
        # SVT line 268 pattern: `(lambda _, v: predicate, 0)` —
        # tuple of [callable predicate, int index] — both branches union.
        d = {"items": [{"key": "a"}, {"key": "b"}, {"key": "c"}]}
        # First branch: keep items where v["key"] == "b". Second: index 0.
        result = traverse_obj(
            d,
            ("items",
             (lambda _, v: v["key"] == "b", 0),
             "key"),
        )
        assert isinstance(result, list)
        # "b" from predicate branch + "a" from index branch.
        assert set(result) == {"a", "b"}


class TestPredicateCallableTwoArg:
    """A bare callable in the path with signature `(key, value)` is a
    Mapping/Iterable predicate filter — keep items where callable returns
    truthy. SVT uses this on lists of dicts."""

    def test_predicate_filters_list(self):
        items = [{"k": 1}, {"k": 2}, {"k": 3}]
        result = traverse_obj(items, (lambda _, v: v["k"] >= 2,))
        assert result == [{"k": 2}, {"k": 3}]

    def test_predicate_filters_dict_by_key(self):
        # On a Mapping, iter_obj is .items() so the callable sees (k, v).
        d = {"a": 1, "b": 2, "c": 3}
        result = traverse_obj(d, (lambda k, _: k in ("a", "c"),))
        # values for keys "a" and "c"
        assert isinstance(result, list)
        assert set(result) == {1, 3}


class TestGetAllFalse:
    """`get_all=False` returns first match instead of branched list."""

    def test_get_all_false_returns_scalar(self):
        d = {"items": [{"id": "a"}, {"id": "b"}]}
        result = traverse_obj(d, ("items", ..., "id"), get_all=False)
        assert result == "a"


class TestCasesenseFalse:
    """`casesense=False` makes dict-key lookup case-insensitive. The
    Slice-2 rewrite reassigns `key` via `casefold()` mid-iteration —
    pin behaviour against accidental regressions during future edits."""

    def test_case_insensitive_lookup(self):
        d = {"Title": "Hello"}
        assert traverse_obj(d, "title", casesense=False) == "Hello"

    def test_case_insensitive_lookup_nested(self):
        d = {"Outer": {"Inner": "value"}}
        assert traverse_obj(
            d, ("outer", "inner"), casesense=False,
        ) == "value"

    def test_casesense_true_default_misses_wrong_case(self):
        d = {"Title": "Hello"}
        # Default casesense=True → strict match.
        assert traverse_obj(d, "title") is None

    def test_case_insensitive_does_not_affect_int_index(self):
        # Integer indices are unaffected by casefold().
        assert traverse_obj([10, 20, 30], 1, casesense=False) == 20


class TestSliceOnePathBackcompat:
    """Slice-1 behaviour MUST remain green for the existing test surface."""

    def test_str_key_simple(self):
        assert traverse_obj({"a": 1}, "a") == 1

    def test_int_index(self):
        assert traverse_obj([10, 20, 30], 1) == 20

    def test_ellipsis_branch(self):
        d = {"x": 1, "y": 2}
        result = traverse_obj(d, ...)
        assert isinstance(result, list)
        assert sorted(result) == [1, 2]

    def test_first_path_wins(self):
        d = {"a": 1, "b": 2}
        assert traverse_obj(d, "a", "b") == 1
        assert traverse_obj(d, "missing", "b") == 2

    def test_default_returned(self):
        assert traverse_obj({}, "missing", default="X") == "X"

    def test_expected_type(self):
        assert traverse_obj({"a": "x"}, "a", expected_type=str) == "x"
        assert traverse_obj({"a": 7}, "a", expected_type=str) is None


class TestNestedSvtPattern:
    """End-to-end check against the exact SVT line 260-262 pattern.

      data = traverse_obj(self._search_nextjs_data(webpage, video_id), (
          'props', 'urqlState', ..., 'data', {json.loads},
          'detailsPageByPath', {dict}, any, {require('video data')}))
    """

    def test_full_pattern(self):
        nextjs = {
            "props": {
                "urqlState": {
                    "key1": {"data": '{"detailsPageByPath": {"video_meta": "yes"}}'},
                    "key2": {"data": '{"otherKey": {"foo": "bar"}}'},
                },
            },
        }
        result = traverse_obj(nextjs, (
            "props", "urqlState", ..., "data", {json.loads},
            "detailsPageByPath", {dict}, any, {require("video data")},
        ))
        assert result == {"video_meta": "yes"}

    def test_full_pattern_raises_when_missing(self):
        nextjs = {"props": {"urqlState": {}}}
        with pytest.raises(ExtractorError) as excinfo:
            traverse_obj(nextjs, (
                "props", "urqlState", ..., "data", {json.loads},
                "detailsPageByPath", {dict}, any, {require("video data")},
            ))
        assert "Unable to extract video data" in str(excinfo.value)
