#!/usr/bin/env bash
# check-boundary-log.sh — CI gate: verify desktop command handlers cannot bypass
# the boundary-record constructors on AppError.
#
# Two rules, both policing BYPASS of the recording constructors rather than
# presence of a log line -- a gate that tries to prove every failure path
# logged is the "every site must remember" shape that already failed here.
#
#   B1: no literal `AppError::Variant { ... }` construction outside error.rs,
#       within crates/rdlp-desktop/src-tauri/src, excluding #[cfg(test)] blocks.
#       Production code must go through the `AppError::snake_case(...)`
#       constructors in error.rs, which record before returning.
#   B2: no `warn!`/`error!` macro in crates/rdlp-desktop/src-tauri/src/commands/.
#       Terminal records come from the AppError constructors; `debug!`/`info!`
#       breadcrumbs stay allowed.
#
# Exit 0 = PASS.  Exit 1 = FAIL (violations found).  Exit 2 = cannot run.
#
# Usage: bash scripts/check-boundary-log.sh [--self-test]
#
# B1's test exclusion is RANGE-based, not "stop at the first #[cfg(test)]".
# An earlier version of this plan used
#     awk '/#\[cfg\(test\)\]/{exit}'
# which is blind to everything after the first test module -- and
# commands/download.rs proves that matters: cancel_download, remove_job, and
# job_options are production functions that sit AFTER that file's `mod tests`
# block. This gate finds the matching close brace of each #[cfg(test)] block
# by depth-counting and excludes only that range, so production code that
# resumes afterward is still checked.
#
# B2 is scoped to the commands/ DIRECTORY literally, not a glob like
# `**/commands*`. That would also match crates/rdlp-cli/src/commands.rs (a
# FILE, a different crate) which legitimately owns the CLI's own terminal
# `error!` record, and it would need to keep excluding
# crates/rdlp-desktop/src-tauri/src/events.rs's one sanctioned `warn!`
# (outside commands/ already, so untouched here).

set -euo pipefail

export LC_ALL=C

cd "$(git rev-parse --show-toplevel)" || exit 2

require_tool() {
    command -v "$1" >/dev/null 2>&1 && return 0
    printf 'error: %s: required tool %s not found in PATH\n' "${0##*/}" "$1" >&2
    printf '       This gate cannot run, and will NOT report a PASS it did not verify.\n' >&2
    exit 2
}
require_tool grep
require_tool awk
require_tool find

SRC_DIR="crates/rdlp-desktop/src-tauri/src"
ERROR_RS="crates/rdlp-desktop/src-tauri/src/error.rs"
COMMANDS_DIR="crates/rdlp-desktop/src-tauri/src/commands"

if [[ ! -d "$SRC_DIR" ]]; then
    printf 'CANNOT RUN: expected %s to exist.\n' "$SRC_DIR" >&2
    exit 2
fi

# The two patterns, named so --self-test drives the SAME strings the real run
# uses rather than a re-typed copy that can drift.
B1_PATTERN='AppError::[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\{'
B2_PATTERN='(warn|error)!\('

# matches_b1 / matches_b2 — the matcher, as a function of ONE LINE. Both the
# real scan and --self-test's canary call these, so a canary can only pass by
# exercising the actual matcher.
matches_b1() { printf '%s\n' "$1" | grep -qE "$B1_PATTERN"; }
matches_b2() { printf '%s\n' "$1" | grep -qE "$B2_PATTERN"; }

# find_test_ranges FILE — prints "start end" line pairs, one per top-level
# #[cfg(test)] block, found by counting braces from the first '{' seen after
# the attribute to the line where that depth returns to zero. Naive w.r.t.
# strings/comments containing braces, same pragmatic tradeoff the rest of this
# repo's grep-based gates make.
find_test_ranges() {
    awk '
        BEGIN { pending = 0; in_block = 0; depth = 0 }
        /#\[cfg\(test\)\]/ && !in_block { pending = 1; start = NR; next }
        pending {
            n = split($0, chars, "")
            for (i = 1; i <= n; i++) {
                if (chars[i] == "{") depth++
                else if (chars[i] == "}") depth--
            }
            if (depth > 0) { in_block = 1; pending = 0 }
            next
        }
        in_block {
            n = split($0, chars, "")
            for (i = 1; i <= n; i++) {
                if (chars[i] == "{") depth++
                else if (chars[i] == "}") depth--
            }
            if (depth <= 0) {
                print start, NR
                in_block = 0
                depth = 0
            }
        }
    ' "$1"
}

# in_test_range LINENO RANGES — RANGES is find_test_ranges' output, one
# "start end" pair per line.
in_test_range() {
    local lineno="$1" ranges="$2" s e
    while read -r s e; do
        [ -z "$s" ] && continue
        if [ "$lineno" -ge "$s" ] && [ "$lineno" -le "$e" ]; then
            return 0
        fi
    done <<RANGES
$ranges
RANGES
    return 1
}

# check_b1 [TARGET_DIR] — scans TARGET_DIR (default SRC_DIR) for B1
# violations. Prints each as "B1 [file:line]: content" and returns 1 if any
# were found, 0 otherwise. Parameterized so --self-test can point it at a
# synthetic fixture instead of the real tree.
check_b1() {
    local target="${1:-$SRC_DIR}"
    local hits=0
    local file
    while IFS= read -r file; do
        [ "$file" = "$ERROR_RS" ] && continue
        local ranges
        ranges=$(find_test_ranges "$file")
        local lineno content
        while IFS=: read -r lineno content; do
            [ -z "$lineno" ] && continue
            in_test_range "$lineno" "$ranges" && continue
            if matches_b1 "$content"; then
                echo "B1 [$file:$lineno]: $content"
                hits=1
            fi
        done < <(grep -nE "$B1_PATTERN" "$file" || true)
    done < <(find "$target" -name '*.rs' | sort)
    return "$hits"
}

# check_b2 [TARGET_DIR] — scans TARGET_DIR (default COMMANDS_DIR) for B2
# violations. No test-module exclusion: as of this gate's authoring, no
# `warn!`/`error!` call exists anywhere under commands/, test or otherwise.
check_b2() {
    local target="${1:-$COMMANDS_DIR}"
    local hits=0
    local file
    while IFS= read -r file; do
        local lineno content
        while IFS=: read -r lineno content; do
            [ -z "$lineno" ] && continue
            if matches_b2 "$content"; then
                echo "B2 [$file:$lineno]: $content"
                hits=1
            fi
        done < <(grep -nE "$B2_PATTERN" "$file" || true)
    done < <(find "$target" -name '*.rs' | sort)
    return "$hits"
}

SELF_TEST=0
[ "${1:-}" = "--self-test" ] && SELF_TEST=1

fail() {
    echo "SELF-TEST FAILED: $1" >&2
    exit 1
}

if [ "$SELF_TEST" -eq 1 ]; then
    # --self-test: prove each matcher fires on a violation and stays silent
    # on the compliant precedent. A canary that only checks the positive
    # direction cannot tell a working matcher from one that matches
    # everything.
    fixture_bad_b1='    Err(AppError::Internal { message: m })'
    fixture_ok_b1='    Err(AppError::internal(Action::new("x"), e))'
    fixture_bad_b2='    warn!("something went wrong");'
    fixture_ok_b2='    debug!("emitting payload for {job_id}");'

    matches_b1 "$fixture_bad_b1" || fail "B1 canary: matcher did not fire on a literal construction"
    matches_b1 "$fixture_ok_b1" && fail "B1 canary: matcher fired on a constructor call"
    matches_b2 "$fixture_bad_b2" || fail "B2 canary: matcher did not fire on warn!"
    matches_b2 "$fixture_ok_b2" && fail "B2 canary: matcher fired on debug!"

    # Controller Ruling 8's canary: a literal construction placed AFTER a
    # #[cfg(test)] block must still be caught (the earlier "stop at the first
    # #[cfg(test)]" heuristic would miss it), while the construction INSIDE
    # the test block -- there only for an assertion fixture -- must not be
    # reported.
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    mkdir -p "$tmp/src"
    cat >"$tmp/src/canary.rs" <<'FIXTURE'
fn ok_path() -> Result<(), AppError> {
    Err(AppError::internal(Action::new("x"), "boom"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructs_for_assertion_only() {
        let _ = AppError::Internal {
            message: "fixture-only-marker".into(),
        };
    }
}

fn after_the_test_module() -> Result<(), AppError> {
    Err(AppError::Internal {
        message: "bypass-after-test-module".into(),
    })
}
FIXTURE

    # matches_b1 fires on the opening "AppError::Internal {" line, so the
    # canary distinguishes the two constructions by THEIR line numbers:
    # line 11 sits inside the #[cfg(test)] block (must be silent), line 18
    # is the production construction after it (must be reported).
    range_out=0
    range_output=$(check_b1 "$tmp/src") || range_out=$?
    [ "$range_out" -eq 1 ] || fail "range canary: expected check_b1 to report the post-test-module violation"
    case "$range_output" in
        *:18]:*) : ;;
        *) fail "range canary: did not report the violation after the test module (line 18)" ;;
    esac
    case "$range_output" in
        *:11]:*) fail "range canary: reported a construction INSIDE the test module (line 11)" ;;
        *) : ;;
    esac
    rm -rf "$tmp"
    trap - EXIT

    echo "SELF-TEST OK"
    exit 0
fi

FAIL=0
B1_OUT=""
B2_OUT=""
if ! B1_OUT=$(check_b1); then
    FAIL=1
fi
if ! B2_OUT=$(check_b2); then
    FAIL=1
fi

if [ "$FAIL" -eq 0 ]; then
    echo "PASS — no boundary-record bypasses found"
    exit 0
fi

[ -n "$B1_OUT" ] && printf '%s\n' "$B1_OUT"
[ -n "$B2_OUT" ] && printf '%s\n' "$B2_OUT"
echo ""
echo "FIX: construct AppError only via its snake_case constructors in error.rs"
echo "     (B1), and use debug!/info! instead of warn!/error! in commands/ (B2)."
exit 1
