#!/usr/bin/env bash
#
# Guards the `@tanstack/devtools-event-client` override in
# crates/rdlp-desktop/pnpm-workspace.yaml.
#
# Why the override exists. `@tanstack/form-core` (via @tanstack/react-form)
# declares `^0.4.1` for that package, and for a 0.x dependency a caret pins the
# MINOR — so `^0.4.1` means >=0.4.1 <0.5.0 and excludes 0.5.0 outright. 0.5.0 is
# where the `NODE_ENV` no-op guard lives; without it, `FormApi.mount()` registers
# live `form:request-form-force-submit` / `-reset` / `-state` listeners in the
# PRODUCTION bundle — a debug RPC any script in the page can drive. The override
# forces 0.5.x anyway, deliberately outside form-core's declared range.
# Upstream: TanStack/form#2132.
#
# Why a gate rather than a comment. pnpm overrides are an unconditional
# replacement: pnpm neither errors nor warns when the forced version falls
# outside a dependent's declared range. The override is already out of range
# today and resolves silently. So when form-core moves, nothing will say so — it
# will either become redundant, leaving a workaround carried for nothing, or
# start silently DOWNGRADING form-core below what it then requires.
#
# PREREQUISITE: this gate reads an installed package, so it needs
# `pnpm install` to have been run in crates/rdlp-desktop. Without it the gate
# reports CANNOT RUN (exit 2), not FAILED — a missing install is not an
# invariant violation.
#
# Usage: bash scripts/check-devtools-event-client-override.sh [--self-test]
#
# Exit codes, per the convention of the sibling gates:
#   0  override present, still needed, still forcing forward
#   1  override has gone stale — redundant, or downgrading form-core
#   2  cannot run (missing tool, missing files, unparseable input)
set -euo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 2

DESKTOP="crates/rdlp-desktop"
WORKSPACE="$DESKTOP/pnpm-workspace.yaml"
LOCKFILE="$DESKTOP/pnpm-lock.yaml"
PKG="@tanstack/devtools-event-client"

# ---------------------------------------------------------------------------
# Extraction, factored into functions so --self-test drives THESE rather than a
# reimplementation. The comparison below is pure arithmetic and the least likely
# part to rot; the parsing is where the fail-open risk lives, so it is the part
# the canary most needs to exercise.
# ---------------------------------------------------------------------------

# Read the override value from the `overrides:` block of a workspace file.
# Scoped to that block so a same-named key under some other section cannot be
# picked up. Prints the value, or nothing if the block or key is absent.
extract_override() {
    local file="$1"
    awk '
        /^overrides:[[:space:]]*$/ { inblock = 1; next }
        /^[^[:space:]#]/          { inblock = 0 }
        inblock                   { print }
    ' "$file" \
    | sed -n 's/^[[:space:]]*"\?'"${PKG//\//\\/}"'"\?[[:space:]]*:[[:space:]]*"\?\([^"[:space:]]*\)"\?[[:space:]]*$/\1/p' \
    | head -1
}

# Read the resolved form-core version from a lockfile.
#
# Deliberately NOT a node_modules glob: the pnpm store keeps older copies after
# a bump, so a glob picks an arbitrary one. That exact mistake produced a wrong
# answer three times while this override was being investigated, including
# reading a range off a stale 1.28.5 directory while 1.33.5 was resolved.
#
# Only bare semver is accepted. pnpm can suffix an entry with its peer context
# (`1.33.5(react@19.0.0)`), which does not match the `.pnpm` directory encoding
# and would build a path that does not exist — better to refuse than to guess.
# More than one distinct version is likewise refused rather than arbitrated.
extract_form_core_version() {
    local file="$1" versions count
    versions="$(sed -n "s/^[[:space:]]*'@tanstack\/form-core@\([0-9][0-9A-Za-z.+-]*\)':.*/\1/p" "$file" | sort -u)"
    [ -n "$versions" ] || return 0
    count="$(printf '%s\n' "$versions" | grep -c .)"
    if [ "$count" -ne 1 ]; then
        printf 'AMBIGUOUS\n'
        return 0
    fi
    printf '%s\n' "$versions"
}

# Compare two caret ranges over a 0.x line. For 0.x, `^0.M.P` permits only the
# 0.M line, so the minor is the whole question.
#
# Args: <declared-range> <override-range>
# Returns 0 (still needed), 1 (stale), 2 (unparseable).
compare_lines() {
    local declared="$1" override="$2" dmin omin

    if [[ ! "$declared" =~ ^\^0\.([0-9]+)\. ]]; then
        printf 'CANNOT RUN: form-core declares %s, which is not a ^0.x range.\n' "$declared" >&2
        printf '       This check only reasons about 0.x caret ranges. If form-core has\n' >&2
        printf '       reached 1.0 or changed range syntax, re-read the override and\n' >&2
        printf '       rewrite this check rather than loosening it.\n' >&2
        return 2
    fi
    dmin="${BASH_REMATCH[1]}"

    if [[ ! "$override" =~ ^\^0\.([0-9]+)\. ]]; then
        printf 'CANNOT RUN: the override is %s, which is not a ^0.x range.\n' "$override" >&2
        return 2
    fi
    omin="${BASH_REMATCH[1]}"

    if (( dmin < omin )); then
        printf 'OK: form-core declares ^0.%s.x, override forces ^0.%s.x — still needed.\n' "$dmin" "$omin"
        return 0
    fi

    if (( dmin == omin )); then
        printf 'STALE: form-core now declares ^0.%s.x itself, which the override also\n' "$dmin" >&2
        printf '       forces. The override is REDUNDANT — remove it from %s\n' "$WORKSPACE" >&2
        printf '       along with the comment above it, and delete this gate.\n' >&2
        return 1
    fi

    printf 'STALE: form-core now declares ^0.%s.x but the override forces ^0.%s.x.\n' "$dmin" "$omin" >&2
    printf '       The override is silently DOWNGRADING form-core below what it\n' >&2
    printf '       requires. Raise or remove it in %s —\n' "$WORKSPACE" >&2
    printf '       pnpm will not warn about this.\n' >&2
    return 1
}

# The override only works because consumers import the package ROOT, which is
# what carries the NODE_ENV switch. 0.5.0 also ships a `/production` subpath
# that deliberately re-exports the REAL client and is never stripped, so a
# single import of it silently defeats the whole mitigation — a failure this
# gate would otherwise sail straight past while checking version ranges.
check_production_subpath() {
    local dir="$1" hits
    hits="$(grep -rlF "$PKG/production" "$dir" 2>/dev/null || true)"
    if [ -n "$hits" ]; then
        printf 'STALE: something imports %s/production, which\n' "$PKG" >&2
        printf '       re-exports the real client and is never stripped from a production\n' >&2
        printf '       build. That defeats the override entirely:\n' >&2
        printf '%s\n' "$hits" | sed 's/^/         /' >&2
        return 1
    fi
    return 0
}

# ---------------------------------------------------------------------------
# --self-test: drives the real extraction functions over fixtures, not just the
# arithmetic. A canary that only exercises the robust half would report green
# while the fragile half rots.
# ---------------------------------------------------------------------------
if [ "${1:-}" = "--self-test" ]; then
    fail=0
    check() { # <label> <actual> <expected>
        if [ "$2" = "$3" ]; then
            printf '  ok   %-46s -> %s\n' "$1" "$2"
        else
            printf '  FAIL %-46s -> %s (expected %s)\n' "$1" "$2" "$3"
            fail=1
        fi
    }
    rc_case() { # <label> <declared> <override> <expected-rc>
        local rc=0
        compare_lines "$2" "$3" >/dev/null 2>&1 || rc=$?
        check "$1" "$rc" "$4"
    }

    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT

    printf 'check-devtools-event-client-override --self-test\n'

    # --- extraction: the half that actually fails open ---
    cat >"$tmp/ws-quoted.yaml" <<'FIXTURE'
allowBuilds:
  cypress: true
overrides:
  "@tanstack/devtools-event-client": "^0.5.0"
FIXTURE
    check "override, quoted" "$(extract_override "$tmp/ws-quoted.yaml")" '^0.5.0'

    # The shape that made the previous version report a green "no override".
    cat >"$tmp/ws-bare.yaml" <<'FIXTURE'
overrides:
  @tanstack/devtools-event-client: ^0.5.0
FIXTURE
    check "override, unquoted YAML" "$(extract_override "$tmp/ws-bare.yaml")" '^0.5.0'

    cat >"$tmp/ws-none.yaml" <<'FIXTURE'
allowBuilds:
  cypress: true
FIXTURE
    check "override genuinely absent" "$(extract_override "$tmp/ws-none.yaml")" ''

    # Same key outside the overrides block must NOT be picked up.
    cat >"$tmp/ws-elsewhere.yaml" <<'FIXTURE'
somethingElse:
  "@tanstack/devtools-event-client": "^9.9.9"
overrides:
  "@tanstack/other": "^1.0.0"
FIXTURE
    check "key outside overrides ignored" "$(extract_override "$tmp/ws-elsewhere.yaml")" ''

    cat >"$tmp/lock-ok.yaml" <<'FIXTURE'
snapshots:
  '@tanstack/form-core@1.33.5':
    dependencies:
      '@tanstack/devtools-event-client': 0.5.0
FIXTURE
    check "lockfile version" "$(extract_form_core_version "$tmp/lock-ok.yaml")" '1.33.5'

    cat >"$tmp/lock-two.yaml" <<'FIXTURE'
snapshots:
  '@tanstack/form-core@1.28.5':
  '@tanstack/form-core@1.33.5':
FIXTURE
    check "two versions refused" "$(extract_form_core_version "$tmp/lock-two.yaml")" 'AMBIGUOUS'

    cat >"$tmp/lock-suffixed.yaml" <<'FIXTURE'
snapshots:
  '@tanstack/form-core@1.33.5(react@19.0.0)':
FIXTURE
    check "peer-suffixed version refused" "$(extract_form_core_version "$tmp/lock-suffixed.yaml")" ''

    # --- the production-subpath trap ---
    mkdir -p "$tmp/src-clean" "$tmp/src-trap"
    printf 'import { EventClient } from "@tanstack/devtools-event-client";\n' >"$tmp/src-clean/a.ts"
    printf 'import { EventClient } from "@tanstack/devtools-event-client/production";\n' >"$tmp/src-trap/a.ts"
    rc=0; check_production_subpath "$tmp/src-clean" >/dev/null 2>&1 || rc=$?
    check "root import allowed" "$rc" "0"
    rc=0; check_production_subpath "$tmp/src-trap" >/dev/null 2>&1 || rc=$?
    check "/production import rejected" "$rc" "1"

    # --- the comparison ---
    rc_case "today: declared 0.4 < override 0.5" '^0.4.1' '^0.5.0' 0
    rc_case "upstream caught up (redundant)"     '^0.5.0' '^0.5.0' 1
    rc_case "upstream moved past (downgrading)"  '^0.6.0' '^0.5.0' 1
    rc_case "declared range not a 0.x caret"     '>=1.0.0' '^0.5.0' 2
    rc_case "override not a 0.x caret"           '^0.4.1' 'latest' 2

    [ "$fail" -eq 0 ] || exit 1
    # The sentinel check-all.sh greps for, at line start. It requires this
    # literal rather than trusting the exit status, so a script that merely
    # MENTIONS --self-test cannot report a canary it never ran.
    echo "SELF-TEST OK: extraction, subpath trap and all three verdicts exercised"
    exit 0
fi

# ---------------------------------------------------------------------------
# Real run.
# ---------------------------------------------------------------------------
command -v node >/dev/null 2>&1 || {
    printf 'CANNOT RUN: node is not on PATH.\n' >&2
    printf '       Needed to read an installed package manifest. A missing tool is\n' >&2
    printf '       not an invariant violation, hence exit 2 rather than a failure.\n' >&2
    exit 2
}
[ -f "$WORKSPACE" ] || { printf 'CANNOT RUN: %s not found.\n' "$WORKSPACE" >&2; exit 2; }
[ -f "$LOCKFILE" ]  || { printf 'CANNOT RUN: %s not found.\n' "$LOCKFILE" >&2; exit 2; }

OVERRIDE="$(extract_override "$WORKSPACE")"
if [ -z "$OVERRIDE" ]; then
    # Absent is a legitimate end state — someone may have removed it because
    # upstream fixed the bug. But an unparseable override is NOT absence, and
    # reporting it as such is how a gate disables itself and still prints OK.
    if grep -qF "$PKG" "$WORKSPACE"; then
        printf 'CANNOT RUN: %s appears in %s but its\n' "$PKG" "$WORKSPACE" >&2
        printf '       override value could not be parsed. Refusing to report "absent".\n' >&2
        exit 2
    fi
    printf 'OK: no %s override present — nothing to guard.\n' "$PKG"
    exit 0
fi

FC_VERSION="$(extract_form_core_version "$LOCKFILE")"
if [ -z "$FC_VERSION" ] || [ "$FC_VERSION" = "AMBIGUOUS" ]; then
    printf 'CANNOT RUN: could not read a single bare @tanstack/form-core version from %s.\n' "$LOCKFILE" >&2
    exit 2
fi

FC_PKG="$DESKTOP/node_modules/.pnpm/@tanstack+form-core@$FC_VERSION/node_modules/@tanstack/form-core/package.json"
if [ ! -f "$FC_PKG" ]; then
    printf 'CANNOT RUN: %s not found.\n' "$FC_PKG" >&2
    printf '       Run "pnpm install" in %s first.\n' "$DESKTOP" >&2
    exit 2
fi

# Path passed as argv, never interpolated into the program text: the version it
# is built from comes from a lockfile, which a dependency ultimately controls.
DECLARED="$(node -e '
const fs = require("node:fs");
const deps = JSON.parse(fs.readFileSync(process.argv[1], "utf8")).dependencies || {};
process.stdout.write(deps[process.argv[2]] || "");
' "$FC_PKG" "$PKG")"

if [ -z "$DECLARED" ]; then
    printf 'STALE: form-core %s no longer depends on %s at all.\n' "$FC_VERSION" "$PKG" >&2
    printf '       The override has nothing left to override — remove it from %s.\n' "$WORKSPACE" >&2
    exit 1
fi

check_production_subpath "$DESKTOP/src"

printf 'form-core %s declares %s; override forces %s\n' "$FC_VERSION" "$DECLARED" "$OVERRIDE"
compare_lines "$DECLARED" "$OVERRIDE"
