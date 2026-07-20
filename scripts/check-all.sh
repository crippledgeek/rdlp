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

# A gate passing is not the same as a gate still working. A textual matcher can
# rot into a no-op and then report OK forever, so every gate that ships a
# --self-test canary runs it here -- otherwise the local checklist would be
# weaker than CI, which is the asymmetry this aggregate exists to close.
#
# Discovered by scanning for the flag rather than listing scripts by name, so a
# new gate's canary is picked up the same way the gate itself is.
for script in scripts/check-*.sh; do
    [ "$(basename "$script")" = "check-all.sh" ] && continue
    grep -q -- '--self-test' "$script" || continue
    printf '%-44s' "$(basename "$script" .sh) --self-test"
    # Convention: a self-test's sentinel MUST be a hardcoded literal. A gate
    # whose canary echoed scanned file content could otherwise be spoofed by a
    # repo file containing a line starting with the sentinel.
    #
    # Exit status alone is not evidence a canary ran: a gate that merely
    # MENTIONS the flag (in a comment, or referring to a sibling's canary)
    # ignores the argument, performs its ordinary scan and exits 0 -- reporting
    # a green canary that does not exist. Require the sentinel on stdout so
    # "a real canary ran" is observed rather than inferred, and so the failure
    # mode is loud rather than silent.
    if output=$(bash "$script" --self-test 2>&1) \
        && printf '%s\n' "$output" | grep -q '^SELF-TEST OK'; then
        echo "OK"
    else
        echo "FAILED"
        if ! printf '%s\n' "$output" | grep -q '^SELF-TEST OK'; then
            echo "    no 'SELF-TEST OK' sentinel — does this script actually implement --self-test?"
        fi
        printf '%s\n' "$output" | sed 's/^/    /'
        FAILED=1
    fi
done

if [ "$FAILED" -eq 1 ]; then
    echo ""
    echo "One or more invariant gates failed."
    exit 1
fi

echo ""
echo "All invariant gates passed."
