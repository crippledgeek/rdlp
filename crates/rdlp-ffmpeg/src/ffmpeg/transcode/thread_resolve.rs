//! Resolve the concrete encoder thread count for a recode.
//!
//! The auto path (`None`) caps at `AUTO_RECODE_THREADS_CAP`, which matches
//! x265's auto frame-thread table and the VMAF quality sweet-spot, and bounds
//! encoder RAM on a memory-constrained desktop. Explicit values pass through
//! (already range-validated by `Config::validate()`), clamped to >=1.

use std::thread::available_parallelism;

/// Auto-default cap for `recode_threads` when unset. See module docs.
pub const AUTO_RECODE_THREADS_CAP: u32 = 8;

/// Above this resolved thread count, encoders need per-encoder wide-parallelism
/// params (libvvenc tiles/wavefront; libvpx/libaom row-mt) to actually use the
/// extra threads. See `encoder_options::build_video_encoder_options`.
pub const WIDE_PARALLELISM_THRESHOLD: u32 = 8;

/// Fallback when `available_parallelism()` errors (e.g. sandboxed/seccomp).
const PARALLELISM_FALLBACK: u32 = 4;

/// Resolve an `Option<u32>` recode-thread setting to a concrete positive count.
///
/// - `Some(n)` -> `n.max(1)` (range already enforced by `Config::validate`).
/// - `None` -> `min(available_parallelism(), AUTO_RECODE_THREADS_CAP)`,
///   falling back to `PARALLELISM_FALLBACK` if detection fails.
#[must_use]
pub fn resolve_recode_threads(configured: Option<u32>) -> u32 {
    configured.map_or_else(
        || {
            let cores = available_parallelism().map_or(PARALLELISM_FALLBACK, |n| {
                u32::try_from(n.get()).unwrap_or(PARALLELISM_FALLBACK)
            });
            cores.clamp(1, AUTO_RECODE_THREADS_CAP)
        },
        |n| n.max(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_value_passes_through() {
        assert_eq!(resolve_recode_threads(Some(3)), 3);
        // explicit values bypass AUTO_RECODE_THREADS_CAP (already range-validated by Config::validate)
        assert_eq!(resolve_recode_threads(Some(16)), 16);
    }

    #[test]
    fn explicit_zero_is_floored_to_one() {
        assert_eq!(resolve_recode_threads(Some(0)), 1);
    }

    #[test]
    fn auto_is_within_one_to_cap() {
        let n = resolve_recode_threads(None);
        assert!((1..=AUTO_RECODE_THREADS_CAP).contains(&n), "got {n}");
    }
}
