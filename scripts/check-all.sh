#!/usr/bin/env bash
# Run every invariant gate. Referenced by CODING_RULES.md's Pre-Commit Checklist
# so the gates have a committed local entry point, not only a CI one.
#
# Usage: bash scripts/check-all.sh

set -uo pipefail

# `|| exit` matters here (SC2164): this script runs with `set -uo pipefail` and
# deliberately WITHOUT `-e`, so a failed cd would not abort -- it would run every
# gate against whatever directory the caller happened to be in and report on the
# wrong tree.
cd "$(git rev-parse --show-toplevel)" || exit 2  # 2 = cannot run, 1 = a gate failed

FAILED=0
BLOCKED=0
BLOCKED_GATES=""

for script in scripts/check-*.sh; do
    # Don't recurse into this aggregate.
    [ "$(basename "$script")" = "check-all.sh" ] && continue
    printf '%-44s' "$(basename "$script")"
    # Distinguish the two non-zero cases rather than folding them together.
    # A gate that CANNOT RUN (exit 2 -- a required tool is missing) is not the
    # same news as an invariant being violated (exit 1), and telling an operator
    # "invariants failed" when rg simply isn't installed sends them hunting for
    # a defect that does not exist.
    rc=0
    output=$(bash "$script" 2>&1) || rc=$?
    if [[ $rc -eq 0 ]]; then
        echo "OK"
    elif [[ $rc -eq 2 ]]; then
        echo "CANNOT RUN"
        printf '%s\n' "$output" | sed 's/^/    /'
        BLOCKED=1
        BLOCKED_GATES="$BLOCKED_GATES $script"
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
    rc=0
    output=$(bash "$script" --self-test 2>&1) || rc=$?
    # A canary exiting 2 is only credible as CANNOT RUN if the GATE ITSELF was
    # blocked above. Otherwise a script could mention `--self-test`, ignore it,
    # exit 2, and skip the sentinel assertion entirely -- dodging the anti-spoof
    # property this loop exists for while the gate demonstrably runs fine.
    if [[ $rc -eq 2 ]] && [[ " $BLOCKED_GATES " == *" $script "* ]]; then
        echo "CANNOT RUN"
        printf '%s\n' "$output" | sed 's/^/    /'
        BLOCKED=1
        continue
    fi
    if [[ $rc -eq 0 ]] && printf '%s\n' "$output" | grep -q '^SELF-TEST OK'; then
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

# A real failure outranks a blocked gate: if any invariant is actually violated,
# that is the headline regardless of what else could not run.
if [ "$FAILED" -eq 1 ]; then
    echo ""
    echo "One or more invariant gates failed."
    exit 1
fi

if [ "$BLOCKED" -eq 1 ]; then
    echo ""
    echo "One or more invariant gates COULD NOT RUN (missing tool). Nothing above"
    echo "was reported as violated, but the suite did not fully execute."
    exit 2
fi

echo ""
echo "All invariant gates passed."
