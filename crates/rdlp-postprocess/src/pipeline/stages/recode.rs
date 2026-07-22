//! `RecodeStage` — transcodes video to a different container format.
//!
//! This stage runs at index 4 when `config.recode_video` is `Some` or
//! `config.recode_container` is `Some`.
//! Uses `rdlp_ffmpeg::FFmpegRunner::convert_video()`.

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use log::{debug, info, warn};

use rdlp_ffmpeg::ffmpeg::source::{Audio, Source, SourceState, SourceVideo, Video, muxer_decides};
use rdlp_ffmpeg::ffmpeg::{audio_encoder_registry, video_codecs};
use rdlp_ffmpeg::{FFmpegRunner, PostProcessError, VideoConvertOptions};
use rdlp_types::rule::{Rule, always};
use rdlp_types::{AudioEncoderName, CodecName, ContainerFormat, RecodeAudioMode, VideoEncoderName};

use crate::pipeline::stages::recode_audio_only;
use crate::pipeline::{PipelineMessage, PipelineStage};

/// Named parameters for [`RecodeStage::build_convert_options`].
///
/// Replaces 4 positional arguments so the compiler catches argument-swap bugs,
/// especially the boolean `audio_copy` / `can_remux` pair.
#[derive(Debug, Clone)]
pub(super) struct RecodeParams {
    pub target: ContainerFormat,
    pub encoder_override: Option<VideoEncoderName>,
    pub audio_copy: bool,
    pub audio_codec: Option<AudioEncoderName>,
    /// Resolved-or-configured encoder thread count (None = auto at encode layer).
    pub threads: Option<u32>,
    /// Encoder preset override; None preserves the per-codec default preset.
    pub preset_override: Option<String>,
    /// VPX/AV1 deadline knob (e.g. `"good"`, `"best"`, `"realtime"`).
    pub deadline: Option<String>,
    /// VPX/AV1 cpu-used knob.
    pub cpu_used: Option<i32>,
    /// VVC/x265/SVT-AV1 speed-level knob.
    pub speed_level: Option<u32>,
}

/// A first-class rule answering whether a source's video (as classified by
/// [`SourceVideo`]) may be stream-copied into a particular container.
///
/// Modeling each `rule_for` arm as a `VideoRule` **value** — built by a small
/// factory (`codec_in`, `muxer_decides`, `always`) rather than written inline
/// — means the codec identities a container accepts live in exactly one
/// place (the argument to `codec_in`) instead of being re-typed, with their
/// aliases, into every arm that needs them. #576's failure mode was a hand-
/// repeated alias pair (`"h264"`/`"avc"`) drifting out of sync across arms;
/// a rule value built from one call site cannot drift from itself.
///
/// Boxed because [`rule_for`] returns a different concrete rule per container
/// arm — `codec_in`, `muxer_decides::<Video>`, and `always` are all distinct
/// concrete types. No combinator exists yet to compose two rules into one;
/// each arm currently picks exactly one factory.
type VideoRule = Box<dyn for<'a> Rule<&'a SourceVideo> + Send + Sync>;

/// Matches when the source's video codec canonically identifies as one of
/// `identities` — alias-resolved via
/// [`video_codecs::canonical_codec_key`], so `"avc"` and `"h264"` are the
/// same identity and a caller only ever lists one canonical spelling.
/// [`SourceState::Unnamed`]/[`SourceState::Absent`] never match: neither
/// proves anything about representability (see [`RecodeStage::can_remux_video`]).
fn codec_in(identities: &'static [&'static str]) -> VideoRule {
    Box::new(move |video: &SourceVideo| match video.state() {
        SourceState::Codec(c) => {
            let key = video_codecs::canonical_codec_key(c.as_str()).unwrap_or(c.as_str());
            identities.iter().any(|id| key.eq_ignore_ascii_case(id))
        }
        SourceState::Unnamed | SourceState::Absent => false,
    })
}

/// The remux-compatibility rule for `output`, given the extension `input_ext`
/// originally carried (only the ext-compare fallback arms consult it — the
/// container-specific arms answer from the codec or from `FFmpeg`'s own
/// muxer, so `input_ext` is resolved into a plain `bool` via [`always`]
/// rather than threaded through as a rule parameter).
///
/// Exhaustively matched with no `_` arm: a newly-added [`ContainerFormat`]
/// variant must be classified here or the build fails. #538 is why — a
/// catch-all would have silently swapped the ASF-family codec-compatibility
/// rule for an extension compare when `Asf` split into three variants, with
/// no build error and no failing test. Same discipline as
/// `embed_strategy::for_container` (#525).
fn rule_for(input_ext: &str, output: ContainerFormat) -> VideoRule {
    match output {
        ContainerFormat::Mp4 | ContainerFormat::F4v => codec_in(&["h264", "h265", "mpeg4", "av1"]),
        // Ask the linked build whether this container can represent *this
        // source's* codec, rather than asserting the container is permissive
        // (#630). "Holds many codecs" and "holds the one in front of us" are
        // different questions, and the old unconditional `true` answered the
        // first: `hevc → avi` routed to a remux the mux path then refused
        // outright ("avi cannot represent hevc video"), and #618's
        // `Override("h264")` policy for Nut/Mxf/Avi was unreachable because
        // remux short-circuited before `default_codec_for_container` was
        // ever consulted.
        ContainerFormat::Mkv
        | ContainerFormat::Nut
        | ContainerFormat::Mxf
        | ContainerFormat::Avi => muxer_decides::<Video>(output),
        // `av_guess_format("x.mka")` resolves to matroska's *audio* muxer
        // (identical `name`, different muxer), which rejects every video
        // codec — so `muxer_decides` would answer "transcode", yet the
        // transcode path demonstrably succeeds in writing H.264 video into a
        // `.mka`. Whether an audio-only container may carry video is #577's
        // call, not a side effect of this routing fix; today's success is
        // left as-is via an unconditional `true`.
        ContainerFormat::Mka => Box::new(always(true)),
        ContainerFormat::WebM | ContainerFormat::Ivf => codec_in(&["vp8", "vp9", "av1"]),
        ContainerFormat::ThreeGp => codec_in(&["h264", "h263", "mpeg4"]),
        // All three ASF-family spellings share one muxer and therefore one
        // codec-compatibility answer. Listed explicitly rather than folded
        // into the ext-compare arm below: falling through would silently
        // swap this codec check for an extension compare (#538).
        ContainerFormat::Wmv | ContainerFormat::Wma | ContainerFormat::Asf => {
            codec_in(&["wmv1", "wmv2", "h264", "mpeg4"])
        }
        ContainerFormat::Mpg | ContainerFormat::Vob => codec_in(&["mpeg1", "mpeg2", "mpeg4"]),
        // Audio-only and any-other-video containers fall back to the
        // input/output ext compare, resolved eagerly into a plain `bool`
        // (this rule never needs the source's codec at all).
        ContainerFormat::Mov
        | ContainerFormat::M4v
        | ContainerFormat::Ts
        | ContainerFormat::Flv
        | ContainerFormat::Dv
        | ContainerFormat::Ogg
        | ContainerFormat::M4a
        | ContainerFormat::Mp3
        | ContainerFormat::Wav
        | ContainerFormat::Flac
        | ContainerFormat::Opus
        | ContainerFormat::Aac
        | ContainerFormat::Aiff
        | ContainerFormat::Wv
        | ContainerFormat::Caf
        | ContainerFormat::Ac3 => {
            let matches_ext = input_ext.eq_ignore_ascii_case(output.as_ext());
            Box::new(always(matches_ext))
        }
    }
}

/// An encoder whose output only one container can carry.
///
/// Expressed as data rather than as a hardcoded condition so that the matching
/// rule, the refusal, and the operator-facing explanation all derive from the
/// same row. Adding an encoder here needs no change to any predicate.
struct EncoderRestriction {
    /// The encoder this restriction constrains.
    encoder: VideoEncoderName,
    /// The only container that can carry its output.
    only: ContainerFormat,
    /// What goes wrong otherwise — quoted verbatim in the refusal so the
    /// operator learns the symptom, not just the rule.
    symptom: &'static str,
}

impl EncoderRestriction {
    /// `true` when this restriction permits `encoder` into `target`.
    ///
    /// A restriction has nothing to say about any encoder but its own, so an
    /// unrelated encoder is always allowed through.
    fn allows(&self, encoder: &VideoEncoderName, target: ContainerFormat) -> bool {
        *encoder != self.encoder || target == self.only
    }
}

/// Every known encoder/container restriction.
///
/// `from_static` validates each encoder name at const-eval, so a typo here is a
/// build error rather than a restriction that silently never matches.
static ENCODER_CONTAINER_RESTRICTIONS: &[EncoderRestriction] = &[EncoderRestriction {
    encoder: VideoEncoderName::from_static("libxavs2"),
    only: ContainerFormat::Mkv,
    symptom: "libxavs2 into any other container writes an empty file",
}];

/// Transcodes video to a different container/codec.
///
/// `should_run` triggers when `config.recode_video` is `Some`.
pub struct RecodeStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl RecodeStage {
    /// Create a new `RecodeStage`.
    #[must_use]
    pub const fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Determine if stream copy (remux) is possible for the codec/container
    /// combination, for a source that **has** a video stream.
    ///
    /// `video` distinguishes the two situations a bare `Option<&str>` collapsed
    /// into `None`, which is a distinction with opposite answers — see
    /// [`SourceVideo`]/[`SourceState`]. [`SourceState::Absent`] does not
    /// reach here: the caller routes an audio-only source to
    /// [`Self::can_copy_audio_only`] before asking, so this function never
    /// has to invent a video answer for a file with no video.
    ///
    /// Delegates to [`rule_for`], the first-class-function form of this
    /// compatibility question (#576's B3) — see that function's doc for the
    /// per-container reasoning and [`codec_in`]/[`muxer_decides`]/[`always`]
    /// for how each rule is built.
    fn can_remux_video(input_ext: &str, output: ContainerFormat, video: &SourceVideo) -> bool {
        rule_for(input_ext, output).eval(video)
    }

    /// The audio mode this run should use, forcing `Copy` when audio
    /// normalization is active — the normalizer already re-encoded the audio,
    /// and re-encoding it again degrades quality.
    ///
    /// Shared by the video and audio-only routes so the normalize interaction
    /// cannot apply on one and not the other.
    pub(super) fn resolve_recode_audio_mode(msg: &PipelineMessage) -> RecodeAudioMode {
        if msg.config.normalize_audio {
            if !matches!(msg.config.recode_audio, RecodeAudioMode::Copy) {
                warn!(
                    "RecodeStage: normalize_audio is active — forcing audio copy mode \
                     (re-encoding normalized audio degrades quality)"
                );
            }
            RecodeAudioMode::Copy
        } else {
            msg.config.recode_audio.clone()
        }
    }

    /// Pick the default video codec to encode toward when no explicit
    /// encoder override is given. Delegates to the single source in `rdlp-ffmpeg`
    /// so the recode pipeline and `validate_speed_controls` never resolve differently.
    fn default_codec_for(target: ContainerFormat) -> &'static str {
        rdlp_ffmpeg::default_codec_for_container(target)
    }

    /// The restriction `encoder` violates by targeting `target`, if any.
    ///
    /// Returns the offending row rather than a bare `bool` so the caller can
    /// name the container the encoder *does* mux into. The previous form
    /// hardcoded both the encoder (`"libxavs2"`) in the predicate and "MKV" in
    /// the caller's error string, so a second restricted encoder would have
    /// produced a confidently wrong message.
    fn violated_restriction(
        encoder: &VideoEncoderName,
        target: ContainerFormat,
    ) -> Option<&'static EncoderRestriction> {
        ENCODER_CONTAINER_RESTRICTIONS
            .iter()
            .find(|restriction| !restriction.allows(encoder, target))
    }

    /// Whether `encoder` can be muxed into `target`.
    ///
    /// The yes/no form of [`Self::violated_restriction`]. Test-only: the
    /// production path needs the violated row to build a truthful message, so
    /// keeping this compiled into the library would be dead code.
    #[cfg(test)]
    fn encoder_container_compatible(encoder: &VideoEncoderName, target: ContainerFormat) -> bool {
        Self::violated_restriction(encoder, target).is_none()
    }

    /// Build video conversion options.
    ///
    /// Returns `None` when an explicit encoder override is requested but not
    /// available in this `FFmpeg` build. `audio_codec` is `None` for copy,
    /// `Some(name)` for re-encode.
    ///
    /// `params.encoder_override` no longer needs an empty-string filter here:
    /// [`VideoEncoderName`] cannot hold an empty value, so `None` is the only
    /// spelling of "no override" that reaches this function (#642 removed the
    /// `Option<String>` shape Item 17 of PR-3's re-review had to defend against).
    fn build_convert_options(
        params: &RecodeParams,
        can_remux: bool,
    ) -> Option<VideoConvertOptions> {
        if can_remux {
            return Some(VideoConvertOptions {
                remux_only: true,
                audio_copy: params.audio_copy,
                ..Default::default()
            });
        }

        if let Some(requested) = params.encoder_override.as_ref() {
            let encoder_name = video_codecs::resolve_encoder(requested.as_str())?;
            let (default_preset, crf) = Self::default_preset_crf(&encoder_name);
            let preset = params.preset_override.clone().or(default_preset);
            return Some(VideoConvertOptions {
                remux_only: false,
                video_codec: Some(encoder_name),
                preset,
                crf,
                threads: params.threads,
                deadline: params.deadline.clone(),
                cpu_used: params.cpu_used,
                speed_level: params.speed_level,
                audio_copy: params.audio_copy,
                audio_codec: params.audio_codec.clone(),
                ..Default::default()
            });
        }

        Self::options_for_codec(Self::default_codec_for(params.target), params)
    }

    /// Resolves `target_codec` to an available encoder and assembles the
    /// transcode options, refusing (`None`) rather than silently no-opping
    /// when no encoder is available.
    ///
    /// Split out of `build_convert_options`'s no-override branch so the
    /// "unresolvable codec" refusal is directly testable with a bogus codec
    /// name (Important-4 of PR-3's #618 review), independent of what any
    /// particular linked `FFmpeg` build's muxers happen to declare. Before
    /// this fix, that branch computed `encoder: Option<&str>` and mapped it
    /// straight into `video_codec: encoder.map(String::from)` while still
    /// returning `Some(..)` unconditionally — a silent no-op transcode (no
    /// `video_codec` set, so `FFmpegRunner::convert_video` streams the
    /// source through with no video encoder configured), the exact shape
    /// that hid the `dvvideo` bug ("pipeline terminated with no output and
    /// no error"). Mirrors the audio side's honest refusal in
    /// `resolve_declared_codec`.
    ///
    /// Preset/CRF are looked up by the *resolved encoder*, via
    /// [`Self::default_preset_crf`] — the same table
    /// `build_convert_options`'s explicit-override branch uses — not by
    /// `target_codec`. A codec can resolve to more than one encoder (h264's
    /// row lists both `libx264` and `libopenh264`), and only some of a
    /// codec's encoders are x264-style CRF encoders; keying by codec name
    /// alone (the pre-#576 shape, a second hand-maintained table) would have
    /// forced CRF 23 onto `libopenh264` too, which is not an x264-style-CRF
    /// encoder (#576's B2).
    fn options_for_codec(
        target_codec: &'static str,
        params: &RecodeParams,
    ) -> Option<VideoConvertOptions> {
        let encoder = video_codecs::resolve_encoder(target_codec)?;
        let (default_preset, crf) = Self::default_preset_crf(&encoder);
        let preset = params.preset_override.clone().or(default_preset);

        Some(VideoConvertOptions {
            remux_only: false,
            video_codec: Some(encoder),
            preset,
            crf,
            threads: params.threads,
            deadline: params.deadline.clone(),
            cpu_used: params.cpu_used,
            speed_level: params.speed_level,
            audio_copy: params.audio_copy,
            // `params.audio_codec` is already an `AudioEncoderName` populated
            // exclusively by `Self::resolve_audio_params` / `resolve_audio_only_params`,
            // both of which resolve through `audio_encoder_registry` — so
            // this is a plain clone, not a re-parse.
            audio_codec: params.audio_codec.clone(),
            ..Default::default()
        })
    }

    /// Resolves `(audio_copy, audio_codec)` for a transcode (never called on
    /// the remux fast path, which always copies audio unconditionally).
    ///
    /// `has_audio` gates everything else: `setup_audio_pipeline` only creates
    /// an audio output stream when the input actually has an audio stream
    /// (`transcode/video_transcode_phases.rs`), so a video-only source has
    /// always made the resolved encoder irrelevant. Refusing before this
    /// check (#618) was a regression — a video-only `.ivf` recode used to
    /// succeed and started hard-failing.
    ///
    /// A video-only source returns `(false, None)`, not `(true, None)`: there
    /// is no audio stream to copy, so claiming `audio_copy = true` would
    /// stamp a false "copy" into the `encoding_tool` metadata tag (both
    /// branches are otherwise inert — `setup_audio_pipeline` only opens an
    /// audio output when `ctx.audio_transcode`/an input audio stream exists).
    pub(super) fn resolve_audio_params(
        recode_audio: &RecodeAudioMode,
        target: ContainerFormat,
        has_audio: bool,
    ) -> Result<(bool, Option<AudioEncoderName>), PostProcessError> {
        if !has_audio {
            return Ok((false, None));
        }

        let target_ext = target.as_ext();
        match recode_audio {
            RecodeAudioMode::Copy => Ok((true, None)),
            RecodeAudioMode::Auto => {
                // `None` does not strictly mean "this container cannot carry
                // audio" — see `muxer_defaults::declared_codec`'s doc for the
                // other two causes (no muxer claims the extension; ABI skew
                // between the muxer and libavcodec tables). Refuse truthfully
                // rather than guessing which cause applies, and rather than
                // silently dropping the audio track or defaulting to AAC
                // (FFmpeg itself would refuse with a cryptic "Invalid
                // argument" if we didn't).
                let Some(encoder) =
                    audio_encoder_registry::select_audio_encoder_for_container(target)
                else {
                    return Err(PostProcessError::UnsupportedFormat {
                        format: target_ext.to_string(),
                        operation: format!(
                            "no audio encoder could be determined for container \
                             '{target_ext}'; use --recode-audio=copy with a \
                             video-only source, or choose a different container"
                        ),
                    });
                };
                debug!("RecodeStage: auto audio encoder for {target_ext}: {encoder}");
                Ok((false, Some(encoder)))
            }
            RecodeAudioMode::Encoder { name } => {
                // `name` is a raw operator-supplied string (`--recode-audio=<name>`),
                // not yet validated against the `CodecName` vocabulary. An
                // invalid value can never match a compatibility-matrix row
                // either way, so treating it as "not confirmed compatible"
                // preserves this warn-then-proceed path's existing behaviour.
                let compatible = CodecName::new(name.as_str()).is_ok_and(|codec| {
                    audio_encoder_registry::container_supports_audio_codec(target, &codec)
                });
                if !compatible {
                    warn!(
                        "RecodeStage: audio codec '{name}' may not be compatible with \
                         container '{target_ext}'; proceeding anyway"
                    );
                }
                let resolved = audio_encoder_registry::resolve_audio_encoder(name).or_else(|| {
                    warn!(
                        "RecodeStage: audio encoder '{name}' not available; \
                         falling back to container default"
                    );
                    audio_encoder_registry::select_audio_encoder_for_container(target)
                });
                let Some(resolved) = resolved else {
                    return Err(PostProcessError::UnsupportedFormat {
                        format: target_ext.to_string(),
                        operation: format!(
                            "audio encoder '{name}' is unavailable and no audio \
                             encoder could be determined for container '{target_ext}'"
                        ),
                    });
                };
                debug!("RecodeStage: using audio encoder: {resolved}");
                Ok((false, Some(resolved)))
            }
        }
    }

    /// Default `(preset, crf)` for a resolved video encoder. Returns
    /// `(None, None)` for an encoder without a meaningful rdlp-chosen default
    /// — it falls back to its own `FFmpeg`-shipped defaults.
    ///
    /// The **single** source for this tuning — both `build_convert_options`'s
    /// explicit-`--video-encoder`-override branch and `options_for_codec`'s
    /// no-override (default-codec) branch call this, keyed by the resolved
    /// encoder. Before #576 (B2) there were two separate tables — this one,
    /// and a second keyed by *codec name* — and the codec-keyed one could not
    /// tell `libx264` and `libopenh264` apart (both answer to codec `"h264"`),
    /// so a build without `libx264` would have forced x264's CRF 23 onto
    /// `libopenh264`, which is not an x264-style-CRF encoder at all.
    ///
    /// Explicit per-encoder match (not a substring match): x265's own default
    /// CRF is 28 (x264's is 23 — same RF scale, different calibration curve,
    /// so reusing x264's value on x265 is ~5 CRF points too low and yields
    /// oversized HEVC output), and `libopenh264`/`libkvazaar` (which contain
    /// the substrings `"264"`/`"kvazaar"`) are NOT x264-style-CRF encoders at
    /// all, so a substring match would wrongly force CRF onto them.
    /// VP8/VP9/AV1's `Some(30)`/`Some(28)` are rdlp's own quality defaults,
    /// not each encoder's built-in default (libvpx/libaom/SVT-AV1 default to
    /// bitrate-based rate control absent an explicit `-crf`).
    fn default_preset_crf(encoder: &VideoEncoderName) -> (Option<String>, Option<u32>) {
        match encoder.as_str() {
            "libx264" | "libx264rgb" => (Some("medium".to_string()), Some(23)),
            "libx265" => (Some("medium".to_string()), Some(28)),
            "libvpx-vp9" | "libvpx" => (None, Some(30)),
            "libsvtav1" | "libaom-av1" | "librav1e" => (None, Some(28)),
            // libvvenc/libxeve/libxavs2/liboapv/libkvazaar/libopenh264/… defer to
            // the encoder's own rate control.
            _ => (None, None),
        }
    }
}

#[async_trait]
impl PipelineStage for RecodeStage {
    fn name(&self) -> &'static str {
        "RecodeStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        msg.config.recode_video.is_some() || msg.config.recode_container.is_some()
    }

    #[allow(clippy::too_many_lines)]
    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        if msg.tracker.current_files.is_empty() {
            return Ok(msg);
        }

        let input_file = msg.tracker.primary();

        // "No override" has exactly one representation since #642:
        // `msg.config.video_encoder` is `Option<VideoEncoderName>`, which
        // cannot hold an empty string, so there is no longer a blank-value
        // case to normalize here (or at the five sites below it used to be
        // filtered at — Item 17 of PR-3's re-review). The CLI pre-flight
        // validator (`rdlp_ffmpeg::resolve_recode_encoder`, called from
        // `config.rs`) and this stage now agree structurally, not by
        // convention.
        let video_encoder = msg.config.video_encoder.as_ref();

        // Resolve target container: prefer recode_container, fallback to
        // recode_video, fallback again to MP4.
        let target = msg
            .config
            .recode_container
            .or(msg.config.recode_video)
            .unwrap_or_else(|| {
                debug!("RecodeStage: no recode target configured; defaulting to MP4");
                ContainerFormat::Mp4
            });
        let target_ext = target.as_ext();

        let input_ext = input_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        if input_ext.eq_ignore_ascii_case(target_ext) && video_encoder.is_none() {
            debug!(
                "RecodeStage: file already in target format ({target_ext}) and no encoder override, skipping"
            );
            return Ok(msg);
        }

        info!(
            "RecodeStage: converting {} → {target_ext}",
            input_file.display()
        );

        let media_info = self
            .ffmpeg
            .probe(&input_file)
            .await
            .context("recode stage: failed to probe input file")?;
        let video: SourceVideo =
            Source::from_probe(media_info.has_video, media_info.video_codec.clone());
        let audio: Source<Audio> =
            Source::from_probe(media_info.has_audio, media_info.audio_codec.clone());

        // An audio-only source cannot go through the video pipeline at all —
        // `convert_video` opens a video decoder and ends at `NoVideoStream`
        // (#637). Neither ffmpeg nor yt-dlp imposes that precondition on a
        // container conversion, and `RemuxStage` already does not.
        if !video.is_present() {
            // There is no video stream for a video encoder to apply to, so the
            // override is inert here. Say so rather than discarding it
            // silently — `--video-encoder=libx265` on an `.m4a` otherwise
            // "succeeds" while doing nothing the user asked for.
            if let Some(requested) = video_encoder {
                warn!(
                    "RecodeStage: --video-encoder={requested} ignored — the source has no \
                     video stream to encode"
                );
            }
            return recode_audio_only::recode_audio_only(&self.ffmpeg, msg, target, &audio).await;
        }

        // When a video encoder is explicitly requested, always transcode — never remux
        let can_remux = video_encoder.is_none() && Self::can_remux_video(input_ext, target, &video);

        if can_remux {
            debug!("RecodeStage: remuxing (stream copy)");
        } else {
            debug!("RecodeStage: transcoding video");
        }

        let recode_audio = Self::resolve_recode_audio_mode(&msg);

        // Derive audio_copy / audio_codec from the resolved mode. Remux
        // stream-copies unconditionally when there IS an audio stream to
        // copy (no re-encoding needed); a video-only source has no audio
        // stream to copy, so `audio_copy` must be `false`, not a true claim
        // with nothing behind it — the same "+ none" truthfulness fix
        // `resolve_audio_params` already applies to the transcode path
        // (`23c344a7`), extended to remux so `encoding_tool` never stamps a
        // false "+ copy" for a video-only remux.
        let (audio_copy, audio_codec) = if can_remux {
            (media_info.has_audio, None)
        } else {
            Self::resolve_audio_params(&recode_audio, target, media_info.has_audio)?
        };

        let output_path = msg.tracker.temp_path(&input_file, target_ext);

        // Refuse a container-incompatible encoder up front with a TRUTHFUL error
        // (distinct from the generic "encoder not available" below) — e.g. AVS2
        // into MP4. Only relevant when actually transcoding (not remuxing).
        if !can_remux
            && let Some(req) = video_encoder
            && let Some(enc) = video_codecs::resolve_encoder(req.as_str())
            && let Some(restriction) = Self::violated_restriction(&enc, target)
        {
            return Err(PostProcessError::UnsupportedFormat {
                format: enc.to_string(),
                operation: format!(
                    "{enc} only muxes into {only}; requested container {target:?} ({symptom})",
                    only = restriction.only.as_ext(),
                    symptom = restriction.symptom,
                ),
            }
            .into());
        }

        let Some(mut opts) = Self::build_convert_options(
            &RecodeParams {
                target,
                encoder_override: video_encoder.cloned(),
                audio_copy,
                audio_codec,
                threads: msg.config.recode_threads,
                preset_override: msg.config.recode_preset.clone(),
                deadline: msg.config.recode_deadline.map(|d| d.as_str().to_string()),
                cpu_used: msg.config.recode_cpu_used,
                speed_level: msg.config.recode_speed_level,
            },
            can_remux,
        ) else {
            // Two distinct causes land here now that Important-4 makes the
            // default-codec path refuse instead of silently no-opping: an
            // explicit `--video-encoder` override that isn't available, or
            // (no override) an unresolvable default codec for the target
            // container. Distinguish them so the message names the thing
            // that actually failed, rather than a blank encoder name.
            let (format, operation) = if let Some(requested) = video_encoder {
                (
                    requested.to_string(),
                    format!("video encoder '{requested}' is not available in this FFmpeg build"),
                )
            } else {
                let target_codec = Self::default_codec_for(target);
                (
                    target_codec.to_string(),
                    format!(
                        "no encoder available for default video codec '{target_codec}' \
                         (target container '{target_ext}'); pass --video-encoder to \
                         choose one explicitly, or select a different container"
                    ),
                )
            };
            return Err(PostProcessError::UnsupportedFormat { format, operation }.into());
        };

        opts.verbose = msg.verbose;

        let stage_callback = msg.callback_factory.as_ref().map(|f| f(self.name()));
        // Bridge FFmpeg logs to the callback for real-time encoder output
        let _log_bridge = stage_callback
            .as_ref()
            .and_then(|cb| rdlp_ffmpeg::bridge_ffmpeg_logs(cb).ok());

        // Log recode parameters to the UI
        if let Some(ref cb) = stage_callback {
            let video_info = opts
                .video_codec
                .as_ref()
                .map_or("copy", rdlp_types::media_name::MediaName::as_str);
            let preset_info = opts.preset.as_deref().unwrap_or("default");
            let crf_info = opts
                .crf
                .map_or_else(|| "default".to_string(), |c| c.to_string());
            cb.on_log(&format!(
                "Recode: video={video_info} preset={preset_info} crf={crf_info}"
            ));
            if let Some(ref ac) = opts.audio_codec {
                cb.on_log(&format!("Recode: audio={ac}"));
            } else if opts.audio_copy {
                cb.on_log("Recode: audio=copy");
            }
            cb.on_log(&format!("Recode: container={target_ext}"));
        }

        let progress_callback =
            stage_callback
                .clone()
                .map(|cb| -> Arc<dyn Fn(f64) + Send + Sync> {
                    Arc::new(move |frac| cb.on_progress(rdlp_types::Progress::from_f64(frac)))
                });

        let log_callback: Option<Arc<dyn Fn(&str) + Send + Sync>> = if opts.verbose {
            stage_callback
                .clone()
                .map(|cb| -> Arc<dyn Fn(&str) + Send + Sync> {
                    Arc::new(move |msg| cb.on_log(msg))
                })
        } else {
            None
        };

        self.ffmpeg
            .convert_video(
                &input_file,
                &output_path,
                &opts,
                progress_callback,
                log_callback,
                Some(msg.cancel.clone()),
            )
            .await
            .context("recode stage failed")?;

        // Capture the encoding_tool for downstream pass-through stages.
        {
            let audio_part = rdlp_ffmpeg::ffmpeg::audio_tag_component(
                opts.audio_copy,
                opts.audio_codec
                    .as_ref()
                    .map(rdlp_types::media_name::MediaName::as_str),
            );
            let video_part = opts
                .video_codec
                .as_ref()
                .map_or("libx264", rdlp_types::media_name::MediaName::as_str);
            msg.encoding_tool = Some(format!("{video_part} + {audio_part}"));
        }

        if let Some(ref cb) = stage_callback {
            cb.on_log(&format!(
                "Recode: complete → {}",
                output_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ));
        }
        info!("RecodeStage: converted to {}", output_path.display());

        msg.tracker.replace(vec![output_path]);

        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    use rdlp_types::InfoDict;
    use rdlp_types::PostProcess;

    use crate::pipeline::{FileTracker, TempRegistry};

    #[test]
    fn hevc_default_crf_is_28_not_23() {
        assert_eq!(
            RecodeStage::default_preset_crf(&VideoEncoderName::from_static("libx265")),
            (Some("medium".into()), Some(28))
        );
    }

    /// Integrated regression guard for #576's B2 unification: the
    /// no-override *codec* path (`options_for_codec`) must resolve CRF from
    /// the same encoder-keyed table as the explicit-override path, not from
    /// a separate codec-keyed table that cannot distinguish `libx264` from
    /// `libopenh264` (both answer to codec `"h264"`, but only the former is
    /// an x264-style-CRF encoder).
    #[test]
    fn options_for_codec_h265_resolves_crf_28_via_the_encoder_keyed_table() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        if !rdlp_ffmpeg::ffmpeg::video_codecs::is_encoder_available("libx265") {
            return;
        }
        let params = RecodeParams {
            target: ContainerFormat::Mkv,
            encoder_override: None,
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::options_for_codec("h265", &params).expect("libx265 available");
        assert_eq!(opts.preset.as_deref(), Some("medium"));
        assert_eq!(opts.crf, Some(28));
    }

    #[test]
    fn h264_default_crf_stays_23() {
        assert_eq!(
            RecodeStage::default_preset_crf(&VideoEncoderName::from_static("libx264")),
            (Some("medium".into()), Some(23))
        );
    }

    /// Exercises `resolve_audio_params` itself (real `rdlp-postprocess`
    /// code), not just the registry it wraps.
    #[test]
    fn resolve_audio_params_auto_ivf_with_audio_is_refused_truthfully() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        let err =
            RecodeStage::resolve_audio_params(&RecodeAudioMode::Auto, ContainerFormat::Ivf, true)
                .expect_err("ivf has no audio encoder to resolve");
        let msg = err.to_string();
        assert!(
            msg.contains("no audio encoder could be determined"),
            "error should name the real cause, got: {msg}"
        );
    }

    #[test]
    fn resolve_audio_params_auto_mp4_with_audio_resolves_aac_family() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        let (audio_copy, audio_codec) =
            RecodeStage::resolve_audio_params(&RecodeAudioMode::Auto, ContainerFormat::Mp4, true)
                .expect("mp4 always resolves an aac-family encoder");
        assert!(!audio_copy);
        let codec = audio_codec.expect("mp4 auto mode must pick an encoder");
        assert!(
            codec == "aac" || codec == "libfdk_aac",
            "expected an aac-family encoder for mp4, got {codec}"
        );
    }

    #[test]
    fn resolve_audio_params_bogus_encoder_on_ivf_with_audio_is_refused() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        let mode = RecodeAudioMode::Encoder {
            name: "definitely_not_a_real_encoder_xyz".to_string(),
        };
        let err = RecodeStage::resolve_audio_params(&mode, ContainerFormat::Ivf, true)
            .expect_err("bogus encoder + no container default on ivf must refuse");
        let msg = err.to_string();
        assert!(
            msg.contains("unavailable") && msg.contains("no audio"),
            "error should explain both causes, got: {msg}"
        );
    }

    /// Regression guard (#618): a video-only source recoding to a container
    /// with no resolvable audio default (e.g. `.ivf`) must NOT be refused —
    /// `setup_audio_pipeline` only creates an audio stream when the input
    /// actually has one (`transcode/video_transcode_phases.rs`), so the
    /// chosen/default encoder was always irrelevant for video-only sources.
    /// Resolves to `(false, None)`, not `(true, None)` — there is no audio
    /// stream to copy.
    #[test]
    fn resolve_audio_params_auto_ivf_without_audio_omits_audio() {
        let result =
            RecodeStage::resolve_audio_params(&RecodeAudioMode::Auto, ContainerFormat::Ivf, false);
        assert_eq!(result.unwrap(), (false, None));
    }

    /// `RecodeAudioMode::Copy` with an actual audio stream present was
    /// untested — only the `has_audio == false` early-return path (which
    /// also short-circuits `Copy`) had coverage.
    #[test]
    fn resolve_audio_params_copy_mode_with_audio_copies() {
        let result =
            RecodeStage::resolve_audio_params(&RecodeAudioMode::Copy, ContainerFormat::Mp4, true);
        assert_eq!(result.unwrap(), (true, None));
    }

    /// The `Encoder` arm's SUCCESS path was untested — only its `Err` path
    /// (`resolve_audio_params_bogus_encoder_on_ivf_with_audio_is_refused`)
    /// had coverage. Pins that a valid-but-container-mismatched encoder name
    /// still resolves to `(false, Some(name))` rather than aborting — the
    /// container mismatch only warns (see `container_supports_audio_codec`).
    /// Uses `"flac"` (always built in, and its table row maps `flac -> flac`
    /// so `resolve_audio_encoder` is identity-stable on every build) rather
    /// than picking from a candidate list keyed on availability: `"aac"` is
    /// always available too, but `resolve_audio_encoder("aac")` resolves
    /// through the *preferred*-encoder table and returns `"libfdk_aac"` on a
    /// build that links it — an assertion pinned to the literal `"aac"`
    /// would then fail only on richer builds.
    #[test]
    fn resolve_audio_params_encoder_arm_succeeds_despite_container_mismatch() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        // flac does not support Wav, so this exercises the warn-then-proceed
        // branch rather than the compatible-encoder path.
        let mode = RecodeAudioMode::Encoder {
            name: "flac".to_string(),
        };
        let (audio_copy, audio_codec) =
            RecodeStage::resolve_audio_params(&mode, ContainerFormat::Wav, true)
                .expect("an available encoder must resolve despite a container mismatch warning");
        assert!(!audio_copy);
        assert_eq!(
            audio_codec
                .as_ref()
                .map(rdlp_types::media_name::MediaName::as_str),
            Some("flac")
        );
    }

    #[test]
    fn new_codecs_get_no_forced_crf() {
        assert_eq!(
            RecodeStage::default_preset_crf(&VideoEncoderName::from_static("libvvenc")),
            (None, None)
        );
        assert_eq!(
            RecodeStage::default_preset_crf(&VideoEncoderName::from_static("libxeve")),
            (None, None)
        );
        assert_eq!(
            RecodeStage::default_preset_crf(&VideoEncoderName::from_static("libxavs2")),
            (None, None)
        );
    }

    /// Integrated companion to `new_codecs_get_no_forced_crf`: the no-override
    /// codec path must also leave VVC/EVC/AVS2 unforced, through the same
    /// encoder-keyed table `options_for_codec` now shares with the
    /// explicit-override branch (#576's B2).
    #[test]
    fn options_for_codec_new_codecs_get_no_forced_crf() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        let params = RecodeParams {
            target: ContainerFormat::Mkv,
            encoder_override: None,
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        for codec in ["vvc", "h266"] {
            if let Some(opts) = RecodeStage::options_for_codec(codec, &params) {
                assert_eq!(
                    (opts.preset, opts.crf),
                    (None, None),
                    "{codec} must not get a forced preset/crf"
                );
            }
        }
    }

    fn make_msg(files: Vec<PathBuf>, config: PostProcess) -> PipelineMessage {
        let reg = Arc::new(TempRegistry::new());
        PipelineMessage {
            info: InfoDict::new(
                "id".to_string(),
                "Test".to_string(),
                "Test".to_string(),
                "https://example.com".to_string(),
            ),
            tracker: FileTracker::new(files, reg),
            config: Arc::new(config),
            original_stem: "test".to_string(),
            is_hls: false,
            verbose: false,
            callback_factory: None,
            warnings: Vec::new(),
            encoding_tool: None,
            cancel: tokio_util::sync::CancellationToken::new(),
        }
    }

    /// `resolve_recode_audio_mode` is the rule BOTH routes share, so it is
    /// exactly the thing that must not drift. The full 2x3 matrix:
    /// `normalize_audio` on forces `Copy` regardless of the configured mode;
    /// off passes the configured mode through untouched.
    #[test]
    fn resolve_recode_audio_mode_matrix() {
        let encoder = || RecodeAudioMode::Encoder {
            name: "libopus".to_string(),
        };
        for mode in [RecodeAudioMode::Copy, RecodeAudioMode::Auto, encoder()] {
            let forced = RecodeStage::resolve_recode_audio_mode(&make_msg(
                vec![PathBuf::from("/tmp/a.m4a")],
                PostProcess {
                    normalize_audio: true,
                    recode_audio: mode.clone(),
                    ..PostProcess::default()
                },
            ));
            assert!(
                matches!(forced, RecodeAudioMode::Copy),
                "normalize_audio must force Copy, got {forced:?} from {mode:?}"
            );

            let passthrough = RecodeStage::resolve_recode_audio_mode(&make_msg(
                vec![PathBuf::from("/tmp/a.m4a")],
                PostProcess {
                    normalize_audio: false,
                    recode_audio: mode.clone(),
                    ..PostProcess::default()
                },
            ));
            assert_eq!(
                passthrough, mode,
                "without normalize_audio the configured mode must pass through"
            );
        }
    }

    #[test]
    fn should_run_when_recode_video() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RecodeStage::new(ffmpeg);

        let config = PostProcess {
            recode_video: Some(ContainerFormat::Mkv),
            ..PostProcess::default()
        };
        let msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
        assert!(stage.should_run(&msg));
    }

    #[test]
    fn should_not_run_by_default() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RecodeStage::new(ffmpeg);
        let msg = make_msg(
            vec![PathBuf::from("/tmp/video.mp4")],
            PostProcess::default(),
        );
        assert!(!stage.should_run(&msg));
    }

    #[test]
    fn is_fatal() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RecodeStage::new(ffmpeg);
        assert!(stage.is_fatal());
    }

    /// Builds a [`SourceVideo`] whose codec is named, for call sites that
    /// used to construct the pre-#651 private `SourceVideo` enum's
    /// `Codec("...")` variant directly — now a <code>[Source]<[Video]></code>.
    fn video_codec(name: &'static str) -> SourceVideo {
        Source::from_probe(true, Some(CodecName::from_static(name)))
    }

    /// Builds a [`SourceVideo`] with a stream present but an unnameable codec.
    fn video_unnamed() -> SourceVideo {
        Source::from_probe(true, None)
    }

    /// Builds a [`SourceVideo`] with no video stream at all.
    fn video_absent() -> SourceVideo {
        Source::from_probe(false, None)
    }

    #[test]
    fn can_remux_h264_to_mp4() {
        assert!(RecodeStage::can_remux_video(
            ContainerFormat::Mkv.as_ext(),
            ContainerFormat::Mp4,
            &video_codec("h264")
        ));
    }

    /// #576's B1: the `"avc"` alias for `"h264"` must resolve identically to
    /// `"h264"` itself through the registry (`video_codecs::canonical_codec_key`),
    /// not via a hand-repeated literal in this match arm. Mutation-tested:
    /// removing `"avc"` from the h264 row's `aliases` in `video_codecs.rs`
    /// makes this test fail.
    #[test]
    fn can_remux_avc_alias_resolves_the_same_as_h264() {
        assert!(RecodeStage::can_remux_video(
            ContainerFormat::Mkv.as_ext(),
            ContainerFormat::Mp4,
            &video_codec("avc")
        ));
    }

    #[test]
    fn cannot_remux_vp9_to_mp4() {
        assert!(!RecodeStage::can_remux_video(
            ContainerFormat::WebM.as_ext(),
            ContainerFormat::Mp4,
            &video_codec("vp9")
        ));
    }

    #[test]
    fn can_remux_representable_codecs_to_mkv() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(RecodeStage::can_remux_video(
            ContainerFormat::Mp4.as_ext(),
            ContainerFormat::Mkv,
            &video_codec("h264")
        ));
        assert!(RecodeStage::can_remux_video(
            ContainerFormat::WebM.as_ext(),
            ContainerFormat::Mkv,
            &video_codec("vp9")
        ));
    }

    /// The three `avformat_query_codec` answers, one test each, on the
    /// containers #630 re-routes. `mkv`+`hevc` is the *positive* arm: the
    /// matroska muxer's own `mkv_query_codec` returns 1 even though `hevc`
    /// has no entry in matroska's codec-tag table, so a tag-table-only
    /// predicate would wrongly transcode it.
    #[test]
    fn remuxes_when_the_muxer_reports_the_codec_representable() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(
            RecodeStage::can_remux_video(
                ContainerFormat::Mp4.as_ext(),
                ContainerFormat::Mkv,
                &video_codec("hevc")
            ),
            "mkv must still remux hevc (mkv_query_codec reports 1)"
        );
        assert!(
            RecodeStage::can_remux_video(
                ContainerFormat::Mp4.as_ext(),
                ContainerFormat::Nut,
                &video_codec("hevc")
            ),
            "nut carries hevc under its own codec tag"
        );
        assert!(
            RecodeStage::can_remux_video(
                ContainerFormat::Mkv.as_ext(),
                ContainerFormat::Avi,
                &video_codec("h264")
            ),
            "avi carries h264 under a tag-table entry"
        );
    }

    /// The `0` (explicitly unsupported) arm. `avi` has no tag-table entry for
    /// `hevc` and `avienc` defines no `query_codec`, so the tag-table fallback
    /// answers 0 — and rdlp's own remux path refuses this pair downstream
    /// ("avi cannot represent hevc video"). Routing must reach the transcode
    /// instead of walking into that refusal.
    #[test]
    fn transcodes_when_the_muxer_reports_the_codec_unsupported() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(
            !RecodeStage::can_remux_video(
                ContainerFormat::Mp4.as_ext(),
                ContainerFormat::Avi,
                &video_codec("hevc")
            ),
            "avi cannot represent hevc; must transcode, not attempt a doomed remux"
        );
        assert!(
            !RecodeStage::can_remux_video(
                ContainerFormat::Mov.as_ext(),
                ContainerFormat::Nut,
                &video_codec("prores")
            ),
            "nut has neither a prores tag nor a positive query answer"
        );
    }

    /// The negative (`AVERROR_PATCHWELCOME`, "information unavailable") arm.
    /// `mxfenc` ships no `query_codec` callback and a null `codec_tag` table,
    /// so an undeclared codec answers negative. "Unknown" is not evidence a
    /// stream copy works, so it takes the same conservative branch as an
    /// explicit 0 — mirroring `resolve_codec_tag`'s `query > 0` rule.
    ///
    /// This used to assert `h264`, which #633 moved into
    /// `KNOWN_UNDECLARED_SUPPORT` — `mxfenc` does implement H.264, so that
    /// assertion pinned a false negative rather than the rule. `hevc` still
    /// demonstrates it: no evidence, no allow-list entry, so still "no".
    #[test]
    fn transcodes_when_representability_is_unknown() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(
            !RecodeStage::can_remux_video(
                ContainerFormat::Mp4.as_ext(),
                ContainerFormat::Mxf,
                &video_codec("hevc")
            ),
            "mxf answers AVERROR_PATCHWELCOME for hevc; unknown != supported"
        );
    }

    /// The #633 allow-list reaches the recode router: `h264 → mxf` is now a
    /// stream copy rather than a needless re-encode, because `mxfenc`
    /// implements it (`mxf_h264_codec_uls`) even though it declares nothing.
    #[test]
    fn remuxes_h264_into_mxf_via_the_known_support_table() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(
            RecodeStage::can_remux_video(
                ContainerFormat::Mp4.as_ext(),
                ContainerFormat::Mxf,
                &video_codec("h264")
            ),
            "mxf implements h264; routing must copy rather than re-encode (#633)"
        );
    }

    /// `Source::from_probe` maps the probe's two fields onto the three
    /// states — the discrimination the old `Option<&str>` could not express.
    #[test]
    fn from_probe_separates_absent_from_unnamed() {
        assert!(matches!(video_absent().state(), SourceState::Absent));
        assert!(matches!(video_unnamed().state(), SourceState::Unnamed));
        assert!(matches!(
            video_codec("h264").state(),
            SourceState::Codec(c) if c.as_str() == "h264"
        ));
        // A stale codec name with no video stream is still "no video": the
        // stream's absence wins over a leftover name.
        let stale: SourceVideo = Source::from_probe(false, Some(CodecName::from_static("h264")));
        assert!(matches!(stale.state(), SourceState::Absent));
    }

    /// A source whose codec ffprobe could not name proves nothing about
    /// representability, so it must transcode rather than inherit the old
    /// unconditional `true`.
    #[test]
    fn transcodes_when_the_source_codec_is_unknown() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(!RecodeStage::can_remux_video(
            ContainerFormat::Mp4.as_ext(),
            ContainerFormat::Avi,
            &video_unnamed()
        ));
        assert!(!RecodeStage::can_remux_video(
            ContainerFormat::Mp4.as_ext(),
            ContainerFormat::Mkv,
            &video_unnamed()
        ));
    }

    /// `Mka` is deliberately NOT re-routed by #630. `av_guess_format("x.mka")`
    /// resolves to matroska's *audio* muxer (same `name`, different muxer),
    /// which rejects every video codec — so the representability predicate
    /// would answer "transcode", and the transcode path demonstrably succeeds
    /// in writing an H.264 video stream into a `.mka`. That is the audio-only
    /// container question #577 owns; turning today's hard failure into a
    /// silent category error is not this fix's call to make.
    #[test]
    fn mka_routing_is_unchanged_pending_577() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(RecodeStage::can_remux_video(
            ContainerFormat::Mp4.as_ext(),
            ContainerFormat::Mka,
            &video_codec("h264")
        ));
    }

    #[test]
    fn build_convert_options_uses_recode_params_remux() {
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: None,
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, true).unwrap();
        assert!(opts.remux_only);
        assert!(opts.audio_copy);
    }

    /// Minor-7 regression guard: a video-only remux (`params.audio_copy ==
    /// false`, no audio stream present) must NOT stamp `audio_copy: true` —
    /// that would claim an audio "copy" that never happened. Pins that
    /// `build_convert_options` reads `params.audio_copy` on the remux path
    /// rather than hardcoding `true`.
    #[test]
    fn build_convert_options_remux_video_only_does_not_claim_audio_copy() {
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: None,
            audio_copy: false,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, true).unwrap();
        assert!(opts.remux_only);
        assert!(!opts.audio_copy);
    }

    #[test]
    fn avs2_only_muxes_into_mkv() {
        // AVS2 (libxavs2) recode is refused for non-MKV containers (the caller
        // turns this into a truthful "only muxes into MKV" error). Pure helper,
        // no FFmpeg needed.
        let xavs2 = VideoEncoderName::from_static("libxavs2");
        assert!(!RecodeStage::encoder_container_compatible(
            &xavs2,
            ContainerFormat::Mp4
        ));
        assert!(!RecodeStage::encoder_container_compatible(
            &xavs2,
            ContainerFormat::WebM
        ));
        assert!(RecodeStage::encoder_container_compatible(
            &xavs2,
            ContainerFormat::Mkv
        ));
        // Other encoders are unaffected by the AVS2-specific guard.
        assert!(RecodeStage::encoder_container_compatible(
            &VideoEncoderName::from_static("libx265"),
            ContainerFormat::Mp4
        ));
        assert!(RecodeStage::encoder_container_compatible(
            &VideoEncoderName::from_static("libvvenc"),
            ContainerFormat::Mp4
        ));
    }

    #[test]
    fn build_convert_options_uses_recode_params_transcode() {
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: None,
            audio_copy: false,
            audio_codec: Some(AudioEncoderName::from_static("libopus")),
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, false).unwrap();
        assert!(!opts.audio_copy);
        assert_eq!(
            opts.audio_codec
                .as_ref()
                .map(rdlp_types::media_name::MediaName::as_str),
            Some("libopus")
        );
    }

    #[test]
    fn build_convert_options_remux() {
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: None,
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, true).unwrap();
        assert!(opts.remux_only);
        assert!(opts.audio_copy);
    }

    #[test]
    fn build_convert_options_transcode_mp4() {
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: None,
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, false).unwrap();
        assert!(!opts.remux_only);
        assert!(opts.video_codec.is_some());
        assert_eq!(opts.preset, Some("medium".to_string()));
        assert_eq!(opts.crf, Some(23));
        assert!(opts.audio_copy);
        assert!(opts.audio_codec.is_none());
    }

    #[test]
    fn build_convert_options_with_audio_codec() {
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: None,
            audio_copy: false,
            audio_codec: Some(AudioEncoderName::from_static("libopus")),
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, false).unwrap();
        assert!(!opts.audio_copy);
        assert_eq!(
            opts.audio_codec
                .as_ref()
                .map(rdlp_types::media_name::MediaName::as_str),
            Some("libopus")
        );
    }

    #[test]
    fn build_convert_options_unavailable_encoder_returns_none() {
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: Some(VideoEncoderName::from_static("nonexistent_enc_xyz")),
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let result = RecodeStage::build_convert_options(&params, false);
        assert!(result.is_none());
    }

    /// Important-4 of PR-3's #618 review: an unresolvable *default* codec
    /// (no `--video-encoder` override) must refuse, not silently return
    /// `Some(VideoConvertOptions { video_codec: None, .. })` — the shape
    /// that hid the `dvvideo` bug ("pipeline terminated with no output and
    /// no error"). `options_for_codec` is the seam that makes this
    /// falsifiable with a bogus codec name, independent of any real
    /// container or any particular linked `FFmpeg` build's muxers.
    #[test]
    fn options_for_codec_refuses_an_unresolvable_codec_rather_than_no_opping() {
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: None,
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let result = RecodeStage::options_for_codec("definitely_not_a_real_codec_xyz", &params);
        assert!(
            result.is_none(),
            "an unresolvable codec must refuse rather than returning \
             Some(..) with video_codec: None"
        );
    }

    /// Positive companion: a genuinely resolvable codec must still produce
    /// `video_codec: Some(..)` through the same seam.
    #[test]
    fn options_for_codec_resolves_a_real_codec() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: None,
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts =
            RecodeStage::options_for_codec("h264", &params).expect("libx264 available in tests");
        assert!(!opts.remux_only);
        assert!(opts.video_codec.is_some());
    }

    #[test]
    fn threads_and_preset_override_flow_into_options() {
        let params = RecodeParams {
            target: ContainerFormat::Mkv,
            encoder_override: Some(VideoEncoderName::from_static("libx265")),
            audio_copy: true,
            audio_codec: None,
            threads: Some(6),
            preset_override: Some("faster".to_string()),
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, false).expect("encoder available");
        assert_eq!(opts.threads, Some(6));
        assert_eq!(opts.preset.as_deref(), Some("faster"));
    }

    #[test]
    fn preset_none_keeps_per_codec_default() {
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: Some(VideoEncoderName::from_static("libx265")),
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, false).expect("encoder available");
        assert_eq!(opts.preset.as_deref(), Some("medium"));
        assert_eq!(opts.threads, None);
    }

    #[test]
    fn deadline_and_cpu_used_thread_into_convert_options() {
        // deadline/cpu-used aren't meaningful for libx264, but build_convert_options
        // threads them regardless — that's what this test verifies.
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: Some(VideoEncoderName::from_static("libx264")),
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: Some("good".to_string()),
            cpu_used: Some(2),
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, false).expect("libx264 available");
        assert_eq!(opts.deadline.as_deref(), Some("good"));
        assert_eq!(opts.cpu_used, Some(2));
        assert_eq!(opts.speed_level, None);
    }

    /// Item 17 of PR-3's re-review (pre-#642) needed a runtime test proving
    /// an empty `encoder_override` was treated as "no override" — `Option<String>`
    /// could hold `Some("")`, a state indistinguishable from a real encoder
    /// name request until `build_convert_options` filtered it. #642 replaced
    /// `Option<String>` with `Option<VideoEncoderName>`, and `VideoEncoderName`
    /// cannot hold an empty value (see `media_name::validate`) — so that state
    /// is no longer constructible, and this is now a compile-time guarantee
    /// rather than a runtime behaviour to test. This positive test replaces
    /// the old one: it pins that `RecodeParams::encoder_override` genuinely
    /// requires a `VideoEncoderName`, not a raw string, at every call site.
    #[test]
    fn encoder_override_is_a_validated_video_encoder_name_not_a_raw_string() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: None,
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, false)
            .expect("None must fall through to the default codec");
        assert!(!opts.remux_only);
        assert!(
            opts.video_codec.is_some(),
            "must resolve mp4's default codec when no override is given"
        );
    }

    // The `process()`-entry-point companion to the removed empty-encoder
    // test (Item 17 of PR-3's re-review) lives in `tests/empty_video_encoder_e2e.rs`.
    // Since #642 that scenario constructs `PostProcess { video_encoder: None, .. }`
    // rather than `Some(String::new())` — the type change is the fix, so the
    // e2e test now documents "no override reaches RecodeStage cleanly" rather
    // than "an empty override is normalized to no override".
}
