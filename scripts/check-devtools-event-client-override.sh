#!/usr/bin/env bash
#
# Guards the `@tanstack/devtools-event-client` override in
# crates/rdlp-desktop/pnpm-workspace.yaml.
#
# Why this exists at all. `@tanstack/form-core` (via @tanstack/react-form)
# declares `^0.4.1` for that package, and for a 0.x dependency a caret pins the
# MINOR — so `^0.4.1` means >=0.4.1 <0.5.0 and excludes 0.5.0 outright. 0.5.0 is
# where the `NODE_ENV` no-op guard lives; without it, `FormApi.mount()` registers
# live `form:request-form-force-submit` / `-reset` / `-state` listeners in the
# PRODUCTION bundle — a debug RPC any script in the page can drive. The override
# forces 0.5.x anyway, deliberately outside form-core's declared range.
# Upstream: TanStack/form#2132.
#
# Why it needs a gate rather than a comment. pnpm overrides are an unconditional
# replacement: pnpm neither errors nor warns when the forced version falls
# outside a dependent's declared range. This override is already out of range
# today and resolves silently. So when form-core eventually moves, nothing will
# say so — it will either become redundant (upstream fixed it, and we are
# carrying a workaround for nothing) or start silently DOWNGRADING form-core
# below what it now requires. Both are invisible. This gate makes them loud.
#
# Usage: bash scripts/check-devtools-event-client-override.sh [--self-test]
#
# Exit codes follow the convention of the other gates in this directory:
#   0  override is present, still needed, and still forcing forward
#   1  override has gone stale — redundant, or downgrading form-core
#   2  cannot run (missing files, uninstalled deps, unparseable ranges)
set -euo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 2

DESKTOP="crates/rdlp-desktop"
WORKSPACE="$DESKTOP/pnpm-workspace.yaml"
LOCKFILE="$DESKTOP/pnpm-lock.yaml"
PKG="@tanstack/devtools-event-client"

# ---------------------------------------------------------------------------
# The comparison, factored out so --self-test drives THIS function rather than a
# copy of its logic that can drift away from what the real run does.
#
# Both inputs are caret ranges over a 0.x line. For 0.x, `^0.M.P` permits only
# the 0.M line, so comparing the M is the whole question.
#
# Args: <declared-range> <override-range>
# Prints a verdict; returns 0 (fine), 1 (stale) or 2 (unparseable).
# ---------------------------------------------------------------------------
compare_lines() {
    local declared="$1" override="$2"
    local dmaj dmin omaj omin

    if [[ ! "$declared" =~ ^\^0\.([0-9]+)\. ]]; then
        printf 'CANNOT RUN: form-core declares %s, which is not a ^0.x range.\n' "$declared" >&2
        printf '       This gate only reasons about 0.x caret ranges. If form-core has\n' >&2
        printf '       reached 1.0 or switched range syntax, re-read the override and\n' >&2
        printf '       rewrite this check rather than loosening it.\n' >&2
        return 2
    fi
    dmin="${BASH_REMATCH[1]}"; dmaj=0

    if [[ ! "$override" =~ ^\^0\.([0-9]+)\. ]]; then
        printf 'CANNOT RUN: the override is %s, which is not a ^0.x range.\n' "$override" >&2
        return 2
    fi
    omin="${BASH_REMATCH[1]}"; omaj=0
    : "$dmaj" "$omaj"

    if (( dmin < omin )); then
        printf 'OK: form-core declares ^0.%s.x, override forces ^0.%s.x — still needed.\n' "$dmin" "$omin"
        return 0
    fi

    if (( dmin == omin )); then
        printf 'STALE: form-core now declares ^0.%s.x itself, which the override also\n' "$dmin" >&2
        printf '       forces. The override is REDUNDANT — remove it from\n' >&2
        printf '       %s, drop the revisit note in\n' "$WORKSPACE" >&2
        printf '       %s/CLAUDE.md, and delete this gate.\n' "$DESKTOP" >&2
        return 1
    fi

    printf 'STALE: form-core now declares ^0.%s.x but the override forces ^0.%s.x.\n' "$dmin" "$omin" >&2
    printf '       The override is silently DOWNGRADING form-core below what it\n' >&2
    printf '       requires. Raise or remove it — pnpm will not warn about this.\n' >&2
    return 1
}

# ---------------------------------------------------------------------------
# --self-test: prove the check can reach all three verdicts. A gate that cannot
# fail is not a gate, and the two failure branches here are exactly the states
# nobody will otherwise notice.
# ---------------------------------------------------------------------------
if [ "${1:-}" = "--self-test" ]; then
    fail=0
    run_case() { # <label> <declared> <override> <expected-rc>
        local rc=0
        compare_lines "$2" "$3" >/dev/null 2>&1 || rc=$?
        if [ "$rc" -eq "$4" ]; then
            printf '  ok   %-34s -> rc=%s\n' "$1" "$rc"
        else
            printf '  FAIL %-34s -> rc=%s (expected %s)\n' "$1" "$rc" "$4"
            fail=1
        fi
    }
    printf 'check-devtools-event-client-override --self-test\n'
    run_case "today: declared 0.4 < override 0.5" '^0.4.1' '^0.5.0' 0
    run_case "upstream caught up (redundant)"     '^0.5.0' '^0.5.0' 1
    run_case "upstream moved past (downgrading)"  '^0.6.0' '^0.5.0' 1
    run_case "declared range not a 0.x caret"     '>=1.0.0' '^0.5.0' 2
    run_case "override not a 0.x caret"           '^0.4.1' 'latest' 2
    [ "$fail" -eq 0 ] || exit 1
    # The sentinel check-all.sh greps for, at line start. It requires this
    # literal rather than trusting the exit status, so that a script which
    # merely MENTIONS --self-test, ignores it and exits 0 cannot report a
    # canary it never ran.
    echo "SELF-TEST OK: reaches still-needed, redundant and downgrading verdicts"
    exit 0
fi

# ---------------------------------------------------------------------------
# Real run.
# ---------------------------------------------------------------------------
[ -f "$WORKSPACE" ] || { printf 'CANNOT RUN: %s not found.\n' "$WORKSPACE" >&2; exit 2; }
[ -f "$LOCKFILE" ]  || { printf 'CANNOT RUN: %s not found.\n' "$LOCKFILE" >&2; exit 2; }

# The override. Absent is a legitimate state — someone may have removed it
# because upstream fixed the bug — so say so and pass rather than failing on it.
OVERRIDE="$(sed -n "s/^[[:space:]]*[\"']${PKG//\//\\/}[\"'][[:space:]]*:[[:space:]]*[\"']\([^\"']*\)[\"'].*/\1/p" "$WORKSPACE" | head -1)"
if [ -z "$OVERRIDE" ]; then
    printf 'OK: no %s override present — nothing to guard.\n' "$PKG"
    exit 0
fi

# The RESOLVED form-core version, from the lockfile — never from a node_modules
# glob. The pnpm store keeps older copies after a bump, so a glob picks an
# arbitrary one; that mistake produced a wrong answer three times while this
# override was being investigated.
FC_VERSION="$(sed -n "s/^[[:space:]]*'@tanstack\/form-core@\([0-9][^']*\)':.*/\1/p" "$LOCKFILE" | sort -u | tail -1)"
if [ -z "$FC_VERSION" ]; then
    printf 'CANNOT RUN: could not read a resolved @tanstack/form-core version from %s.\n' "$LOCKFILE" >&2
    exit 2
fi

FC_PKG="$DESKTOP/node_modules/.pnpm/@tanstack+form-core@$FC_VERSION/node_modules/@tanstack/form-core/package.json"
if [ ! -f "$FC_PKG" ]; then
    printf 'CANNOT RUN: %s not found.\n' "$FC_PKG" >&2
    printf '       Run "pnpm install" in %s first.\n' "$DESKTOP" >&2
    exit 2
fi

DECLARED="$(python3 -c "
import json,sys
d=json.load(open('$FC_PKG')).get('dependencies',{})
sys.stdout.write(d.get('$PKG',''))
")"
if [ -z "$DECLARED" ]; then
    printf 'STALE: form-core %s no longer depends on %s at all.\n' "$FC_VERSION" "$PKG" >&2
    printf '       The override has nothing left to override — remove it.\n' >&2
    exit 1
fi

printf 'form-core %s declares %s; override forces %s\n' "$FC_VERSION" "$DECLARED" "$OVERRIDE"
compare_lines "$DECLARED" "$OVERRIDE"
