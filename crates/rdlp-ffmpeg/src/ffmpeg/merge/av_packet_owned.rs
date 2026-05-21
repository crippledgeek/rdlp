//! RAII owner for a heap-allocated `AVPacket`.
//!
//! Dropping frees the packet via `av_packet_free`, so any `?` early-return
//! from a merge loop releases the allocation deterministically. This is
//! the structural guarantee against the leak that existed when packets were
//! freed only on the success path.
//!
//! Lives in its own submodule so the type can be reused across both the
//! standard (`mod.rs`) and MKV (`mkv_raw_ffi.rs`) merge paths without
//! either file owning the definition.

use ffmpeg_the_third::ffi;

use crate::error::{PostProcessError, Result};

/// RAII owner for a heap-allocated `AVPacket`.
///
/// The pointer is `*mut ffi::AVPacket` (`FFmpeg`'s C-allocated heap packet).
/// Construct via [`AvPacketOwned::alloc`]; the [`Drop`] impl calls
/// `av_packet_free`, which both unrefs and frees the packet.
pub(super) struct AvPacketOwned(*mut ffi::AVPacket);

impl AvPacketOwned {
    /// Allocate a fresh packet via `av_packet_alloc`.
    ///
    /// Returns [`PostProcessError::FFmpegLibraryError`] if `FFmpeg`'s allocator
    /// returns null (effectively OOM — extremely rare in practice).
    pub(super) fn alloc() -> Result<Self> {
        // SAFETY: av_packet_alloc returns either a valid heap AVPacket or null.
        // The null check below guarantees Drop always sees a valid pointer.
        let p = unsafe { ffi::av_packet_alloc() };
        if p.is_null() {
            return Err(PostProcessError::FFmpegLibraryError {
                message: "av_packet_alloc failed".into(),
            });
        }
        Ok(Self(p))
    }

    /// Raw pointer accessor for FFI calls (e.g. `av_read_frame`,
    /// `av_write_frame`, `av_packet_unref`).
    pub(super) const fn as_ptr(&self) -> *mut ffi::AVPacket {
        self.0
    }
}

impl Drop for AvPacketOwned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: self.0 is a valid pointer from av_packet_alloc; av_packet_free
            // both unrefs and frees it, then nulls our local copy via the &mut.
            unsafe { ffi::av_packet_free(&mut self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Allocating a packet succeeds and `as_ptr` returns a non-null handle.
    /// Drop runs at end of scope; this test relies on miri / leak sanitizers
    /// to catch missed frees, but at minimum exercises the alloc path.
    #[test]
    fn av_packet_owned_alloc_returns_non_null() {
        let p = AvPacketOwned::alloc().expect("alloc must succeed");
        assert!(!p.as_ptr().is_null());
        // Drop here frees the packet via av_packet_free.
    }

    /// Two independent allocations are independent — dropping one must not
    /// affect the other (regression guard against accidental aliasing).
    #[test]
    fn av_packet_owned_two_allocations_are_independent() {
        let a = AvPacketOwned::alloc().expect("a");
        let b = AvPacketOwned::alloc().expect("b");
        assert_ne!(a.as_ptr(), b.as_ptr());
    }
}
