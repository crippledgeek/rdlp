"""yt-dlp's `traverse_obj` dict-of-paths form (`utils/traversal.py:179-184`
@ tag 2026.03.17). Mirrors upstream `test/test_traversal.py:156-187`
assertions; subset chosen to cover the surface the rdlp shim exposes.

Triggered by godresource plugin port — the call

    traverse_obj(api_data, {
        'title': ('title', {str}),
        'thumbnail': ('thumbnail', {url_or_none}),
        ...
    })

failed with `TypeError: unhashable type: 'dict'` because the shim's
`apply_key` dispatch had no `dict` branch, so the dict fell through to
`Mapping.get(key)` which requires the key to be hashable.
"""
from rdlp_ytdlp_compat import traverse_obj, str_or_none


_TEST_DATA = {
    "str": "hi",
    "int": 0,
    "list": [1, 2, 3],
    "dict": {"a": "x", "b": None},
    "urls": [{"url": "https://example.com/0"}, {"url": "https://example.com/1"}],
}


def test_basic_dict_of_paths():
    """`{key: value_path}` produces `{key: traverse_obj(obj, value_path)}`."""
    assert traverse_obj(_TEST_DATA, {0: "str", 1: "int"}) == {0: "hi", 1: 0}


def test_dict_value_with_path_tuple():
    """Dict values can be nested path tuples."""
    assert traverse_obj(_TEST_DATA, {"u": ("urls", 0, "url")}) == {
        "u": "https://example.com/0"
    }


def test_dict_with_failing_key_pruned():
    """Keys whose sub-path returns None/{} are pruned from the result."""
    assert traverse_obj(_TEST_DATA, {0: "missing"}) == {}


def test_dict_with_default_keeps_failing_keys():
    """When `default` is set, failing sub-paths get the default."""
    assert traverse_obj(_TEST_DATA, {0: "missing"}, default="X") == {0: "X"}


def test_nested_dict_of_paths():
    """A dict path inside a dict value works recursively."""
    assert traverse_obj(_TEST_DATA, {0: {0: "missing"}}) == {}


def test_nested_dict_of_paths_with_default():
    """Nested dict failure with default produces nested default."""
    assert traverse_obj(
        _TEST_DATA, {0: {0: "missing"}}, default="X"
    ) == {0: {0: "X"}}


def test_dict_path_on_none_obj_returns_empty_dict():
    """When obj is None, a dict path returns {}, not None."""
    assert traverse_obj(None, {0: "anything"}) == {}


def test_set_transformer_in_dict_value():
    """Set-transformer (`{callable}`) in a dict value path applies the
    callable to the resolved value. This is the godresource-port shape.
    """
    assert traverse_obj(_TEST_DATA, {"s": ("int", {str_or_none})}) == {"s": "0"}


def test_godresource_shape():
    """Exact shape used by `examples/plugins/godresource/godresource.py`."""
    api_data = {
        "title": "Test",
        "thumbnail": "https://x/t.jpg",
        "views": 100,
        "channelName": "Stedfast",
        "channelId": 5,
    }
    result = traverse_obj(api_data, {
        "title": ("title", {str}),
        "thumbnail": ("thumbnail", {str_or_none}),
        "view_count": ("views", {int}),
        "channel": ("channelName", {str}),
        "channel_id": ("channelId", {str_or_none}),
    })
    assert result == {
        "title": "Test",
        "thumbnail": "https://x/t.jpg",
        "view_count": 100,
        "channel": "Stedfast",
        "channel_id": "5",
    }
