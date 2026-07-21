#!/usr/bin/env bash
# CI guard: fail if production code enumerates a directory and deletes what it
# finds there.
#
# Deleting a file you merely FOUND, rather than one you can prove you created,
# is the defect class behind #558: cleanup_leftover_segments swept the user's
# output directory for `{title}*.part{digits}` and unlinked every match. A stray
# `MyShow S01E01.mp4.part1` from wget or aria2 was deleted, and the sweep also
# destroyed rdlp's own resumable chunks immediately before resume detection
# looked for them.
#
# CERT FIO21-C: "the only secure solution is to not create temporary files in
# shared directories" -- and where unavoidable, use unique names plus
# exclusive-open, never a post-hoc scan-and-delete. See also CWE-377, CWE-59.
# No comparable tool (yt-dlp, aria2, curl, wget, rsync) deletes by
# directory-pattern match; each deletes only exact paths it computed itself.
#
# Sanctioned alternatives, both already in-tree:
#   - Delete an exact path this run computed and owns (see orchestrator/naming.rs).
#   - TempRegistry::cleanup_stale (rdlp-postprocess/src/pipeline/registry.rs):
#     marker-scans `.rdlp-tmp-`, requires an fs4 exclusive lock before removing,
#     and applies an age floor -- so a live peer's file is never touched.
#
# ---------------------------------------------------------------------------
# WHAT THIS GATE DOES NOT CATCH -- read before trusting it.
#
# This is a textual co-occurrence check, not dataflow analysis. Known evasions,
# all identified in the #558 security review:
#
#   1. EXISTENCE-PROBE LOOPS. `for i in 0..N { if p.exists() { remove_file(p) } }`
#      never calls read_dir, so it is invisible here -- and that shape is already
#      precedented in this codebase (resume.rs cleanup_old_chunks,
#      parallel.rs cleanup_chunk_files). A #558-style bug rewritten that way
#      would NOT be flagged. This is the most likely real-world evasion.
#   2. CROSS-FILE SPLIT. A helper in module A enumerates; module B deletes what
#      it returns. Neither file trips the co-occurrence test.
#   3. Enumeration via an aliased or re-exported read_dir.
#
# Treat this as a narrow regression guard for the specific shape #558 had, not
# as proof the defect class is eliminated. Broadening it (semgrep dataflow --
# semgrep is already installed later in the same CI job) is tracked separately.
# ---------------------------------------------------------------------------
#
# Usage: scripts/check-no-dir-sweep-delete.sh [--self-test]
#   --self-test: prove the gate still fires, by scanning a synthetic violating
#                file in a temp dir. Runs in CI every time, so "canary-verified"
#                stays true as the tree evolves instead of being a one-off claim.

set -euo pipefail

# Pin the C locale: GNU grep's manual says range expressions like the `[a-z_]`
# classes used below are UNSPECIFIED outside the C locale -- they "might fail to
# match any character". A correctness fix, NOT a speed one (measured: no
# difference). Full quote and rationale in #621.
export LC_ALL=C

# Anchor to the repo root. `crates/*/src` is a relative glob: run from anywhere
# else it expands to nothing, find scans zero files, and the script would
# cheerfully report OK. A gate that passes having scanned nothing is worse than
# no gate, so this is load-bearing, not tidiness.
cd "$(git rev-parse --show-toplevel)" || exit 2

SELF_TEST=0
[ "${1:-}" = "--self-test" ] && SELF_TEST=1

# Allowlist: each entry must state why scanning-then-deleting is sound there.
#
# - rdlp-postprocess/src/pipeline/registry.rs: TempRegistry::cleanup_stale. Not
#   a pattern sweep -- requires the `.rdlp-tmp-` marker AND an fs4 exclusive
#   advisory lock AND an age floor before unlinking, so it provably never
#   removes a live peer's file. This is the sanctioned discovery mechanism.
# - rdlp-cli/src/plugin_cmd.rs and plugin_cmd/build_from_ytdlp.rs: `rdlp plugin
#   remove <name>` deletes one plugin directory the user explicitly named, after
#   validate_plugin_name rejects traversal. User-directed removal of an exact
#   computed path, not artifact reclamation.
ALLOWLIST='crates/rdlp-postprocess/src/pipeline/registry.rs
crates/rdlp-cli/src/plugin_cmd.rs
crates/rdlp-cli/src/plugin_cmd/build_from_ytdlp.rs'

ENUMERATE='read_dir|WalkDir|walkdir|glob::'
DELETE='remove_file|remove_dir'

# Scan one directory tree; echo any offending files. Shared by the real run and
# the self-test so the self-test exercises the SAME matcher, not a copy of it.
scan() {
    local file prod
    while IFS= read -r file; do
        # Exact match, not substring: a substring test would let a new file whose
        # path contains an allowlisted path slip through unexamined.
        if printf '%s\n' "$ALLOWLIST" | grep -Fxq "$file"; then
            continue
        fi
        # Production code only: drop everything from the first `#[cfg(test)]` on.
        # Tests legitimately read_dir a TempDir to assert on its contents (e.g.
        # naming.rs's finalize tests), which is not this defect. This assumes the
        # first `#[cfg(test)]` in a file begins its tail test module -- true
        # across crates/*/src today.
        prod=$(sed '/#\[cfg(test)\]/,$d' "$file")
        if printf '%s' "$prod" | grep -qE "$ENUMERATE" \
            && printf '%s' "$prod" | grep -qE "$DELETE"; then
            printf '%s\n' "$file"
        fi
    done < <(find "$@" -name '*.rs' -type f)
}

# The one place the scan scope is defined. `crates/*/src` deliberately excludes
# `crates/*/tests/` — an integration test has no `#[cfg(test)]` attribute (the
# whole file is test code), so the stripping above would not apply and a
# legitimate TempDir-cleanup test would be flagged as a #558 violation.
SCAN_ROOTS=(crates/*/src)

if [ "$SELF_TEST" -eq 1 ]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    cat > "$tmp/sweep.rs" <<'FIXTURE'
async fn sweep(dir: &Path) {
    let mut entries = tokio::fs::read_dir(dir).await.unwrap();
    while let Ok(Some(e)) = entries.next_entry().await {
        let _ = tokio::fs::remove_file(e.path()).await;
    }
}
FIXTURE
    if [ -n "$(scan "$tmp")" ]; then
        echo "SELF-TEST OK: the gate still detects a directory sweep."
        exit 0
    fi
    echo "SELF-TEST FAILED: the gate did NOT flag a known sweep — it is broken."
    exit 1
fi

# Guard against scanning nothing and calling it a pass. Counted over the SAME
# roots the scan walks, so the reported number is the number actually examined.
file_count=$(find "${SCAN_ROOTS[@]}" -name '*.rs' -type f | wc -l)
if [ "$file_count" -eq 0 ]; then
    echo "ERROR: found no .rs files under crates/*/src — refusing to report OK."
    exit 1
fi

violations=$(scan "${SCAN_ROOTS[@]}")

if [ -n "$violations" ]; then
    echo "ERROR: directory enumeration co-located with file deletion:"
    printf '%s\n' "$violations"
    echo ""
    echo "Deleting files discovered by a directory scan cannot distinguish"
    echo "'a file rdlp created' from 'a file the user already had' (#558)."
    echo "Delete an exact path this run computed, or use TempRegistry's"
    echo "lock-gated cleanup_stale. See the header of this script."
    exit 1
fi

echo "OK: No directory-sweep deletion in production code ($file_count files scanned)."
