#!/usr/bin/env bash
# count-loc.sh — count non-test LOC in a Rust file or directory.
#
# Usage:
#   scripts/count-loc.sh <path-to-file-or-dir>
#
# Strips:
#   - files named tests.rs or *_tests.rs
#   - files under any tests/ directory
#   - #[cfg(test)] mod tests { ... } blocks (heuristic — first-level
#     brace-matched extraction)
#
# Used by reviewers to assess whether a file exceeds the 800-LOC
# audit threshold or the 1200-LOC justify-in-PR threshold per
# CODING_RULES.md's File Cohesion section.

set -euo pipefail

# Pin the C locale: GNU grep's manual says range expressions like the `[a-z_]`
# classes used below are UNSPECIFIED outside the C locale -- they "might fail to
# match any character". A correctness fix, NOT a speed one (measured: no
# difference). Full quote and rationale in #621.
export LC_ALL=C

# gawk specifically: the awk program below uses BEGINFILE/ENDFILE, which are
# gawk extensions. Any other awk would parse them as always-false patterns and
# print `0  total` with exit 0. Exit 2 = cannot run, distinct from a real result.
if ! command -v gawk >/dev/null 2>&1; then
    echo "error: ${0##*/}: requires gawk (uses BEGINFILE/ENDFILE)" >&2
    exit 2
fi

if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <path>" >&2
    exit 1
fi

target="$1"

# A path that does not exist is a caller error, not "zero lines of code". Without
# this, `count-loc.sh /nonexistent` printed `0  total (non-test LOC)` and exited
# 0 -- the same silently-wrong shape this suite exists to eliminate.
if [[ ! -e "$target" ]]; then
    echo "error: ${0##*/}: no such file or directory: $target" >&2
    exit 2
fi

if [[ -f "$target" ]]; then
    files=( "$target" )
else
    mapfile -t files < <(
        find "$target" -name '*.rs' \
            -not -path '*/target/*' \
            -not -path '*/tests/*' \
            ! -name 'tests.rs' \
            ! -name '*_tests.rs'
    )
fi

if [[ ${#files[@]} -eq 0 ]]; then
    echo "0  total (non-test LOC)"
    exit 0
fi

# ONE awk process over every file, rather than `awk | wc -l` per file.
# Measured on crates/rdlp-extractor/src (128 files): 287ms -> 31ms, because
# process-spawn cost dominates the scanning itself at this file count. Totals
# verified identical before and after the change. (An isolated micro-benchmark
# of the same two forms gave 300ms -> 26ms; the numbers above are the ones
# reproducible by running this script, so they are the ones quoted.)
#
# The per-file invocation got its state reset for free by starting a fresh
# process; a single pass must do it explicitly at FNR==1, which is also where
# the previous file's tally is emitted.
per_file=0
[[ ${#files[@]} -gt 1 ]] && per_file=1

gawk -v per_file="$per_file" '
    function emit() {
        if (per_file) printf "%5d  %s\n", count, fname
        total += count
    }

    # BEGINFILE/ENDFILE rather than `FNR == 1`: awk never fires FNR==1 for a
    # ZERO-BYTE file, so a `FNR==1 { emit(); reset }` form silently omits empty
    # files from the per-file listing (the total is unaffected, since they
    # contribute 0).
    #
    # These are GAWK-ONLY, which is why this calls `gawk` explicitly and guards
    # for it above. Under another awk they would parse as ordinary (always
    # false) variable patterns, so nothing would ever be emitted and the script
    # would print `0  total` and exit 0 -- a silently wrong answer, the same
    # fail-open shape this gate suite exists to eliminate.
    BEGINFILE {
        in_attr = 0; in_block = 0; depth = 0; count = 0
        fname = FILENAME
    }
    ENDFILE { emit() }

    /#\[cfg\(test\)\][[:space:]]*$/ { in_attr = 1; next }
    in_attr && /^[[:space:]]*mod[[:space:]]+/ {
        in_attr = 0
        if (/\{[[:space:]]*$/) { depth = 1; in_block = 1; next }
    }
    in_attr { in_attr = 0 }
    in_block {
        n = gsub(/\{/, "{"); depth += n
        n = gsub(/\}/, "}"); depth -= n
        if (depth == 0) { in_block = 0 }
        next
    }
    { count++ }

    END {
        if (per_file) print "----"
        printf "%d  total (non-test LOC)\n", total
    }
' "${files[@]}"
