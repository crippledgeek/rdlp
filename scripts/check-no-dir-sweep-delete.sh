#!/bin/bash
# CI guard: fail if orchestrator code enumerates a directory and deletes
# entries by filename pattern.
#
# Deleting a file you merely FOUND, rather than one you can prove you
# created, is the defect class behind #558: cleanup_leftover_segments swept
# the user's output directory for `{title}*.part{digits}` and unlinked every
# match. A stray `MyShow S01E01.mp4.part1` from wget or aria2 was deleted, and
# the sweep also destroyed rdlp's own resumable chunks immediately before
# resume detection looked for them.
#
# CERT FIO21-C: "the only secure solution is to not create temporary files in
# shared directories" -- and where unavoidable, use unique names plus
# exclusive-open, never a post-hoc scan-and-delete. See also CWE-377 and
# CWE-59. No comparable tool (yt-dlp, aria2, curl, wget, rsync) deletes by
# directory-pattern match; every one of them deletes only exact computed
# paths it tracked itself.
#
# The sanctioned alternatives, both already in-tree:
#   - Delete an exact path this run computed and owns (see naming.rs).
#   - TempRegistry::cleanup_stale (rdlp-postprocess/src/pipeline/registry.rs),
#     which marker-scans `.rdlp-tmp-` AND requires an fs4 exclusive lock
#     before removing, so a live peer's file is never touched.
#
# Usage: scripts/check-no-dir-sweep-delete.sh [--canary]
#   --canary: verify the gate actually fires (expects to FIND a violation)

set -euo pipefail

SCOPE="crates/rdlp-api/src/orchestrator"
CANARY=0
[ "${1:-}" = "--canary" ] && CANARY=1

# Find files that BOTH enumerate a directory and remove files. Either alone is
# fine -- listing a directory is harmless, and deleting a computed path is the
# sanctioned pattern. The co-occurrence is what indicates a sweep.
#
# Production code only: everything from the first `#[cfg(test)]` onward is
# dropped before scanning. Tests legitimately read_dir a TempDir to assert on
# its contents (e.g. naming.rs's finalize tests), which is not this defect.
violations=""
while IFS= read -r file; do
    prod=$(sed '/#\[cfg(test)\]/,$d' "$file")
    if printf '%s' "$prod" | grep -q 'read_dir' \
        && printf '%s' "$prod" | grep -q 'remove_file\|remove_dir'; then
        violations="${violations}${file}\n"
    fi
done < <(find "$SCOPE" -name '*.rs' -type f 2>/dev/null)

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

echo "OK: No directory-sweep deletion in $SCOPE."
