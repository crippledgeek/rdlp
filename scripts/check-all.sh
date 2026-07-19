#!/bin/bash
# Run every invariant gate. Referenced by CODING_RULES.md's Pre-Commit Checklist
# so the gates have a committed local entry point, not only a CI one.
#
# Usage: bash scripts/check-all.sh

set -uo pipefail

cd "$(git rev-parse --show-toplevel)"

FAILED=0

for script in scripts/check-*.sh; do
    # Don't recurse into this aggregate.
    [ "$(basename "$script")" = "check-all.sh" ] && continue
    printf '%-44s' "$(basename "$script")"
    if output=$(bash "$script" 2>&1); then
        echo "OK"
    else
        echo "FAILED"
        printf '%s\n' "$output" | sed 's/^/    /'
        FAILED=1
    fi
done

# A gate passing is not the same as a gate still working. The dir-sweep gate is
# a textual matcher that could rot into a no-op, so verify it still fires --
# otherwise the local checklist would be weaker than CI, which is the asymmetry
# this aggregate exists to close.
printf '%-44s' "check-no-dir-sweep-delete --self-test"
if output=$(bash scripts/check-no-dir-sweep-delete.sh --self-test 2>&1); then
    echo "OK"
else
    echo "FAILED"
    printf '%s\n' "$output" | sed 's/^/    /'
    FAILED=1
fi

if [ "$FAILED" -eq 1 ]; then
    echo ""
    echo "One or more invariant gates failed."
    exit 1
fi

echo ""
echo "All invariant gates passed."
