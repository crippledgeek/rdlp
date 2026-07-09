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

/// Compile-time coupling: the auto-default cap and the wide-parallelism
/// threshold MUST stay equal. If they diverged, the auto path (`None`) could
/// resolve to a count that silently trips the wide-parallelism encoder params
/// (tiles/wavefront/row-mt) without the operator ever asking for them.
///
/// Uses `assert!(a == b)`, not `assert_eq!` — `assert_eq!` is not
/// const-fn-compatible (it `Debug`-formats operands via a non-const panic
/// path; rust-lang/rust#119826). Build fails here if the two ever diverge, so
/// the invariant is enforced by construction, not by convention.
const _: () = assert!(
    AUTO_RECODE_THREADS_CAP == WIDE_PARALLELISM_THRESHOLD,
    "AUTO_RECODE_THREADS_CAP must equal WIDE_PARALLELISM_THRESHOLD"
);

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
        |n| {
            // Contract: `Config::validate` rejects an explicit 0 upstream, so a
            // `Some(0)` never reaches here in production. The debug-only tripwire
            // makes a contract violation loud in tests/dev; `.max(1)` stays as
            // the production-safe fallback so release builds never emit `0`
            // (libvvenc at 0 threads runs main-thread-only).
            debug_assert!(
                n != 0,
                "resolve_recode_threads got Some(0); Config::validate must reject 0 upstream"
            );
            n.max(1)
        },
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
    #[should_panic(expected = "Config::validate must reject 0 upstream")]
    fn explicit_zero_trips_debug_assert() {
        // Contract violation: a `Some(0)` should never arrive (rejected by
        // `Config::validate`). In debug/test builds the tripwire fires; in
        // release builds `.max(1)` floors it to 1 as a production-safe fallback.
        let _ = resolve_recode_threads(Some(0));
    }

    #[test]
    fn auto_is_within_one_to_cap() {
        let n = resolve_recode_threads(None);
        assert!((1..=AUTO_RECODE_THREADS_CAP).contains(&n), "got {n}");
    }

    #[test]
    fn auto_cap_couples_to_wide_parallelism_threshold() {
        // Runtime companion to the module-scope `const _` compile-time assert:
        // documents the coupling as a named, discoverable test.
        assert_eq!(AUTO_RECODE_THREADS_CAP, WIDE_PARALLELISM_THRESHOLD);
    }
}
