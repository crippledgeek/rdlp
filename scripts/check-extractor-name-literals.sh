#!/usr/bin/env bash
# CI guard (#756): a site extractor's display name is spelled once, in its
# `const NAME`. Every other spelling — a bracketed log tag `"[PornoXO]"`, a
# bare `"PornoXO"` passed to InfoDict::new / format_std_filter_error / name()
# — is a copy nothing keeps in sync (EPorner logged as `[eporner]`, SpankBang
# as `[spankbang]`, before this gate). Textual check; the bracketed form is
# matched exactly, the bare form only for names the registry knows.
#
# NAMES is every registered extractor's `name()`, the Generic fallback
# included: it has no search side, but its `[Generic]` log prefix and six
# `InfoDict::new` sites were spelled by hand too.
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
NAMES='ABXXX|EMPFlix|EPorner|Generic|HQPorner|KoreanPornMovie|MovieFap|9anime|PornHub|PornOne|PornoXO|RedTube|SpankBang|TNAFlix|XHamster|XNXX|XTits|XVideos'

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
        # This trusts the DIRECTORY NAME, not file content: a production
        # `.rs` placed under such a `tests/` directory would go unscanned. No
        # such file exists today (verify with `find
        # crates/rdlp-extractor/src/extractors -path '*/tests/*' -name
        # '*.rs'`); the self-test below asserts every file the trust applies
        # to actually holds a test, so a future production file placed there
        # trips the self-test instead of silently going unscanned.
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
    if [ "$count" -ne 2 ]; then
        echo "SELF-TEST FAILED: gate did not flag both literal shapes (got $count, want 2)"
        exit 1
    fi

    # The `*/tests/*` skip in scan() trusts the directory NAME, not file
    # content (see the comment at its case statement). Assert the trust is
    # currently sound: every real `.rs` file it would skip must be either an
    # actual test file, or a pure `mod` re-export file (e.g. pornhub's
    # `tests/mod.rs`, which only lists `#[cfg(test)] mod extractor;` etc. and
    # carries no string literal of its own to hide). A file that is neither —
    # any other production code — would go unscanned by the real run AND
    # fail this assertion, so the failure is loud here rather than a silent
    # scan gap there.
    while IFS= read -r f; do
        if grep -qE '#\[(tokio::)?test\]' "$f"; then
            continue
        fi
        # Pure declaration file: after stripping comments, blank lines,
        # `#[cfg(test)]` attributes, and `mod <ident>;` lines, nothing remains.
        remainder=$(grep -vE '^\s*(//|#\[cfg\(test\)\]|mod [A-Za-z_][A-Za-z0-9_]*;|\s*$)' "$f" || true)
        if [ -n "$remainder" ]; then
            echo "SELF-TEST FAILED: $f is under a trusted 'tests/' directory but is neither a test file (#[test]/#[tokio::test]) nor a pure mod-declaration file — the */tests/* skip in scan() would silently exempt production code"
            exit 1
        fi
    done < <(find "${SCAN_ROOTS[@]}" -path '*/tests/*' -name '*.rs' -type f)

    echo "SELF-TEST OK"
    exit 0
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
