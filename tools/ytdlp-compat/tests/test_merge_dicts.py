"""yt-dlp's `merge_dicts` — None-aware dict merge."""

from rdlp_ytdlp_compat import merge_dicts


def test_two_dicts_merge():
    assert merge_dicts({'a': 1}, {'b': 2}) == {'a': 1, 'b': 2}


def test_later_dict_wins_on_conflict():
    assert merge_dicts({'a': 1}, {'a': 2}) == {'a': 2}


def test_skips_none_values():
    assert merge_dicts({'a': 1}, {'a': None}) == {'a': 1}


def test_empty_first_dict():
    assert merge_dicts({}, {'a': 1}) == {'a': 1}


def test_three_dicts():
    assert merge_dicts({'a': 1}, {'b': 2}, {'c': 3}) == {'a': 1, 'b': 2, 'c': 3}
