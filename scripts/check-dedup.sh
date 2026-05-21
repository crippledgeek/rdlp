#!/usr/bin/env bash
# Local pre-push dedup detection gate. See
# docs/superpowers/specs/2026-05-21-code-quality-sprint-design.md (PR A).
#
# Two stages:
#   1. cargo dupes check (--exclude-tests --min-lines 20) — the GATE.
#      AST-shape detector with a per-fingerprint allowlist
#      (.dupes-ignore.toml) for intentional similarity. Fails on any
#      NEW duplicate not in the allowlist.
#   2. similarity-rs (--threshold 0.99 --skip-test) — ADVISORY.
#      Function-pair detector with no allowlist mechanism; reports
#      structural copy-paste for human review but does not gate
#      because the same intentional pairs would be reported forever.
#
# Clippy lints (match_same_arms, redundant_clone, collapsible_match)
# are enforced by the surrounding pre-PR clippy gate (`-D warnings`),
# not here.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

echo "==> cargo dupes check --exclude-tests --min-lines 20 -p crates/"
cargo dupes check --exclude-tests --min-lines 20 -p crates/
echo "    OK"

echo "==> similarity-rs --threshold 0.99 --skip-test crates/  (advisory)"
sim_count=$(
  similarity-rs --threshold 0.99 --skip-test crates/ 2>&1 \
    | grep -cE "^  Similarity: " \
    || true
)
echo "    advisory: $sim_count function-pair(s) at >=99% similarity (no allowlist; review manually if count grows beyond known intentional pairs)"

echo "Dedup gate: PASS"
