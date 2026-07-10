#!/bin/bash
# Issue-hygiene reconciliation report (read-only; makes no changes).
#
# Surfaces the rot patterns that let issues drift out of sync with the code,
# per issue-tracking-discipline. Run it on demand, or before declaring a sprint
# complete:
#
#   1a. A PR said `Closes/Fixes/Resolves #N` but #N is STILL OPEN — a real bug:
#       auto-close failed (wrong base branch, or manually reopened). Investigate.
#   1b. A PR said `Refs #N` (non-closing) and #N is open — usually intentional
#       (partial/tracking); listed so you can confirm it's still meant to stay open.
#       This is the #390/#391 rot bucket when the work actually shipped.
#   2.  Merged PRs with NO issue keyword at all — the #435 rot: scope merged
#       without ever linking an issue. Link scope going forward.
#   3.  Open issues with no activity in a while — candidates to close or split.
#
# It does NOT close anything — closure is a human/agent judgement call (verify
# the acceptance criteria against the code first). It only points at candidates.
#
# Usage:
#   scripts/audit-issues.sh                 # defaults: 40 recent merged PRs, 90-day stale
#   PR_SCAN=80 STALE_DAYS=60 scripts/audit-issues.sh
#
# Requires: gh (authenticated), jq.

set -euo pipefail

PR_SCAN="${PR_SCAN:-40}"       # how many recently-merged PRs to scan
STALE_DAYS="${STALE_DAYS:-90}" # open-issue inactivity threshold (days)

# Auto-closing keywords (fire on merge to the default branch) vs the linking-
# but-non-closing `Refs`. GitHub's recognized closing verbs: close/closes/closed,
# fix/fixes/fixed, resolve/resolves/resolved.
CLOSING_RE='[Cc]los(e|es|ed)|[Ff]ix(es|ed)?|[Rr]esolv(e|es|ed)'
REFS_RE='[Rr]efs?'

for bin in gh jq; do
  if ! command -v "$bin" >/dev/null 2>&1; then
    echo "error: '$bin' is required but not on PATH." >&2
    exit 1
  fi
done

if ! gh auth status >/dev/null 2>&1; then
  echo "error: 'gh' is not authenticated. Run 'gh auth login'." >&2
  exit 1
fi

echo "=== Issue-hygiene reconciliation (read-only) ==="
echo "Scanning the $PR_SCAN most recently-merged PRs; stale threshold ${STALE_DAYS}d."
echo

# Set of currently-open issue numbers (one lookup, reused below).
open_issues="$(gh issue list --state open --limit 300 --json number --jq '.[].number')"
is_open() { grep -qxF "$1" <<<"$open_issues"; }

# Pull issue numbers that follow a keyword class in a PR body.
refs_for() { grep -oiE "($1)[[:space:]]+#[0-9]+" <<<"$2" | grep -oE '[0-9]+' | sort -u || true; }

bug_lines=""      # 1a: Closes-but-still-open
refs_lines=""     # 1b: Refs-and-open
unlinked_lines="" # 2:  no keyword

while IFS=$'\t' read -r pr_num pr_title pr_body; do
  closing="$(refs_for "$CLOSING_RE" "$pr_body")"
  refs="$(refs_for "$REFS_RE" "$pr_body")"

  if [[ -z "$closing" && -z "$refs" ]]; then
    unlinked_lines+="   PR #${pr_num}  ${pr_title}"$'\n'
    continue
  fi
  while read -r n; do
    [[ -z "$n" ]] && continue
    is_open "$n" && bug_lines+="   PR #${pr_num} said Closes #${n} but #${n} is OPEN  —  ${pr_title}"$'\n'
  done <<<"$closing"
  while read -r n; do
    [[ -z "$n" ]] && continue
    is_open "$n" && refs_lines+="   PR #${pr_num} → Refs #${n} (OPEN)  —  ${pr_title}"$'\n'
  done <<<"$refs"
done < <(gh pr list --state merged --limit "$PR_SCAN" \
           --json number,title,body \
           --jq '.[] | [(.number|tostring), .title, (.body // "" | gsub("[\t\n]"; " "))] | @tsv')

echo "## 1a. PR said Closes/Fixes/Resolves but the issue is STILL OPEN (real bug — investigate)"
echo
[[ -n "$bug_lines" ]] && printf '%s' "$bug_lines" || echo "   (none — every auto-close keyword resolved)"
echo
echo "## 1b. PR said Refs (non-closing) and the issue is open (confirm still meant to stay open)"
echo "   (this is the #390/#391 rot bucket when the work actually shipped)"
echo
[[ -n "$refs_lines" ]] && printf '%s' "$refs_lines" || echo "   (none)"
echo
echo "## 2. Merged PRs with NO issue keyword (the #435 pattern — link scope going forward)"
echo
[[ -n "$unlinked_lines" ]] && printf '%s' "$unlinked_lines" || echo "   (none — every scanned merged PR links an issue)"
echo

echo "## 3. Open issues with no activity in > ${STALE_DAYS} days (review: close, split, or ping)"
echo
gh issue list --state open --limit 300 --json number,title,updatedAt \
  | jq -r --arg days "$STALE_DAYS" '
      (now - ($days | tonumber) * 86400) as $cutoff
      | map(select((.updatedAt | fromdateiso8601) < $cutoff))
      | sort_by(.updatedAt)
      | .[]
      | "   #\(.number)  (\(.updatedAt[0:10]))  \(.title)"
    '
[[ "${PIPESTATUS[1]}" -eq 0 ]] || true
echo
echo "=== end ==="
echo "Note: this report only flags candidates. Verify each issue's acceptance"
echo "criteria against the current code before closing (see issue-tracking-discipline)."
