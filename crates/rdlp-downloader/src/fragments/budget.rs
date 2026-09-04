//! List-wide retry budget for the fragment downloader (issue #570).
//!
//! What this does *not* guard against, despite the obvious guess: a
//! systematically broken origin. The fragment stream returns `Err` on the
//! first failing item (`fragments.rs`, the `match item` in the write loop), so
//! a list whose every fragment fails ends the download as soon as the *first*
//! fragment exhausts its own allowance — one backoff ladder, not N of them.
//! Per-fragment retries already bound that case.
//!
//! The case they do not bound is the flaky one: a transfer where fragments
//! fail and then succeed. Nothing there ever propagates an error, so a
//! thousand-fragment list can spend a thousand backoff ladders and still be
//! "making progress" hours later. The budget is a pool of retry attempts
//! shared by the whole list, so a transfer that is limping rather than working
//! gives up in bounded time instead of dragging on.
//!
//! Its size therefore expresses a rate, not a count — see
//! [`RETRY_BUDGET_FRAGMENT_SHARE`].

use std::sync::atomic::{AtomicU64, Ordering};

/// Divisor setting how much cumulative retrying a fragment list may do before
/// it is judged to be limping rather than working.
///
/// The budget is `max_retries x max(fragments / SHARE, 1)`, so dividing
/// through by the fragment count, the list gets `max_retries / SHARE` retries
/// per fragment *on average* — at the default ten retries and a share of ten,
/// exactly one. A transfer where every fragment needs one retry fits; one
/// where every fragment needs two does not, and fails partway rather than
/// taking twice as long as it should.
///
/// That average is the real bound, and it is deliberately far below the
/// per-fragment allowance: the allowance is there for the occasional bad
/// fragment, not for all of them. The number is a judgement, not a
/// measurement — it is the knob to move if real transfers are seen failing on
/// this bound.
const RETRY_BUDGET_FRAGMENT_SHARE: u64 = 10;

/// A pool of retry attempts shared across one fragment list.
///
/// Cloned by reference (`Arc`) into every concurrent fragment task, so the
/// count is authoritative across the whole download rather than per task.
#[derive(Debug)]
pub(crate) struct FragmentRetryBudget {
    remaining: AtomicU64,
}

impl FragmentRetryBudget {
    /// Size a budget for a list of `total_fragments`, given the per-fragment
    /// allowance `max_retries` (the operator's `--fragment-retries`).
    ///
    /// The floor of one fragment's allowance keeps a short list (anything under
    /// `RETRY_BUDGET_FRAGMENT_SHARE` fragments) able to retry at all — without
    /// it, integer division would hand a 5-fragment list a budget of zero.
    pub(crate) fn for_list(max_retries: usize, total_fragments: usize) -> Self {
        let fragments = u64::try_from(total_fragments).unwrap_or(u64::MAX);
        let per_fragment = u64::try_from(max_retries).unwrap_or(u64::MAX);
        let allowances = (fragments / RETRY_BUDGET_FRAGMENT_SHARE).max(1);
        Self {
            remaining: AtomicU64::new(per_fragment.saturating_mul(allowances)),
        }
    }

    /// Take one attempt from the pool, reporting whether one was available.
    ///
    /// `false` means the list has spent its budget and this failure must
    /// propagate. Racing callers cannot both take the last attempt: the
    /// decrement is a compare-and-swap loop, not a read-then-write.
    pub(crate) fn try_consume(&self) -> bool {
        self.remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |left| {
                (left > 0).then(|| left - 1)
            })
            .is_ok()
    }

    /// Attempts still available. Test and diagnostic use only.
    #[cfg(test)]
    pub(crate) fn remaining(&self) -> u64 {
        self.remaining.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_grants_exactly_its_size_and_then_refuses() {
        // Both sides of the boundary: the last attempt inside the budget is
        // granted, the first one past it is not.
        let budget = FragmentRetryBudget::for_list(3, 100); // 3 x (100/10) = 30
        assert_eq!(budget.remaining(), 30);
        for taken in 1..=30 {
            assert!(budget.try_consume(), "attempt {taken} is within the budget");
        }
        assert_eq!(budget.remaining(), 0);
        assert!(
            !budget.try_consume(),
            "the attempt past the budget must be refused"
        );
        assert_eq!(
            budget.remaining(),
            0,
            "a refused attempt must not wrap the counter"
        );
    }

    #[test]
    fn the_average_allowance_per_fragment_is_the_documented_rate() {
        // The property the SHARE constant actually expresses: budget divided by
        // fragment count is max_retries / SHARE, whatever the list length.
        for fragments in [10_usize, 100, 1_000, 10_000] {
            let budget = FragmentRetryBudget::for_list(10, fragments);
            #[allow(
                clippy::cast_possible_truncation,
                reason = "test inputs are small literals"
            )]
            let per_fragment = budget.remaining() / fragments as u64;
            assert_eq!(
                per_fragment, 1,
                "at ten retries and a share of ten, a {fragments}-fragment list \
                 gets one retry per fragment on average"
            );
        }
    }

    #[test]
    fn budget_scales_with_the_fragment_count() {
        assert_eq!(FragmentRetryBudget::for_list(2, 50).remaining(), 10);
        assert_eq!(FragmentRetryBudget::for_list(2, 500).remaining(), 100);
    }

    #[test]
    fn short_list_keeps_one_full_per_fragment_allowance() {
        // Both sides of the floor: below the share divisor the budget is one
        // allowance, at it the proportional term takes over with the same value,
        // and above it the budget grows.
        assert_eq!(FragmentRetryBudget::for_list(3, 1).remaining(), 3);
        assert_eq!(FragmentRetryBudget::for_list(3, 9).remaining(), 3);
        assert_eq!(FragmentRetryBudget::for_list(3, 10).remaining(), 3);
        assert_eq!(FragmentRetryBudget::for_list(3, 20).remaining(), 6);
    }

    #[test]
    fn zero_max_retries_yields_no_budget() {
        let budget = FragmentRetryBudget::for_list(0, 1000);
        assert_eq!(budget.remaining(), 0);
        assert!(!budget.try_consume());
    }

    #[test]
    fn absurd_inputs_saturate_rather_than_overflow() {
        assert_eq!(
            FragmentRetryBudget::for_list(usize::MAX, usize::MAX).remaining(),
            u64::MAX
        );
    }
}
