#!/usr/bin/env bash
# CI guard (#756): a site extractor's display name is spelled once, in its
# `const NAME`. Every other spelling — a bracketed log tag `"[PornoXO]"`, a
# bare `"PornoXO"` passed to InfoDict::new / format_std_filter_error / name()
# — is a copy nothing keeps in sync (EPorner logged as `[eporner]`, SpankBang
# as `[spankbang]`, before this gate). Textual check; the bracketed form is
# matched exactly, the bare form only for names the registry knows.
#
# PornOne is deliberately excluded from NAMES: it is not on `develop` yet
# (open PR #760 / branch `feature/pornone-extractor`). Add it once that PR
# merges.
#
# Usage: scripts/check-extractor-name-literals.sh [--self-test]
set -euo pipefail

# Pin the C locale: GNU grep's manual says range/character-class matching is
# UNSPECIFIED outside the C locale. Correctness, not speed (see #621).
export LC_ALL=C

# Anchor to the repo root, mirroring check-no-dir-sweep-delete.sh: a relative
# glob run from elsewhere would scan zero files and report a false OK.
cd "$(git rev-parse --show-toplevel)" || exit 2

SELF_TEST=0
[ "${1:-}" = "--self-test" ] && SELF_TEST=1

# Names as `name()` returns them (the registry's vocabulary). Keep in sync
# with the `const NAME` declarations; the parity test in
# rdlp-extractor/src/lib.rs guards the InfoExtractor/SearchExtractor pair,
# this list guards the literals.
NAMES='ABXXX|EMPFlix|EPorner|HQPorner|KoreanPornMovie|MovieFap|NineAnime|PornHub|PornoXO|RedTube|SpankBang|TNAFlix|XHamster|XNXX|XTits|XVideos'

BRACKETED='"\[[A-Za-z0-9]+\]'
BARE="\"(${NAMES})\""

# Scan one directory tree; echo any offending `file:line` hits. Shared by the
# real run and the self-test so the self-test exercises the SAME matcher.
#
# Test fixtures legitimately pin the name as a string literal
# (`assert_eq!(e.name(), "PornoXO")`), so production code is scanned
# separately from `#[cfg(test)]` modules, per-file, the same way
# check-no-dir-sweep-delete.sh does it: strip from the first `#[cfg(test)]`
# onward before scanning. This assumes a file's tail test module starts at
# its first `#[cfg(test)]`, true across crates/*/src today.
scan() {
    local file prod bracket_hits bare_hits
    while IFS= read -r file; do
        # A `#[cfg(test)] mod tests;` file declaration (pornhub, tnaflix,
        # eporner, xnxx, xvideos, spankbang, pornoxo) puts entire test files
        # under a `tests/` subdirectory with no `#[cfg(test)]` attribute of
        # their own — the attribute lives on the *declaration*, not in the
        # file. Stripping from the first `#[cfg(test)]` would find none and
        # scan the whole file as production, false-flagging pinned literals
        # like `assert_eq!(e.name(), "PornHub")`. Skip such files outright.
        case "$file" in
        */tests/*) continue ;;
        esac
        prod=$(sed '/#\[cfg(test)\]/,$d' "$file")
        bracket_hits=$(printf '%s\n' "$prod" | grep -nE "$BRACKETED" || true)
        if [ -n "$bracket_hits" ]; then
            printf '%s\n' "$bracket_hits" | sed "s#^#${file}:#"
        fi
        bare_hits=$(printf '%s\n' "$prod" | grep -nE "$BARE" | grep -vE 'const NAME: &str = "' || true)
        if [ -n "$bare_hits" ]; then
            printf '%s\n' "$bare_hits" | sed "s#^#${file}:#"
        fi
    done < <(find "$@" -name '*.rs' -type f)
}

SCAN_ROOTS=(crates/rdlp-extractor/src/extractors)

if [ "$SELF_TEST" -eq 1 ]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    printf 'fn name(&self) -> &str { "RedTube" }\nconst T: &str = "[RedTube]";\n' > "$tmp/x.rs"
    hits=$(scan "$tmp")
    count=$(printf '%s\n' "$hits" | grep -c . || true)
    if [ "$count" -eq 2 ]; then
        echo "SELF-TEST OK"
        exit 0
    fi
    echo "SELF-TEST FAILED: gate did not flag both literal shapes (got $count, want 2)"
    exit 1
fi

file_count=$(find "${SCAN_ROOTS[@]}" -name '*.rs' -type f | wc -l)
if [ "$file_count" -eq 0 ]; then
    echo "ERROR: scanned no files under ${SCAN_ROOTS[*]}"
    exit 2
fi

violations=$(scan "${SCAN_ROOTS[@]}")
if [ -n "$violations" ]; then
    echo "ERROR: extractor name spelled outside its const NAME (#756):"
    printf '%s\n' "$violations"
    exit 1
fi

echo "OK: extractor names single-sourced ($file_count files scanned)."
