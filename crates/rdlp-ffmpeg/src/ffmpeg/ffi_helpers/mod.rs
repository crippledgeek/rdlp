//! Unsafe FFI helper functions.
//!
//! These encapsulate all `unsafe` FFI operations that lack safe wrappers
//! in `ffmpeg-the-third`, providing safe call-site signatures. The `unsafe`
//! blocks are limited to these well-documented helpers.
//!
//! # Lint allowances
//!
//! - `clippy::cast_*`: `FFmpeg` codec parameter types use `u32`/`i32`. Conversions
//!   are audited and within valid ranges.
//! - `clippy::redundant_pub_crate`: `pub(crate)` functions are accessed from sibling
//!   modules via `crate::ffmpeg::ffi_helpers::*`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::redundant_pub_crate,
    clippy::similar_names,  // FFI convention: ctx_ptr / dec_ctx / enc_ctx are standard names
)]

pub(crate) mod filter_graph;

use std::path::Path;

use crate::error::Result;

use super::FFmpegRunner;

/// What to do with a stream's copied codec tag for the target container.
///
/// Decided once, by [`FFmpegRunner::resolve_codec_tag`] — the single decision
/// point every stream-copy call site consults, whether through the safe
/// wrapper ([`FFmpegRunner::add_stream_copy`]) or a raw-FFI mux path
/// (`remux_mkv_raw_ffi`, `merge_mkv_raw_ffi`, `embed_thumbnail_mkv_raw_ffi`).
/// `pub(crate)`: those raw-FFI sites live in sibling modules and need to name
/// this type to match on the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodecTagAction {
    /// The muxer's codec-tag table has an entry for this codec: keep the
    /// tag `avcodec_parameters_copy` already copied from the source stream.
    Preserve,
    /// Zero the tag and let `FFmpeg` auto-fill from the muxer's own table.
    ///
    /// Two cases reach here: the muxer isn't tag-driven at all (no
    /// codec-tag table, e.g. MPEG-TS), where zeroing is inert; or the
    /// muxer does have an entry for this codec but under a different tag
    /// than the source's, where preserving would write a tag the muxer's
    /// own validation rejects (AAC `mp4a` into AVI, H.264 `avc1` into FLV).
    Clear,
}

/// Whether `oformat` can represent `codec_id` — the single accept rule every
/// stream-copy decision consults, whether it is *routing* toward a remux
/// ([`crate::ffmpeg::muxer_defaults::muxer_can_represent`]) or *enforcing* one
/// mid-mux ([`FFmpegRunner::resolve_codec_tag`]).
///
/// Sharing this rule is what #630 was about: `RecodeStage::can_remux` routed
/// `hevc → avi` to the remux path on a hand-written codec list, and
/// `resolve_codec_tag` — 200 lines and one FFI boundary later — refused the
/// very same pairing with "avi cannot represent hevc video". The user got a
/// fatal error where a transcode was available.
///
/// **The two are not identical, deliberately.** `resolve_codec_tag` accepts
/// unconditionally when the muxer has no codec-tag table at all
/// (`tags.is_null()`, e.g. `mxfenc`), *before* it ever reaches this rule.
/// Enforcement may not refuse a copy the muxer has given no evidence
/// against; routing may not *choose* one on no evidence. The invariant that
/// matters is the implication, not the equivalence:
///
/// > whenever this predicate says yes, `resolve_codec_tag` does not refuse.
///
/// `routing_never_chooses_a_copy_enforcement_would_refuse` pins exactly that,
/// and asserting equivalence instead would assert a bug (enforcement would
/// start hard-failing `h264 → mxf`).
///
/// Positive evidence only, in the two forms `FFmpeg` offers:
///
/// - the muxer's codec-tag table has an entry for `codec_id`, or
/// - `avformat_query_codec` answers strictly positive.
///
/// `> 0` rather than `!= 0` is load-bearing: per `avformat.h` the call returns
/// `1` (supported), `0` (not supported), or **negative**
/// (`AVERROR_PATCHWELCOME` — "information unavailable"), and the negative case
/// is real, not theoretical. `mxfenc` ships no `query_codec` callback and a
/// null `codec_tag` table, so every codec **other than the ones it declares**
/// (`mpeg2video`, `pcm_s16le`, `eia_608` — `avformat_query_codec`'s
/// declared-defaults branch answers `1` for those) reports
/// `AVERROR_PATCHWELCOME` there. "Unknown" is not evidence a stream copy
/// works, so it takes the same branch as an explicit `0`.
///
/// **Known false negatives are handled one level up, not here.** A muxer can
/// *implement* a codec without *declaring* it through either channel, and this
/// rule reads that as no — correctly, because it has no evidence. `mxfenc` is
/// the live example: it carries a full H.264 essence mapping
/// (`mxf_h264_codec_uls`, `mxf_parse_h264_frame`) and `ffmpeg -c copy` muxes
/// `h264 → mxf` fine, but exposes it via neither channel.
///
/// This function is deliberately left conservative. The curated exceptions
/// live in `muxer_defaults::KNOWN_UNDECLARED_SUPPORT` (#633), consulted by
/// `muxer_can_represent` *after* its media-kind check, so a table entry can
/// never authorise a copy for the wrong medium. Enforcement
/// (`resolve_codec_tag`) is unaffected and keeps its own more-permissive rule,
/// which preserves the implication above — routing now says yes to `h264 →
/// mxf`, and enforcement was already not refusing it.
///
/// A positive answer is not always `1`: `mp3enc`'s `query_codec` returns
/// `MKTAG('A','P','I','C')` for attached-picture codecs, which is why the
/// predicate is `> 0` and not `== 1` (see `mp3_widened_thumbnail_gate.rs`).
pub(crate) fn oformat_can_represent(
    oformat: *const ffmpeg_the_third::ffi::AVOutputFormat,
    codec_id: ffmpeg_the_third::ffi::AVCodecID,
) -> bool {
    // SAFETY: `oformat` is a non-null descriptor from FFmpeg's static muxer
    // registry (never freed). Both calls are pure reads: `av_codec_get_tag`
    // over the muxer's compiled-in tag table (null-checked first), and
    // `avformat_query_codec` over the muxer's own `query_codec` callback or
    // that same table.
    unsafe {
        let tags = (*oformat).codec_tag;
        if !tags.is_null() && ffmpeg_the_third::ffi::av_codec_get_tag(tags, codec_id) != 0 {
            return true;
        }

        ffmpeg_the_third::ffi::avformat_query_codec(
            oformat,
            codec_id,
            ffmpeg_the_third::ffi::FF_COMPLIANCE_NORMAL,
        ) > 0
    }
}

/// Which of the two frame-rate fields a write should actually touch.
///
/// Split out of [`set_stream_frame_rates`] so the threshold is testable
/// without a live `AVStream`: it decides behaviour, and a decision buried
/// behind a raw pointer is a decision no boundary test can reach.
///
/// A non-positive rate yields `None` — the field already holds `0/0`, and
/// writing a nonsense rate would trade a precise muxer refusal for a file
/// asserting a rate its essence does not have. Zero **denominator** matters as
/// much as zero numerator: `av_q2d` on `n/0` is a division by zero, and
/// `mxf_init` inverts the rate (`av_inv_q`) before using it.
///
/// The two fields are decided independently: a source can carry a usable
/// `avg_frame_rate` and a junk `r_frame_rate`, and the good one should still
/// be written.
const fn frame_rates_to_apply(
    avg: ffmpeg_the_third::ffi::AVRational,
    r: ffmpeg_the_third::ffi::AVRational,
) -> (
    Option<ffmpeg_the_third::ffi::AVRational>,
    Option<ffmpeg_the_third::ffi::AVRational>,
) {
    const fn usable(
        rate: ffmpeg_the_third::ffi::AVRational,
    ) -> Option<ffmpeg_the_third::ffi::AVRational> {
        if rate.num > 0 && rate.den > 0 {
            Some(rate)
        } else {
            None
        }
    }

    (usable(avg), usable(r))
}

/// Set the frame-rate fields on an output `AVStream`.
///
/// These live on `AVStream`, **not** in `AVCodecParameters`, so neither
/// `avcodec_parameters_copy` nor `Stream::set_parameters` carries them — an
/// output stream starts at `0/0` unless something sets it explicitly.
///
/// Most muxers infer a workable rate and never notice, which is why the gap
/// survived so long. `mxfenc` does not: `mxf_init` reads `st->avg_frame_rate`,
/// falls back to `st->r_frame_rate`, and **never consults `st->time_base`** —
/// so a stream carrying only a time base yields an edit rate of `0/0` and
/// `mxf_init_timecode` refuses the whole file with "Unsupported frame rate
/// 0/0" at `write_header`. That is #629, on both the transcode and the
/// stream-copy path.
///
/// Non-positive rates are skipped ([`frame_rates_to_apply`]). Note that this
/// only ever *fires* on the stream-copy path: the transcode caller passes
/// `video_ist_frame_rate`, which substitutes a 30/1 default when the source
/// carries no usable rate, so a transcode always writes some rate by
/// construction. The skip is therefore a guarantee about remuxes, not a
/// global one.
///
/// # Safety
///
/// `stream` must be non-null and point to a live `AVStream` belonging to an
/// output context that stays alive for the call. Re-read the pointer after any
/// subsequent `add_stream`: `AVFormatContext::streams` is reallocated as it
/// grows, which invalidates pointers taken before.
pub(crate) unsafe fn set_stream_frame_rates(
    stream: *mut ffmpeg_the_third::ffi::AVStream,
    avg: ffmpeg_the_third::ffi::AVRational,
    r: ffmpeg_the_third::ffi::AVRational,
) {
    let (avg, r) = frame_rates_to_apply(avg, r);

    // SAFETY: the caller guarantees `stream` is non-null, live, and not stale
    // (see `# Safety`). Both writes are of plain `AVRational` values.
    unsafe {
        if let Some(avg) = avg {
            (*stream).avg_frame_rate = avg;
        }
        if let Some(r) = r {
            (*stream).r_frame_rate = r;
        }
    }
}

/// Carry the source stream's frame rates onto the output stream — the
/// stream-copy form of [`set_stream_frame_rates`].
///
/// # Safety
///
/// `dst` carries [`set_stream_frame_rates`]'s contract. `src` must be non-null
/// and point to a live `AVStream` — in practice one borrowed from an open
/// input context — and must not alias `dst`.
pub(crate) unsafe fn copy_stream_frame_rates(
    dst: *mut ffmpeg_the_third::ffi::AVStream,
    src: *const ffmpeg_the_third::ffi::AVStream,
) {
    // SAFETY: the caller guarantees both pointers are live, non-null and
    // non-aliasing (distinct format contexts at the only call site). The read
    // and the write are of plain `AVRational` fields.
    unsafe {
        set_stream_frame_rates(dst, (*src).avg_frame_rate, (*src).r_frame_rate);
    }
}

impl FFmpegRunner {
    /// Add a stream-copy output stream: add stream, copy parameters, resolve
    /// the codec tag for the target container.
    ///
    /// This is the standard pattern for remuxing/metadata/merge where a stream
    /// is copied without re-encoding. Returns the output stream index.
    ///
    /// # Arguments
    /// * `octx` - Output format context
    /// * `params` - Input stream parameters to copy
    /// * `context_msg` - Error context message (e.g., "for remux", "for merge video")
    ///
    /// # Errors
    ///
    /// Returns an error if the target container's muxer cannot represent the
    /// stream's codec at all (see [`Self::resolve_codec_tag`]).
    pub(crate) fn add_stream_copy(
        octx: &mut ffmpeg_the_third::format::context::Output,
        params: impl ffmpeg_the_third::AsPtr<ffmpeg_the_third::ffi::AVCodecParameters>,
        context_msg: &str,
    ) -> Result<usize> {
        use crate::error::PostProcessError;
        use anyhow::Context;

        // Captured before `add_stream` takes a mutable borrow of `octx`.
        // SAFETY: `octx` owns a valid AVFormatContext; `oformat` is set once
        // at context-alloc time (`avformat_alloc_output_context2`) and never
        // changes afterward, so reading it through a short-lived reborrow
        // here is sound regardless of what happens to `octx` later.
        let oformat: *const ffmpeg_the_third::ffi::AVOutputFormat =
            unsafe { (*octx.as_mut_ptr()).oformat };

        let mut ost = octx
            .add_stream(ffmpeg_the_third::encoder::find(
                ffmpeg_the_third::codec::Id::None,
            ))
            .map_err(PostProcessError::from)
            .context(format!("failed to add output stream {context_msg}"))?;
        ost.set_parameters(params);

        let params_ptr = ost.parameters().as_ptr();
        match Self::resolve_codec_tag(oformat, params_ptr)? {
            CodecTagAction::Preserve => {}
            CodecTagAction::Clear => Self::clear_codec_tag(params_ptr),
        }

        Ok(ost.index())
    }

    /// Decide what to do with a stream's codec tag for the target container
    /// (see [`CodecTagAction`]) — the single decision point `add_stream_copy`
    /// consults; no other call site re-implements this predicate.
    ///
    /// Two distinct questions, two distinct `FFmpeg` APIs — conflating them
    /// is the bug this fix corrects:
    ///
    /// - **"Which tag should this stream be written under?"** —
    ///   `av_codec_get_tag`/`av_codec_get_id` against `oformat.codec_tag`,
    ///   the muxer's own static fourcc table. `ffmpeg-the-third` has no safe
    ///   wrapper for either.
    /// - **"Can this muxer represent the codec at all?"** — `avformat_query_codec`,
    ///   which (per `avformat.c`) dispatches to the muxer's own
    ///   `query_codec` callback when one exists, falling back to the tag
    ///   table only when it doesn't. `matroskaenc` defines `mkv_query_codec`
    ///   and reports HEVC/SubRip as representable even though `mkv`'s
    ///   *tag table* (keyed off `CodecID` strings, not a fourcc) has no
    ///   entry for either — so gating rejection on the tag table alone
    ///   (this fix's first attempt) hard-failed HEVC-into-MKV and any
    ///   subtitled input, an actual regression this predicate must not
    ///   reintroduce. `avienc` defines no `query_codec`, so AVI genuinely
    ///   falls back to its tag table and HEVC-into-AVI stays rejected.
    ///
    /// Decision:
    ///
    /// - The muxer has no codec-tag table at all (`oformat.codec_tag` is
    ///   null, e.g. MPEG-TS): tag-clearing is inert either way ->
    ///   [`CodecTagAction::Clear`].
    /// - The table has an entry for `codec_id`, and the source's specific
    ///   tag maps back to that same `codec_id` (`av_codec_get_id` round-trips)
    ///   -> [`CodecTagAction::Preserve`]. Zeroing here would make `FFmpeg`
    ///   auto-fill the tag from the muxer's own table, which for AVI
    ///   substitutes a literal `'H264'` fourcc — that value arms
    ///   `avienc.c`'s start-code guard and hard-fails AVCC-packaged H.264
    ///   (no start codes in the bitstream).
    /// - The table has an entry for `codec_id`, but under a *different* tag
    ///   than the source's -> [`CodecTagAction::Clear`]. Preserving would
    ///   write a tag the muxer's own validation rejects outright (observed
    ///   for AAC's `mp4a` into AVI, and H.264's `avc1` into FLV); zeroing
    ///   lets `FFmpeg` auto-fill the muxer's own preferred tag instead.
    /// - The table has **no** entry for `codec_id`: fall back to
    ///   `avformat_query_codec` (representability, not tag choice). Before
    ///   consulting it, streams whose medium is not Video/Audio/Subtitle
    ///   (Attachment, Data, Unknown — fonts, timed ID3, generic binary
    ///   blobs) are never eligible for rejection at all: `Err` here means
    ///   "this `FFmpeg` build cannot decode/play the codec in this
    ///   container", a question that only applies to playable media
    ///   streams. An attachment's "codec" (e.g. `ttf`) is never decoded —
    ///   it is opaque payload the muxer stores verbatim — so `mkv`'s
    ///   `query_codec` reporting it unsupported is not a representability
    ///   gap, it is simply not a question `query_codec`'s table was built
    ///   to answer for that medium. Rejecting these unconditionally broke
    ///   `salvage_remux_sync` (the container-corruption recovery path of
    ///   last resort, which copies every stream with no medium filter by
    ///   design) on any input carrying a font attachment — the standard
    ///   layout for a subtitled release. These streams resolve to
    ///   [`CodecTagAction::Clear`] (the pre-branch behavior: no tag-table
    ///   entry to preserve or auto-fill from, so clearing is inert).
    /// - Otherwise (Video/Audio/Subtitle), if `avformat_query_codec` reports
    ///   the codec is supported (MKV+HEVC, MKV+SubRip) ->
    ///   [`CodecTagAction::Clear`] — there is no tag-table entry to preserve
    ///   or auto-fill from, so clearing is simply inert here too. If it
    ///   reports unsupported (AVI+HEVC): the pairing cannot be represented
    ///   in this container by this `FFmpeg` build (a convention gap, not a
    ///   documented standard — no spec covers HEVC-in-AVI). Reject before
    ///   writing rather than let a zeroed tag silently decode as something
    ///   else (AVI + HEVC: tag stays 0, `riff.c` maps a zero fourcc to raw
    ///   video — exit 0, corrupt file).
    ///
    /// `avformat_query_codec` itself returns `1` (supported), `0` (not
    /// supported), or negative (`AVERROR_PATCHWELCOME` — information
    /// unavailable), per its header contract in `avformat.h`. Only a
    /// strictly positive result is treated as "supported"
    /// (`query > 0`, not `query != 0`): a negative "unknown" answer is not
    /// evidence the pairing works, so it takes the same conservative,
    /// reject-before-writing branch as an explicit `0`. At every call site
    /// this predicate actually reaches (guarded above by the tag-table-miss
    /// branch), the linked `FFmpeg` 8.0 build's `query_codec` callbacks
    /// (`mkv_query_codec`/`webm_query_codec`) and the C library's own
    /// tag-table fallback in `avformat_query_codec` both only ever return
    /// `0` or `1` here — never negative — so this correction changes no
    /// case in the regression matrix; it is a defensive alignment with the
    /// documented contract for a `query_codec` implementation this build
    /// does not ship.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::PostProcessError::IncompatibleContainerCodec`]
    /// when the muxer cannot represent a Video/Audio/Subtitle stream's codec
    /// under any tag.
    ///
    /// `pub(crate)`: called directly (not through [`Self::add_stream_copy`])
    /// by the raw-FFI mux paths, which build `AVStream`/`AVCodecParameters`
    /// by hand and have no `add_stream_copy` call site to route through.
    pub(crate) fn resolve_codec_tag(
        oformat: *const ffmpeg_the_third::ffi::AVOutputFormat,
        params_ptr: *const ffmpeg_the_third::ffi::AVCodecParameters,
    ) -> Result<CodecTagAction> {
        // SAFETY: `oformat` is the live output context's format descriptor
        // (see the SAFETY note at the `add_stream_copy` call site);
        // `params_ptr` is the just-copied codecpar of the stream we added to
        // that same context, so both reads are into live FFmpeg-owned memory.
        let (codec_id, source_tag, codec_type, tags) = unsafe {
            (
                (*params_ptr).codec_id,
                (*params_ptr).codec_tag,
                (*params_ptr).codec_type,
                (*oformat).codec_tag,
            )
        };

        if tags.is_null() {
            return Ok(CodecTagAction::Clear);
        }

        // SAFETY: `tags` was just proven non-null; both lookups are pure
        // reads over the muxer's static, compiled-in tag table.
        let (muxer_tag_for_codec, id_for_source_tag) = unsafe {
            (
                ffmpeg_the_third::ffi::av_codec_get_tag(tags, codec_id),
                ffmpeg_the_third::ffi::av_codec_get_id(tags, source_tag),
            )
        };

        if muxer_tag_for_codec != 0 {
            return Ok(if id_for_source_tag == codec_id {
                // The source's specific tag round-trips to this codec: keep it.
                CodecTagAction::Preserve
            } else {
                // Codec supported, but not under the source's specific tag;
                // let FFmpeg auto-fill its own preferred tag instead.
                CodecTagAction::Clear
            });
        }

        // No tag-table entry. Attachment/Data/Unknown streams are opaque
        // payload, never decoded, so representability (which only applies
        // to playable Video/Audio/Subtitle media) is not a question that
        // can reject them — see the doc comment above.
        if !matches!(
            codec_type,
            ffmpeg_the_third::ffi::AVMediaType::AVMEDIA_TYPE_VIDEO
                | ffmpeg_the_third::ffi::AVMediaType::AVMEDIA_TYPE_AUDIO
                | ffmpeg_the_third::ffi::AVMediaType::AVMEDIA_TYPE_SUBTITLE
        ) {
            return Ok(CodecTagAction::Clear);
        }

        // Ask representability, not tag choice — through the shared rule, so
        // this enforcement point and the routing predicate that decides
        // whether to remux at all stay in step (#630; the one deliberate
        // asymmetry is documented on that function).
        //
        // Note its tag-table branch is inert from here: the `tags.is_null()`
        // early return and the `muxer_tag_for_codec != 0` branch above have
        // already established both that a table exists and that it has no
        // entry for `codec_id`, so only the `avformat_query_codec` half can
        // decide anything at this call site.
        if oformat_can_represent(oformat, codec_id) {
            return Ok(CodecTagAction::Clear);
        }

        Err(Self::unrepresentable_codec_error(
            oformat, codec_id, codec_type,
        ))
    }

    /// Build the [`crate::error::PostProcessError::IncompatibleContainerCodec`]
    /// for a codec/container pairing `resolve_codec_tag` has already decided
    /// is unrepresentable — kept separate so `resolve_codec_tag` stays a pure
    /// decision (the message-building is not part of the predicate).
    fn unrepresentable_codec_error(
        oformat: *const ffmpeg_the_third::ffi::AVOutputFormat,
        codec_id: ffmpeg_the_third::ffi::AVCodecID,
        codec_type: ffmpeg_the_third::ffi::AVMediaType,
    ) -> crate::error::PostProcessError {
        use crate::error::{Medium, PostProcessError};

        // SAFETY: every registered muxer descriptor has a non-null,
        // NUL-terminated `name`.
        let container = unsafe {
            std::ffi::CStr::from_ptr((*oformat).name)
                .to_string_lossy()
                .into_owned()
        };
        // `avcodec_get_name` rather than `codec::Id::from(codec_id).name()`:
        // `codec_id` comes from a remote, attacker-influenced file, and
        // `ffmpeg-the-third`'s `From<AVCodecID>` ends in `_ => unimplemented!()`
        // under its default `non-exhaustive-enums` feature — so any codec the
        // crate's compiled-in table doesn't know (one newer than the binding,
        // or a malformed stream) would panic here. The C API always yields a
        // valid string, falling back to "unknown_codec".
        // SAFETY: `avcodec_get_name` accepts any AVCodecID and is documented to
        // return a static, NUL-terminated string; it never returns NULL.
        let codec = unsafe {
            std::ffi::CStr::from_ptr(ffmpeg_the_third::ffi::avcodec_get_name(codec_id))
                .to_string_lossy()
                .into_owned()
        };

        PostProcessError::IncompatibleContainerCodec {
            container,
            codec,
            medium: Medium::from(codec_type),
        }
    }

    /// Reset the codec tag to 0 for container compatibility.
    ///
    /// Correct to call only for the cases [`CodecTagAction::Clear`] covers
    /// (see [`Self::resolve_codec_tag`]): either the muxer has no codec-tag
    /// table, or it has no entry matching the source's specific tag. Calling
    /// it unconditionally is the bug #549 fixed — for a tag-driven muxer that
    /// *does* recognize the source tag, zeroing makes `FFmpeg` substitute its
    /// own (for AVI + H.264, the literal `'H264'` fourcc, which then demands
    /// Annex-B start codes that AVCC-packaged streams do not carry).
    pub(crate) fn clear_codec_tag(params_ptr: *const ffmpeg_the_third::ffi::AVCodecParameters) {
        // SAFETY: `params_ptr` points to a valid AVCodecParameters allocated by FFmpeg.
        // Setting codec_tag to 0 is always valid — it tells FFmpeg to auto-select.
        unsafe {
            (*params_ptr.cast_mut()).codec_tag = 0;
        }
    }

    /// Resolve and apply the codec-tag decision for one just-added raw-FFI
    /// output stream — [`Self::resolve_codec_tag`] followed by
    /// [`Self::clear_codec_tag`] on [`CodecTagAction::Clear`], a no-op on
    /// [`CodecTagAction::Preserve`].
    ///
    /// The 4 raw-FFI mux paths (`remux_mkv_raw_ffi`, `merge_mkv_raw_ffi`'s
    /// two streams, `embed_thumbnail_mkv_raw_ffi`) each repeated this
    /// 3-arm `match` verbatim; collapsing the two `Ok` arms here leaves each
    /// call site only its own cleanup (closing input/output contexts) on
    /// `Err`, which differs per site and cannot be folded in without losing
    /// that context-specific teardown.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::PostProcessError::IncompatibleContainerCodec`]
    /// when [`Self::resolve_codec_tag`] rejects the pairing; the caller is
    /// still responsible for its own cleanup before propagating.
    pub(crate) fn resolve_and_apply_codec_tag(
        oformat: *const ffmpeg_the_third::ffi::AVOutputFormat,
        codecpar: *const ffmpeg_the_third::ffi::AVCodecParameters,
    ) -> Result<()> {
        match Self::resolve_codec_tag(oformat, codecpar)? {
            CodecTagAction::Preserve => {}
            CodecTagAction::Clear => Self::clear_codec_tag(codecpar),
        }
        Ok(())
    }

    /// Copy encoder parameters back to an output stream.
    ///
    /// After opening an encoder, its parameters (codec, dimensions, sample rate,
    /// etc.) must be copied to the corresponding output stream before writing
    /// the header.
    pub(crate) fn copy_encoder_params_to_stream(
        octx: &mut ffmpeg_the_third::format::context::Output,
        stream_index: usize,
        encoder_ptr: *const ffmpeg_the_third::ffi::AVCodecContext,
    ) {
        // SAFETY: `octx` owns the output context with a valid stream array.
        // `stream_index` was obtained from a stream added to this context.
        // `encoder_ptr` points to a valid, opened encoder context.
        unsafe {
            let stream_ptr = *(*octx.as_mut_ptr()).streams.add(stream_index);
            ffmpeg_the_third::ffi::avcodec_parameters_from_context(
                (*stream_ptr).codecpar,
                encoder_ptr,
            );
        }
    }

    /// Set the default channel layout for the given number of channels.
    pub(crate) fn set_default_channel_layout(
        encoder_ptr: *mut ffmpeg_the_third::ffi::AVCodecContext,
        channels: i32,
    ) {
        // SAFETY: `encoder_ptr` is a valid, pre-open encoder context.
        // `av_channel_layout_default` populates the ch_layout field in-place.
        unsafe {
            ffmpeg_the_third::ffi::av_channel_layout_default(
                &raw mut (*encoder_ptr).ch_layout,
                channels,
            );
        }
    }

    /// Pick a sample rate supported by the encoder codec.
    ///
    /// If the codec accepts any rate (`supported_samplerates` is NULL), returns
    /// `preferred` unchanged.  Otherwise returns `preferred` if it appears in
    /// the supported list, or the nearest supported rate (preferring higher
    /// rates on ties, which naturally selects 48 kHz for a 44.1 kHz source on
    /// libopus).
    pub(crate) fn pick_audio_sample_rate(codec: &ffmpeg_the_third::Codec, preferred: u32) -> u32 {
        // SAFETY: `codec.as_ptr()` returns a valid AVCodec pointer.
        // `supported_samplerates` is a NULL-terminated i32 array (or NULL if
        // the codec accepts any rate).
        unsafe {
            let ptr = codec.as_ptr();
            let rates = (*ptr).supported_samplerates;
            if rates.is_null() {
                return preferred;
            }

            let mut i: isize = 0;
            let mut best: Option<u32> = None;
            let mut best_dist = u32::MAX;
            loop {
                debug_assert!(
                    i < 1000,
                    "FFmpeg rate array iteration exceeded safety limit"
                );
                let rate = *rates.offset(i);
                if rate == 0 {
                    break;
                }
                let rate_u = rate as u32;
                if rate_u == preferred {
                    return preferred;
                }
                let dist = rate_u.abs_diff(preferred);
                if dist < best_dist || (dist == best_dist && rate_u > best.unwrap_or(0)) {
                    best = Some(rate_u);
                    best_dist = dist;
                }
                i += 1;
            }

            best.unwrap_or(preferred)
        }
    }

    /// Enable VBR (variable bitrate) quality mode on an encoder.
    pub(crate) fn set_vbr_quality(
        encoder_ptr: *mut ffmpeg_the_third::ffi::AVCodecContext,
        quality: i32,
    ) {
        // SAFETY: `encoder_ptr` is a valid, pre-open encoder context.
        // Setting QSCALE flag + global_quality is the standard way to enable VBR.
        unsafe {
            (*encoder_ptr).flags |= ffmpeg_the_third::ffi::AV_CODEC_FLAG_QSCALE as i32;
            (*encoder_ptr).global_quality = quality * ffmpeg_the_third::ffi::FF_QP2LAMBDA;
        }
    }

    /// Set the global header flag on an encoder.
    ///
    /// Required when the output format needs codec parameters in the container
    /// header rather than in each packet (e.g., MP4, MKV).
    pub(crate) fn set_global_header_flag(encoder_ptr: *mut ffmpeg_the_third::ffi::AVCodecContext) {
        // SAFETY: `encoder_ptr` is a valid, pre-open encoder context.
        // This flag is required by certain container formats.
        unsafe {
            (*encoder_ptr).flags |= ffmpeg_the_third::ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }

    /// Write thumbnail packets from the thumbnail input to the output context.
    pub(crate) fn write_thumbnail_packets(
        thumb_ictx: &mut ffmpeg_the_third::format::context::Input,
        octx: &mut ffmpeg_the_third::format::context::Output,
        thumb_ist_index: usize,
        thumb_ist_time_base: ffmpeg_the_third::Rational,
        thumb_ost_index: usize,
    ) -> Result<()> {
        use crate::error::PostProcessError;

        let thumb_ost_time_base = octx
            .stream(thumb_ost_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "thumbnail output stream {thumb_ost_index} not found"
                ))
            })?
            .time_base();
        for result in thumb_ictx.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read thumbnail packet: {e}"),
                })?;
            if stream.index() == thumb_ist_index {
                packet.rescale_ts(thumb_ist_time_base, thumb_ost_time_base);
                packet.set_position(-1);
                packet.set_stream(thumb_ost_index);
                packet.write_interleaved(octx).map_err(|e| {
                    PostProcessError::FFmpegLibraryError {
                        message: format!("failed to write thumbnail packet: {e}"),
                    }
                })?;
            }
        }
        Ok(())
    }

    /// Set `AVDISCARD_ALL` on all non-audio streams in an input context.
    ///
    /// Tells the demuxer to skip non-audio packets entirely, avoiding
    /// memory allocation for large video packets during audio-only analysis.
    pub(crate) fn discard_non_audio_streams(
        ictx: &mut ffmpeg_the_third::format::context::Input,
        audio_stream_index: usize,
    ) {
        // SAFETY: `ictx` owns a valid AVFormatContext. Setting `discard` on
        // streams is a standard FFmpeg operation that tells the demuxer to
        // skip packets for those streams.
        unsafe {
            let ctx_ptr = ictx.as_mut_ptr();
            let nb_streams = (*ctx_ptr).nb_streams as usize;
            for i in 0..nb_streams {
                if i != audio_stream_index {
                    let stream = *(*ctx_ptr).streams.add(i);
                    (*stream).discard = ffmpeg_the_third::ffi::AVDiscard::AVDISCARD_ALL;
                }
            }
        }
    }

    /// Set the frame size on an audio buffersink filter.
    ///
    /// Tells the buffersink to output exactly `frame_size` samples per frame.
    /// The last frame at EOF is automatically zero-padded. This is the proper
    /// way to feed fixed-frame-size encoders (AAC=1024, MP3=1152, Opus=960,
    /// FLAC=4608 at 44.1/48 kHz — `flacenc` sets `frame_size` from its
    /// `max_blocksize`, itself `select_blocksize(sample_rate, 105 ms)`, so the
    /// FLAC figure is rate-dependent and smaller at low sample rates).
    ///
    /// No-op if `frame_size` is 0. That means either the encoder accepts
    /// variable-length frames (`AV_CODEC_CAP_VARIABLE_FRAME_SIZE` — the
    /// `pcm_*` family) or there is no encoder at all, as in a
    /// measurement-only analysis graph. FLAC is **not** in that group: it
    /// reports 4608 and rejects anything else, which is why
    /// `--audio-format=flac` was broken until #638.
    /// Private on purpose: the only legitimate caller is
    /// [`super::filter_graph::build_audio_filter_graph`], a child module, which
    /// applies it as part of building the graph. Leaving it `pub(crate)` would
    /// let a future author elsewhere in the crate hand-roll around the builder
    /// and reintroduce #638 — the omission this pairing exists to prevent.
    fn set_buffersink_frame_size(
        graph: &mut ffmpeg_the_third::filter::Graph,
        sink_name: &str,
        frame_size: u32,
    ) {
        if frame_size == 0 {
            return;
        }

        let Ok(name_c) = std::ffi::CString::new(sink_name) else {
            return;
        };

        // SAFETY: `graph` owns a valid filter graph. `avfilter_graph_get_filter`
        // retrieves the named context. `av_buffersink_set_frame_size` sets the
        // min/max sample counts on the sink's input link.
        unsafe {
            let ctx = ffmpeg_the_third::ffi::avfilter_graph_get_filter(
                graph.as_mut_ptr(),
                name_c.as_ptr(),
            );
            if !ctx.is_null() {
                ffmpeg_the_third::ffi::av_buffersink_set_frame_size(ctx, frame_size);
            }
        }
    }

    /// Configure a stream with `ATTACHED_PIC` disposition (for cover art).
    ///
    /// Sets the stream disposition and clears the codec tag. Used for MP4,
    /// FLAC, OGG, and other containers that embed cover art as a video stream
    /// with special disposition.
    pub(crate) fn set_attached_pic_disposition(stream_ptr: *mut ffmpeg_the_third::ffi::AVStream) {
        // SAFETY: `stream_ptr` is a valid output stream pointer from a live
        // output context. Setting disposition and clearing codec_tag configures
        // the stream as cover art.
        unsafe {
            (*stream_ptr).disposition = ffmpeg_the_third::ffi::AV_DISPOSITION_ATTACHED_PIC;
            (*((*stream_ptr).codecpar)).codec_tag = 0;
        }
    }
}

/// Delete a just-created, still-empty output file after a stream-copy setup
/// failure (best-effort; a failure here is not itself propagated).
///
/// `ffmpeg_the_third::format::output(path)` creates/truncates `path` on disk
/// immediately — before a single stream, header, or packet is written. If
/// [`FFmpegRunner::add_stream_copy`] then rejects the pairing, that 0-byte
/// file is left behind looking like a completed-but-empty operation rather
/// than "never ran". Ten call sites across `remux.rs`, `salvage.rs`,
/// `metadata.rs`, `fixup.rs`, `thumbnail/mod.rs`, `merge/mod.rs`, and
/// `transcode/{audio_extract.rs,video_transcode_phases.rs}` share this exact
/// cleanup; a single helper keeps the ten from drifting apart.
///
/// The raw-FFI codec-tag callers (e.g. `merge/mkv_raw_ffi.rs`) call
/// [`FFmpegRunner::resolve_and_apply_codec_tag`] directly, but that helper is
/// NOT a cleanup trigger there: in every raw-FFI mux path, `avio_open` runs
/// AFTER the per-stream loop that calls it, so no output file exists yet at
/// the point a rejection could occur — there is nothing to clean up.
///
/// Deletes exactly the caller-supplied `path` — never a directory sweep
/// (rdlp #558's lesson: a directory-wide cleanup can delete a concurrent
/// operation's live output).
pub(crate) fn cleanup_partial_output(path: &Path) {
    // Safe: sync FFmpeg wrapper — all callers invoke via spawn_blocking from
    // async boundaries (see rdlp-ffmpeg/src/ffmpeg/mod.rs spawn_blocking helper).
    #[allow(clippy::disallowed_methods)]
    let _ = std::fs::remove_file(path);
}

/// Release packet buffer references. Idempotent — safe on empty packets.
///
/// SAFETY: `Packet::as_ptr()` returns a valid, non-null `AVPacket` pointer
/// owned by the Rust wrapper. `av_packet_unref` only zeroes internal fields.
pub fn packet_unref(pkt: &mut ffmpeg_the_third::Packet) {
    use ffmpeg_the_third::packet::Mut;
    unsafe { ffmpeg_the_third::ffi::av_packet_unref(pkt.as_mut_ptr()) }
}

/// Release audio frame buffer references. Idempotent — safe on empty frames.
///
/// Calling this after `filter.source().add(&frame)` and after
/// `encoder.send_frame(&frame)` releases our reference immediately,
/// reducing peak memory when the filter/encoder also holds a ref.
pub fn frame_unref_audio(frame: &mut ffmpeg_the_third::frame::Audio) {
    unsafe { ffmpeg_the_third::ffi::av_frame_unref((*frame).as_mut_ptr()) }
}

/// Release video frame buffer references. Idempotent — safe on empty frames.
pub fn frame_unref_video(frame: &mut ffmpeg_the_third::frame::Video) {
    unsafe { ffmpeg_the_third::ffi::av_frame_unref((*frame).as_mut_ptr()) }
}

/// Force single-threaded operation on a codec context.
///
/// Must be called **before** `avcodec_open2` (i.e. before `.audio()?` or
/// `.open_as()`). Setting `thread_count = 1` causes `FFmpeg`'s
/// `validate_thread_parameters()` to set `active_thread_type = 0`, which
/// disables both frame threading and slice threading. This eliminates:
/// - Frame threading's per-thread decode buffer pre-allocation (N × `frame_size`)
/// - Slice threading's per-slice scratch buffers
///
/// For audio normalization paths this is the primary RSS reduction knob:
/// the default auto-threading can allocate hundreds of MB in decode/encode
/// buffers that are unnecessary for a single-stream sequential pipeline.
///
/// # Safety
///
/// `ctx_ptr` must point to a valid, **unopened** `AVCodecContext`.
pub fn set_single_thread_codec(ctx_ptr: *mut ffmpeg_the_third::ffi::AVCodecContext) {
    // SAFETY: caller guarantees ctx_ptr is valid and unopened.
    // Setting thread_count before open is the documented way to control threading.
    unsafe {
        (*ctx_ptr).thread_count = 1;
    }
}

/// Read `thread_count` and `active_thread_type` from an opened codec context.
///
/// Returns `(thread_count, active_thread_type)` for diagnostic logging.
///
/// # Safety
///
/// `ctx_ptr` must point to a valid, opened `AVCodecContext`.
pub const fn codec_threading_info(
    ctx_ptr: *const ffmpeg_the_third::ffi::AVCodecContext,
) -> (i32, i32) {
    unsafe { ((*ctx_ptr).thread_count, (*ctx_ptr).active_thread_type) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Terser than spelling out the FFI struct in every case below.
    const fn rate(num: i32, den: i32) -> ffmpeg_the_third::ffi::AVRational {
        ffmpeg_the_third::ffi::AVRational { num, den }
    }

    /// A usable rate is written through unchanged — the case #629 needs to
    /// work at all.
    #[test]
    fn a_positive_rate_is_applied() {
        let (avg, r) = frame_rates_to_apply(rate(25, 1), rate(30000, 1001));
        assert_eq!(avg.map(|q| (q.num, q.den)), Some((25, 1)));
        assert_eq!(r.map(|q| (q.num, q.den)), Some((30000, 1001)));
    }

    /// Both sides of the threshold, per field. `1/1` is the smallest usable
    /// rate and must pass; every degenerate neighbour must not. `n/0` matters
    /// as much as `0/n`: `mxf_init` inverts the rate, and `av_q2d` on a zero
    /// denominator divides by zero.
    #[test]
    fn non_positive_rates_are_skipped() {
        for degenerate in [
            rate(0, 0),
            rate(1, 0),
            rate(0, 1),
            rate(-25, 1),
            rate(25, -1),
        ] {
            let (avg, r) = frame_rates_to_apply(degenerate, degenerate);
            assert!(
                avg.is_none() && r.is_none(),
                "{}/{} must be skipped, not written",
                degenerate.num,
                degenerate.den
            );
        }
        let (avg, _) = frame_rates_to_apply(rate(1, 1), rate(1, 1));
        assert_eq!(
            avg.map(|q| (q.num, q.den)),
            Some((1, 1)),
            "1/1 is usable and must be written"
        );
    }

    /// The fields are decided independently: a junk `r_frame_rate` must not
    /// suppress a good `avg_frame_rate`, which is the pairing `mxf_init`
    /// actually reads first.
    #[test]
    fn each_field_is_decided_on_its_own() {
        let (avg, r) = frame_rates_to_apply(rate(25, 1), rate(0, 0));
        assert_eq!(avg.map(|q| (q.num, q.den)), Some((25, 1)));
        assert!(r.is_none());

        let (avg, r) = frame_rates_to_apply(rate(0, 0), rate(25, 1));
        assert!(avg.is_none());
        assert_eq!(r.map(|q| (q.num, q.den)), Some((25, 1)));
    }

    /// IMPORTANT #2 regression guard: the shared 0-byte-output cleanup
    /// helper must actually delete the file it's given.
    #[test]
    fn cleanup_partial_output_deletes_existing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("partial.mkv");
        // Safe: test fixture — creating an empty file synchronously is not
        // on any async-runtime hot path.
        #[allow(clippy::disallowed_methods)]
        std::fs::write(&path, b"").expect("create empty fixture");
        assert!(path.exists());

        cleanup_partial_output(&path);

        assert!(
            !path.exists(),
            "cleanup_partial_output must delete the partial output file"
        );
    }

    /// `remove_file` on an already-absent path must not panic — every call
    /// site invokes this from an error path where a prior step (e.g. a
    /// mid-loop cancellation) may have already removed the file.
    #[test]
    fn cleanup_partial_output_is_idempotent_on_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("never_created.mkv");
        assert!(!path.exists());

        cleanup_partial_output(&path); // must not panic
        cleanup_partial_output(&path); // still must not panic
    }

    #[test]
    fn packet_unref_on_empty_packet() {
        let mut pkt = ffmpeg_the_third::Packet::empty();
        // Should not panic on an empty/zeroed packet
        packet_unref(&mut pkt);
        // Double-unref should also be safe (idempotent)
        packet_unref(&mut pkt);
    }

    #[test]
    fn frame_unref_audio_on_empty_frame() {
        let mut frame = ffmpeg_the_third::frame::Audio::empty();
        frame_unref_audio(&mut frame);
        frame_unref_audio(&mut frame);
    }

    #[test]
    fn frame_unref_video_on_empty_frame() {
        let mut frame = ffmpeg_the_third::frame::Video::empty();
        frame_unref_video(&mut frame);
        frame_unref_video(&mut frame);
    }

    #[test]
    fn pick_sample_rate_opus_rejects_44100() {
        crate::ffmpeg::ensure_init().unwrap();
        let codec = ffmpeg_the_third::encoder::find_by_name("libopus").unwrap();
        // 44100 is not a supported libopus rate; should pick 48000 (nearest).
        let rate = FFmpegRunner::pick_audio_sample_rate(&codec, 44100);
        assert_eq!(rate, 48000, "libopus should resample 44100→48000");
    }

    #[test]
    fn pick_sample_rate_opus_accepts_48000() {
        crate::ffmpeg::ensure_init().unwrap();
        let codec = ffmpeg_the_third::encoder::find_by_name("libopus").unwrap();
        let rate = FFmpegRunner::pick_audio_sample_rate(&codec, 48000);
        assert_eq!(rate, 48000);
    }

    #[test]
    fn pick_sample_rate_aac_accepts_44100() {
        crate::ffmpeg::ensure_init().unwrap();
        if let Some(codec) = ffmpeg_the_third::encoder::find_by_name("aac") {
            // AAC supports 44100; should return it unchanged.
            let rate = FFmpegRunner::pick_audio_sample_rate(&codec, 44100);
            assert_eq!(rate, 44100);
        }
    }

    /// Build a zeroed `AVCodecParameters` naming only `codec_id`/`codec_type` —
    /// the only fields `resolve_codec_tag` reads.
    ///
    /// SAFETY: zero-initializing `AVCodecParameters` mirrors what
    /// `avcodec_parameters_alloc` does internally (`av_mallocz`); every field
    /// this test leaves zeroed (extradata pointers, side-data lists, etc.) is
    /// never read by `resolve_codec_tag`.
    fn fake_params(
        codec_id: ffmpeg_the_third::ffi::AVCodecID,
        codec_type: ffmpeg_the_third::ffi::AVMediaType,
    ) -> ffmpeg_the_third::ffi::AVCodecParameters {
        let mut params: ffmpeg_the_third::ffi::AVCodecParameters = unsafe { std::mem::zeroed() };
        params.codec_id = codec_id;
        params.codec_type = codec_type;
        params
    }

    /// Get the live `AVOutputFormat*` for a freshly-opened output context
    /// targeting the given extension (mirrors the capture in `add_stream_copy`).
    fn oformat_for_extension(
        dir: &std::path::Path,
        ext: &str,
    ) -> *const ffmpeg_the_third::ffi::AVOutputFormat {
        let octx = ffmpeg_the_third::format::output(dir.join(format!("probe.{ext}")))
            .expect("open probe output context");
        // SAFETY: `octx` is a live output context just opened above; `oformat`
        // is a read-only static descriptor set at alloc time.
        unsafe { (*octx.as_ptr()).oformat }
    }

    /// CRITICAL #1/#2 regression guard: MKV's codec-tag table has no entry
    /// for HEVC at all (Matroska keys tags off `CodecID` strings, not a
    /// fourcc table), so the old predicate — reject whenever
    /// `av_codec_get_tag` returns 0 — hard-failed HEVC into MKV even though
    /// `matroskaenc` defines `mkv_query_codec` and demonstrably supports it.
    /// Fails against the unpatched predicate (which returns
    /// `IncompatibleContainerCodec` here).
    /// The property #630 actually rests on, and the only thing that stops the
    /// two decisions drifting apart again: **whenever routing chooses a stream
    /// copy, enforcement must not then refuse it.**
    ///
    /// Stated as an implication, not an equivalence, because the converse is
    /// deliberately false — see `oformat_can_represent`'s doc. Enforcement
    /// additionally accepts anything when the muxer has no tag table at all,
    /// so `mxf` accepts codecs routing would never have picked. Asserting
    /// equivalence here would be asserting a bug.
    #[test]
    fn routing_never_chooses_a_copy_enforcement_would_refuse() {
        use crate::ffmpeg::codec_registry::MediaKind;
        use crate::ffmpeg::muxer_defaults::muxer_can_represent;

        crate::ffmpeg::ensure_init().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");

        // The containers `RecodeStage::can_remux` routes through the shared
        // predicate, against a codec spread covering all three
        // `avformat_query_codec` answers.
        let containers = [
            (rdlp_types::ContainerFormat::Mkv, "mkv"),
            (rdlp_types::ContainerFormat::Nut, "nut"),
            (rdlp_types::ContainerFormat::Mxf, "mxf"),
            (rdlp_types::ContainerFormat::Avi, "avi"),
        ];
        let codecs = [
            ("h264", ffmpeg_the_third::ffi::AVCodecID::AV_CODEC_ID_H264),
            ("hevc", ffmpeg_the_third::ffi::AVCodecID::AV_CODEC_ID_HEVC),
            ("vp9", ffmpeg_the_third::ffi::AVCodecID::AV_CODEC_ID_VP9),
            ("av1", ffmpeg_the_third::ffi::AVCodecID::AV_CODEC_ID_AV1),
            ("mpeg4", ffmpeg_the_third::ffi::AVCodecID::AV_CODEC_ID_MPEG4),
            (
                "mpeg2video",
                ffmpeg_the_third::ffi::AVCodecID::AV_CODEC_ID_MPEG2VIDEO,
            ),
            (
                "prores",
                ffmpeg_the_third::ffi::AVCodecID::AV_CODEC_ID_PRORES,
            ),
        ];

        let mut routed_copies = 0_u32;
        for (container, ext) in containers {
            let oformat = oformat_for_extension(dir.path(), ext);
            for (name, id) in codecs {
                if !muxer_can_represent(
                    container,
                    &rdlp_types::media_name::CodecName::from_static(name),
                    MediaKind::Video,
                ) {
                    continue;
                }
                routed_copies += 1;
                let params =
                    fake_params(id, ffmpeg_the_third::ffi::AVMediaType::AVMEDIA_TYPE_VIDEO);
                assert!(
                    FFmpegRunner::resolve_codec_tag(oformat, std::ptr::from_ref(&params)).is_ok(),
                    "routing would stream-copy {name} into {ext}, but enforcement refuses it — \
                     the two rules have drifted"
                );
            }
        }

        // Guards against the loop passing vacuously if the predicate ever
        // starts answering `false` for everything.
        assert!(
            routed_copies >= 10,
            "expected the matrix to exercise real copies, got {routed_copies}"
        );

        // The #633 allow-list is the one place routing says yes on evidence
        // FFmpeg does not publish, so it is the most likely source of a future
        // drift. Iterate the table itself rather than restating its rows here:
        // the fixed matrix above is video-only and would have missed the
        // `aac → mpegts` row entirely, and any row added later is covered by
        // construction instead of by remembering to extend a list.
        let allow_list = crate::ffmpeg::muxer_defaults::KNOWN_UNDECLARED_SUPPORT;
        assert!(
            !allow_list.is_empty(),
            "the allow-list is empty — this loop would pass vacuously"
        );
        for &(container, codec_id) in allow_list {
            let oformat = oformat_for_extension(dir.path(), container.as_ext());

            // The descriptor gives the row's true medium, so the synthesised
            // params match what a real stream of this codec would carry.
            // SAFETY: pure lookup over FFmpeg's static descriptor table; the
            // returned pointer is null-checked before its fields are read.
            let media_type = unsafe {
                let desc = ffmpeg_the_third::ffi::avcodec_descriptor_get(codec_id);
                assert!(!desc.is_null(), "allow-listed codec has no descriptor");
                (*desc).type_
            };

            let params = fake_params(codec_id, media_type);
            assert!(
                FFmpegRunner::resolve_codec_tag(oformat, std::ptr::from_ref(&params)).is_ok(),
                "KNOWN_UNDECLARED_SUPPORT routes a stream copy into {} that enforcement \
                 refuses — the allow-list has outrun resolve_codec_tag",
                container.as_ext()
            );
        }
    }

    #[test]
    fn resolve_codec_tag_permits_hevc_into_mkv() {
        crate::ffmpeg::ensure_init().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        let oformat = oformat_for_extension(dir.path(), "mkv");
        let params = fake_params(
            ffmpeg_the_third::ffi::AVCodecID::AV_CODEC_ID_HEVC,
            ffmpeg_the_third::ffi::AVMediaType::AVMEDIA_TYPE_VIDEO,
        );

        let action = FFmpegRunner::resolve_codec_tag(oformat, std::ptr::from_ref(&params)).expect(
            "MKV must represent HEVC: matroskaenc defines mkv_query_codec even though \
             its static codec-tag table has no HEVC entry",
        );
        assert_eq!(
            action,
            CodecTagAction::Clear,
            "no tag-table entry for HEVC in mkv -> Clear (not Preserve, not Err)"
        );
    }

    /// Same predicate bug, subtitle-medium variant: MKV's tag table has no
    /// entry for `SubRip` either, but subtitled inputs (routed through
    /// `salvage::salvage_remux_sync`, which copies every stream with no
    /// media-type filter) must still be representable in MKV.
    #[test]
    fn resolve_codec_tag_permits_subrip_into_mkv() {
        crate::ffmpeg::ensure_init().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        let oformat = oformat_for_extension(dir.path(), "mkv");
        let params = fake_params(
            ffmpeg_the_third::ffi::AVCodecID::AV_CODEC_ID_SUBRIP,
            ffmpeg_the_third::ffi::AVMediaType::AVMEDIA_TYPE_SUBTITLE,
        );

        FFmpegRunner::resolve_codec_tag(oformat, std::ptr::from_ref(&params))
            .expect("MKV must represent SubRip subtitles despite no tag-table entry");
    }

    /// Hypothesis check for the raw-FFI routing change: an AAC stream tagged
    /// with its MP4 `mp4a` fourcc, going into MKV, must resolve to
    /// [`CodecTagAction::Clear`] — not `Preserve`. MKV's tag table *does*
    /// carry an AAC entry, but under Matroska's own tag, not `mp4a`, so
    /// `av_codec_get_id(tags, mp4a)` must NOT round-trip to
    /// `AV_CODEC_ID_AAC`. This is the exact case an earlier blunt experiment
    /// (preserving every source tag unconditionally) got wrong: preserving
    /// `mp4a` on Matroska-bound AAC is what regressed `mp4_to_mkv`/
    /// `ts_to_mkv`/`hevc_to_mkv` (all AAC-bearing fixtures) in that
    /// experiment. Routing through the real predicate must NOT reproduce it.
    #[test]
    fn resolve_codec_tag_clears_mp4a_tagged_aac_into_mkv() {
        crate::ffmpeg::ensure_init().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        let oformat = oformat_for_extension(dir.path(), "mkv");
        let mut params = fake_params(
            ffmpeg_the_third::ffi::AVCodecID::AV_CODEC_ID_AAC,
            ffmpeg_the_third::ffi::AVMediaType::AVMEDIA_TYPE_AUDIO,
        );
        // MP4/ISOBMFF's fourcc for AAC ('mp4a'), packed the same way FFmpeg's
        // `MKTAG` macro does (`u32::from_le_bytes` of the ASCII bytes).
        params.codec_tag = u32::from_le_bytes(*b"mp4a");

        let action = FFmpegRunner::resolve_codec_tag(oformat, std::ptr::from_ref(&params))
            .expect("MKV must represent AAC");
        assert_eq!(
            action,
            CodecTagAction::Clear,
            "mp4a does not round-trip to AAC in mkv's tag table -> Clear, not Preserve"
        );
    }

    /// CRITICAL regression guard (post-#549 review): `salvage_remux_sync`
    /// copies every stream in the corrupt input with no media-type filter
    /// (by design — it is the last-resort recovery path and must not drop
    /// arbitrary attachments), so an MKV carrying a font attachment (the
    /// standard layout for a subtitled release) must not have its font
    /// stream rejected. `AV_CODEC_ID_TTF`'s `AVMediaType` is
    /// `AVMEDIA_TYPE_ATTACHMENT` — mkv's codec-tag table has no entry for it
    /// and `mkv_query_codec` reports it unsupported (0), so before this fix
    /// the fallback reached the final `Err` branch. Fails against the
    /// unpatched predicate (which does not gate on medium at all).
    #[test]
    fn resolve_codec_tag_clears_ttf_attachment_into_mkv() {
        crate::ffmpeg::ensure_init().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        let oformat = oformat_for_extension(dir.path(), "mkv");
        let params = fake_params(
            ffmpeg_the_third::ffi::AVCodecID::AV_CODEC_ID_TTF,
            ffmpeg_the_third::ffi::AVMediaType::AVMEDIA_TYPE_ATTACHMENT,
        );

        let action = FFmpegRunner::resolve_codec_tag(oformat, std::ptr::from_ref(&params)).expect(
            "attachment streams (e.g. embedded fonts) must never be rejected on \
             codec representability — only Video/Audio/Subtitle streams are \
             eligible for IncompatibleContainerCodec",
        );
        assert_eq!(
            action,
            CodecTagAction::Clear,
            "no tag-table entry for a font attachment -> Clear (not Err)"
        );
    }

    /// Same medium-gate bug, `AVMEDIA_TYPE_DATA` variant: `BIN_DATA` (generic
    /// binary side-channel data, e.g. subtitle-adjacent blobs) has no
    /// mkv tag-table entry either and must not be rejected.
    #[test]
    fn resolve_codec_tag_clears_data_medium_into_mkv() {
        crate::ffmpeg::ensure_init().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        let oformat = oformat_for_extension(dir.path(), "mkv");
        let params = fake_params(
            ffmpeg_the_third::ffi::AVCodecID::AV_CODEC_ID_BIN_DATA,
            ffmpeg_the_third::ffi::AVMediaType::AVMEDIA_TYPE_DATA,
        );

        FFmpegRunner::resolve_codec_tag(oformat, std::ptr::from_ref(&params))
            .expect("AVMEDIA_TYPE_DATA streams must never be rejected on codec representability");
    }

    /// Sanity check on the other side of the same predicate: AVI's tag table
    /// genuinely has no representation for HEVC (`avienc` defines no
    /// `query_codec`, so it falls back to its own tag table, which is the
    /// case the fix must still reject).
    #[test]
    fn resolve_codec_tag_rejects_hevc_into_avi() {
        crate::ffmpeg::ensure_init().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        let oformat = oformat_for_extension(dir.path(), "avi");
        let params = fake_params(
            ffmpeg_the_third::ffi::AVCodecID::AV_CODEC_ID_HEVC,
            ffmpeg_the_third::ffi::AVMediaType::AVMEDIA_TYPE_VIDEO,
        );

        let err = FFmpegRunner::resolve_codec_tag(oformat, std::ptr::from_ref(&params))
            .expect_err("AVI cannot represent HEVC under this FFmpeg build");
        let msg = err.to_string().to_lowercase();
        assert!(msg.contains("avi") && msg.contains("hevc"), "got: {msg}");
    }

    /// IMPORTANT #5 (code-review follow-up): the medium gate at the top of
    /// `resolve_codec_tag` exempts Attachment/Data/Unknown streams from the
    /// representability question entirely (see
    /// `resolve_codec_tag_clears_ttf_attachment_into_mkv` /
    /// `resolve_codec_tag_clears_data_medium_into_mkv` above) — but Subtitle
    /// stays in the eligible set alongside Video/Audio. Nothing pinned that a
    /// subtitle codec a container genuinely cannot represent is still
    /// rejected, so a future widening of the exemption to
    /// `AVMEDIA_TYPE_SUBTITLE` would go green with no test noticing. AVI's
    /// tag table has no entry for `SubRip` and `avienc` defines no
    /// `query_codec` fallback (the same shape that makes
    /// `resolve_codec_tag_rejects_hevc_into_avi` reject HEVC above), so the
    /// pairing must still be rejected, not waved through as if it were an
    /// attachment.
    #[test]
    fn resolve_codec_tag_rejects_subrip_into_avi() {
        crate::ffmpeg::ensure_init().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        let oformat = oformat_for_extension(dir.path(), "avi");
        let params = fake_params(
            ffmpeg_the_third::ffi::AVCodecID::AV_CODEC_ID_SUBRIP,
            ffmpeg_the_third::ffi::AVMediaType::AVMEDIA_TYPE_SUBTITLE,
        );

        let err = FFmpegRunner::resolve_codec_tag(oformat, std::ptr::from_ref(&params)).expect_err(
            "subtitle streams must remain rejection-eligible: AVI cannot represent SubRip",
        );
        let msg = err.to_string().to_lowercase();
        assert!(msg.contains("avi") && msg.contains("subrip"), "got: {msg}");
    }
}
