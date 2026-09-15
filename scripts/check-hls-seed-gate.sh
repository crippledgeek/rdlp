#!/usr/bin/env bash
# Run the HLS seed-URL security gate's loopback-rejection tests in the ONE
# build configuration that can compile them.
#
# Why: `crates/rdlp-extractor/tests/hls_seed_gate.rs` asserts that
# `expand_hls_url` refuses loopback seeds (issue #660). That refusal is
# bypassed under the `loopback-test-exemption` cargo feature, which
# rdlp-plugin's and rdlp-api's dev-dependencies enable so THEIR mockito tests
# can drive expansion. Cargo unifies features across a workspace build, so
# under `cargo test --workspace` the feature reaches rdlp-extractor's own
# test build and the two loopback cases are compiled out
# (`#[cfg(not(feature = "loopback-test-exemption"))]`) — the run stays green
# having asserted nothing about them. Only `cargo test -p rdlp-extractor`
# alone exercises them, and nothing else in the gate runs that.
#
# This gate runs exactly that invocation and requires BOTH loopback tests
# to have run and passed: a run in which they were compiled out (a stray
# `--features`, a new dev-dependency on rdlp-extractor enabling the
# exemption for itself) exits 0 from cargo and is REJECTED here.
#
# Usage: scripts/check-hls-seed-gate.sh [--self-test]
#   --self-test: prove the result matcher fires on a real passing line and
#                stays silent on the ignored / absent shapes, against canned
#                lines (no cargo run), and print the literal sentinel.

set -euo pipefail

# Pin the C locale: standing practice across this script family (#621) so
# grep's matching cannot drift with the invoking shell's locale.
export LC_ALL=C

cd "$(git rev-parse --show-toplevel)" || exit 2

# The two cases `hls_seed_gate.rs` gates behind
# `#[cfg(not(feature = "loopback-test-exemption"))]`. Named here, not
# derived from the source, so a rename of either test fails this gate
# loudly instead of quietly dropping it from the check.
REQUIRED_TESTS=(
    "loopback_seed_rejected_in_production_build"
    "https_loopback_seed_rejected_in_production_build"
)

# `cargo test` prints one `test <name> ... ok` line per passing test.
passed() {
    local name="$1" output="$2"
    grep -qE "^test ${name} \.\.\. ok$" <<<"$output"
}

if [ "${1:-}" = "--self-test" ]; then
    for name in "${REQUIRED_TESTS[@]}"; do
        if ! passed "$name" "test ${name} ... ok"; then
            echo "SELF-TEST FAILED: matcher does not fire on a passing line for ${name}" >&2
            exit 1
        fi
        if passed "$name" "test ${name} ... ignored"; then
            echo "SELF-TEST FAILED: matcher fires on an ignored line for ${name}" >&2
            exit 1
        fi
        if passed "$name" "test ${name} ... FAILED"; then
            echo "SELF-TEST FAILED: matcher fires on a failing line for ${name}" >&2
            exit 1
        fi
        if passed "$name" "test result: ok. 3 passed; 0 failed"; then
            echo "SELF-TEST FAILED: matcher fires with the test absent for ${name}" >&2
            exit 1
        fi
    done
    echo "SELF-TEST OK"
    exit 0
fi

command -v cargo >/dev/null 2>&1 || { echo "ERROR: cargo not found -- cannot run this gate." >&2; exit 2; }

# No `--workspace`, no `--features`: the point is the un-unified build.
rc=0
output=$(cargo test -p rdlp-extractor --test hls_seed_gate 2>&1) || rc=$?
if [ "$rc" -ne 0 ]; then
    echo "error: cargo test -p rdlp-extractor --test hls_seed_gate failed (exit $rc):" >&2
    printf '%s\n' "$output" >&2
    exit 1
fi

for name in "${REQUIRED_TESTS[@]}"; do
    if ! passed "$name" "$output"; then
        echo "error: ${name} did not run and pass in an un-unified rdlp-extractor build." >&2
        echo "       The loopback-test-exemption feature has leaked into rdlp-extractor's" >&2
        echo "       own test build, so the HLS seed gate's loopback rejection is untested." >&2
        printf '%s\n' "$output" >&2
        exit 1
    fi
done

echo "ok: HLS seed gate loopback-rejection tests ran and passed in isolation"
