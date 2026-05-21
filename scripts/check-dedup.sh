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

echo "==> M1 ratchet: no new \`fn get_<name>(&self...)\` accessors"
# Per CODING_RULES.md M1: accessors use the bare noun (no get_ prefix).
# This grep targets METHODS only — `fn get_<name>(&` — so free functions
# like `get_audio_codec(name)` and `get_thumbnail_url(json_ld)` are
# excluded from M1's scope (M1 governs accessors on types, not free
# functions).
#
# Known exceptions (documented in code or in tracking issues):
#   - state/download_queue.rs::get_job{,_mut}: HashMap-style K/V lookup
#     with a key parameter (M1 exception).
#   - host/cookie_jar.rs::get_cookies: WIT-generated trait impl
#     (cannot rename without breaking the WIT contract).
#   - format/mod.rs::get_filesize: rename pending design decision
#     (would collide with field `filesize`; method has fallback logic
#     to filesize_approx — needs a new name like `resolved_filesize`).
#     Tracked in a follow-up GitHub issue.
#   - host/store_kv.rs::get_blocking: HashMap-style K/V lookup
#     (CODING_RULES.md M1 exception, also referenced in rdlp-plugin
#     module docs).
m1_violations=$(
  grep -rnE 'fn get_[a-z_]+\(&' crates/ \
    | grep -v '/tests/' \
    | grep -vF 'crates/rdlp-desktop/src-tauri/src/state/download_queue.rs' \
    | grep -vF 'crates/rdlp-plugin/src/host/cookie_jar.rs' \
    | grep -vF 'crates/rdlp-plugin/src/host/store_kv.rs' \
    | grep -vF 'crates/rdlp-types/src/format/mod.rs:301' \
    || true
)
if [ -n "$m1_violations" ]; then
  echo "FAIL: forbidden \`fn get_<name>(&self)\` accessor(s) found (Rule M1):"
  echo "$m1_violations"
  echo
  echo "Either rename to drop the get_ prefix, or add a documented exception"
  echo "to scripts/check-dedup.sh with a justification."
  exit 1
fi
echo "    OK"

echo "Dedup gate: PASS"
