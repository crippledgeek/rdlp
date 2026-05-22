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

if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <path>" >&2
    exit 1
fi

target="$1"

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

total=0
for f in "${files[@]}"; do
    # Strip #[cfg(test)] mod tests { ... } blocks via awk brace-balance.
    lines=$(awk '
        /#\[cfg\(test\)\][[:space:]]*$/ { in_attr=1; next }
        in_attr && /^[[:space:]]*mod[[:space:]]+/ {
            in_attr=0
            if (/\{[[:space:]]*$/) { depth=1; in_block=1; next }
        }
        in_attr { in_attr=0 }
        in_block {
            n=gsub(/\{/, "{"); depth+=n
            n=gsub(/\}/, "}"); depth-=n
            if (depth==0) { in_block=0 }
            next
        }
        { print }
    ' "$f" | wc -l)
    total=$((total + lines))
    if [[ ${#files[@]} -gt 1 ]]; then
        printf "%5d  %s\n" "$lines" "$f"
    fi
done

if [[ ${#files[@]} -gt 1 ]]; then
    echo "----"
fi
echo "$total  total (non-test LOC)"
