//! List-wide retry budget for the fragment downloader (issue #570).
//!
//! Per-fragment retries alone do not bound a systematically failing playlist:
//! every one of its fragments would burn its full allowance before the
//! download gave up, so a broken stream fails slowly instead of quickly. The
//! budget is a pool of retry attempts shared by the whole fragment list; when
//! it runs out, the next retryable failure propagates as if it were not
//! retryable and ends the download.

use std::sync::atomic::{AtomicU64, Ordering};

/// Share of a fragment list expected to need a retry on a healthy transfer,
/// expressed as a divisor: the budget is sized as if one fragment in this many
/// spent its full per-fragment allowance.
///
/// A transfer that needs to re-fetch more than a tenth of its fragments is not
/// hitting isolated transient errors — it is talking to a broken origin, and
/// further retrying only delays a failure that is already certain. The number
/// is a judgement, not a measurement; it is the knob to move if real transfers
/// are seen failing on this bound.
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
    /// allowance `max_retries` from the operator's `RetryConfig`.
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
