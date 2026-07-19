#!/bin/bash
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
# Usage: scripts/check-no-dir-sweep-delete.sh [--canary]
#   --canary: invert the check -- expects to FIND a violation. Used to prove the
#             gate actually fires; run it against a tree that still has one.

set -euo pipefail

CANARY=0
[ "${1:-}" = "--canary" ] && CANARY=1

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

violations=""
while IFS= read -r file; do
    case "$ALLOWLIST" in *"$file"*) continue ;; esac
    # Production code only: drop everything from the first `#[cfg(test)]` on.
    # Tests legitimately read_dir a TempDir to assert on its contents (e.g.
    # naming.rs's finalize tests), which is not this defect.
    prod=$(sed '/#\[cfg(test)\]/,$d' "$file")
    if printf '%s' "$prod" | grep -qE "$ENUMERATE" \
        && printf '%s' "$prod" | grep -qE "$DELETE"; then
        violations="${violations}${file}\n"
    fi
done < <(find crates/*/src -name '*.rs' -type f 2>/dev/null)

if [ -n "$violations" ]; then
    if [ "$CANARY" -eq 1 ]; then
        echo "CANARY OK: gate fires on:"
        printf "%b" "$violations"
        exit 0
    fi
    echo "ERROR: directory enumeration co-located with file deletion:"
    printf "%b" "$violations"
    echo ""
    echo "Deleting files discovered by a directory scan cannot distinguish"
    echo "'a file rdlp created' from 'a file the user already had' (#558)."
    echo "Delete an exact path this run computed, or use TempRegistry's"
    echo "lock-gated cleanup_stale. See the header of this script."
    exit 1
fi

if [ "$CANARY" -eq 1 ]; then
    echo "CANARY FAILED: gate found nothing -- it would not catch a regression."
    exit 1
fi

echo "OK: No directory-sweep deletion in production code."
