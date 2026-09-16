//! Seeded Fisher-Yates shuffle.

/// Fisher-Yates shuffle over a copy of `items`, drawing `next(i + 1)` in `0..=i` for `i`
/// from the end down to 1. The caller supplies `next`, so any PRNG can drive the draw.
///
/// # Panics
///
/// Panics if `next(bound)` returns a value `>= bound`, on every target: a
/// draw outside its bound is a broken PRNG contract, not an input to be
/// silently clamped or skipped.
#[must_use]
pub fn seeded_shuffle<T: Clone>(items: &[T], mut next: impl FnMut(u64) -> u64) -> Vec<T> {
    let mut result: Vec<T> = items.to_vec();
    for i in (1..result.len()).rev() {
        let bound = u64::try_from(i + 1).unwrap_or(u64::MAX);
        let draw = next(bound);
        // Checked before the narrowing: on a 32-bit `usize` (wasm32) a draw
        // >= 2^32 would otherwise fail the conversion below and the fallback
        // would silently skip the swap instead of panicking.
        assert!(
            draw < bound,
            "PRNG draw {draw} is outside its bound {bound}"
        );
        // `draw < bound <= len`, so it always fits a `usize`; the fallback is unreachable.
        let swap_idx = usize::try_from(draw).unwrap_or(i);
        result.swap(i, swap_idx);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shuffle_is_a_permutation() {
        let items: Vec<u8> = (0..50).collect();
        let mut s = 7u64;
        let out = seeded_shuffle(&items, |bound| {
            // PCG-style LCG step; wrapping because the product overflows u64.
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1) % (1 << 31);
            s % bound
        });
        let mut sorted = out;
        sorted.sort_unstable();
        assert_eq!(sorted, items);
    }

    #[test]
    fn shuffle_calls_next_with_descending_bounds() {
        let items = [1, 2, 3, 4];
        let mut seen = Vec::new();
        let _ = seeded_shuffle(&items, |bound| {
            seen.push(bound);
            0
        });
        assert_eq!(seen, [4, 3, 2]); // i = 3, 2, 1 -> bound i + 1
    }

    #[test]
    fn shuffle_with_zero_draws_is_a_rotation_by_swaps() {
        // drawing 0 every time swaps each i with 0: [1,2,3] -> swap(2,0)=[3,2,1] -> swap(1,0)=[2,3,1]
        assert_eq!(seeded_shuffle(&[1, 2, 3], |_| 0), vec![2, 3, 1]);
    }

    #[test]
    #[should_panic(expected = "PRNG draw 3 is outside its bound 3")]
    fn shuffle_panics_when_a_draw_reaches_its_bound() {
        // A draw equal to its bound is the first out-of-contract value.
        let _ = seeded_shuffle(&[1, 2, 3], |bound| bound);
    }

    #[test]
    #[should_panic(expected = "is outside its bound")]
    fn shuffle_panics_when_a_draw_exceeds_usize() {
        // A draw no 32-bit `usize` can hold must panic too, not be skipped —
        // the contract check runs before the narrowing conversion.
        let _ = seeded_shuffle(&[1, 2, 3], |_| u64::MAX);
    }

    #[test]
    fn shuffle_of_empty_and_singleton_is_identity() {
        assert_eq!(seeded_shuffle::<u8>(&[], |_| 0), Vec::<u8>::new());
        assert_eq!(seeded_shuffle(&[9], |_| 0), vec![9]);
    }
}
