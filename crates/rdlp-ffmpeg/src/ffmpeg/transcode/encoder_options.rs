//! Build the ordered encoder-option key/value list for a video recode.
//!
//! Kept pure (no [`ffmpeg_the_third::Dictionary`], no encoder open) so the
//! full matrix — preset, crf, threads, and per-encoder wide-parallelism params
//! — is unit-testable.  The caller turns the returned pairs into an
//! [`ffmpeg_the_third::Dictionary`] and passes it to `open_as_with`.

use crate::ffmpeg::options::VideoConvertOptions;
use crate::ffmpeg::transcode::thread_resolve::{
    WIDE_PARALLELISM_THRESHOLD, resolve_recode_threads,
};

/// libvvenc tuning string applied above `WIDE_PARALLELISM_THRESHOLD` threads.
/// `wavefrontsynchro=1` enables wavefront-parallel processing; `tiles=2x2`
/// splits the frame into a 2×2 grid for balanced spatial parallelism on typical
/// HD/4K sources — without these, libvvenc does not spread work past ~8 threads.
const VVENC_WIDE_PARAMS: &str = "wavefrontsynchro=1:tiles=2x2";

/// Produce `(key, value)` pairs to pass to `open_as_with`.
///
/// `encoder_name` is the resolved encoder name (e.g. `"libvvenc"`,
/// `"libx265"`, `"libvpx-vp9"`). Always emits an explicit positive `threads`
/// (libvvenc at 0 runs main-thread-only). Above `WIDE_PARALLELISM_THRESHOLD`,
/// adds the per-encoder params required to actually use the threads.
#[must_use]
pub fn build_video_encoder_options(
    opts: &VideoConvertOptions,
    encoder_name: &str,
) -> Vec<(&'static str, String)> {
    let mut kv: Vec<(&'static str, String)> = Vec::new();

    if let Some(ref preset) = opts.preset {
        kv.push(("preset", preset.clone()));
    }
    if let Some(crf) = opts.crf {
        kv.push(("crf", crf.to_string()));
    }

    let threads = resolve_recode_threads(opts.threads);
    kv.push(("threads", threads.to_string()));

    if threads > 1 && (encoder_name.contains("vpx") || encoder_name.contains("aom")) {
        kv.push(("row-mt", "1".to_string()));
    }
    if threads > WIDE_PARALLELISM_THRESHOLD && encoder_name.contains("vvenc") {
        kv.push(("vvenc-params", VVENC_WIDE_PARAMS.to_string()));
    }

    kv
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts_with(
        preset: Option<&str>,
        crf: Option<u32>,
        threads: Option<u32>,
    ) -> VideoConvertOptions {
        VideoConvertOptions {
            preset: preset.map(String::from),
            crf,
            threads,
            ..Default::default()
        }
    }

    #[test]
    fn always_sets_explicit_positive_threads() {
        let kv = build_video_encoder_options(&opts_with(None, None, Some(6)), "libx265");
        assert!(kv.iter().any(|(k, v)| *k == "threads" && v == "6"));
    }

    #[test]
    fn threads_never_zero_even_when_unset() {
        let kv = build_video_encoder_options(&opts_with(None, None, None), "libvvenc");
        let t = kv
            .iter()
            .find(|(k, _)| *k == "threads")
            .map(|(_, v)| v)
            .unwrap();
        assert_ne!(t, "0");
        assert!(t.parse::<u32>().unwrap() >= 1);
    }

    #[test]
    fn preset_override_reaches_options() {
        let kv = build_video_encoder_options(&opts_with(Some("faster"), None, Some(4)), "libvvenc");
        assert!(kv.iter().any(|(k, v)| *k == "preset" && v == "faster"));
    }

    #[test]
    fn crf_reaches_options() {
        let kv =
            build_video_encoder_options(&opts_with(Some("medium"), Some(28), Some(4)), "libx265");
        assert!(kv.iter().any(|(k, v)| *k == "crf" && v == "28"));
    }

    #[test]
    fn vpx_gets_row_mt() {
        let kv = build_video_encoder_options(&opts_with(None, Some(30), Some(4)), "libvpx-vp9");
        assert!(kv.iter().any(|(k, v)| *k == "row-mt" && v == "1"));
    }

    #[test]
    fn vvenc_wide_params_only_above_threshold() {
        let narrow = build_video_encoder_options(&opts_with(None, None, Some(8)), "libvvenc");
        assert!(!narrow.iter().any(|(k, _)| *k == "vvenc-params"));
        let wide = build_video_encoder_options(&opts_with(None, None, Some(16)), "libvvenc");
        assert!(wide.iter().any(|(k, _)| *k == "vvenc-params"));
    }

    #[test]
    fn x265_no_wide_params() {
        let kv =
            build_video_encoder_options(&opts_with(Some("medium"), Some(28), Some(16)), "libx265");
        assert!(
            !kv.iter()
                .any(|(k, _)| *k == "vvenc-params" || *k == "row-mt")
        );
    }

    #[test]
    fn aom_gets_row_mt() {
        let kv = build_video_encoder_options(&opts_with(None, Some(28), Some(4)), "libaom-av1");
        assert!(kv.iter().any(|(k, v)| *k == "row-mt" && v == "1"));
    }

    #[test]
    fn row_mt_absent_at_single_thread() {
        let kv = build_video_encoder_options(&opts_with(None, Some(30), Some(1)), "libvpx-vp9");
        assert!(!kv.iter().any(|(k, _)| *k == "row-mt"));
    }
}
