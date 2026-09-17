//! One place that says which `FFmpeg` codec a manifest or format-id token
//! names.
//!
//! HLS `CODECS=` and DASH `codecs=` carry RFC 6381 strings (`avc1.640028`,
//! `mp4a.40.2`), and extractors mint format ids that embed plain names
//! (`hls-h264-fallback`, `hls-av1-url`). Before this table each consumer
//! classified those with `contains("aac")` / `starts_with("avc")` — nothing
//! enforced that the mappings were complete, mutually exclusive, or even
//! the same across sites (#648, same class as #576). Identity now resolves
//! here, by **exact token**, against the codes the MP4 Registration
//! Authority registers, and the set is open: an unrecognised token is
//! `None`, never a default.
//!
//! Sources: MP4RA "Codecs" (sample entry codes: `avc1`–`avc4`, `hvc1`,
//! `hev1`, `av01`, `vp08`, `vp09`, `mp4a`, `Opus`, `ac-3`, `ec-3`, `fLaC`,
//! `dva1`/`dvav`/`dvh1`/`dvhe` Dolby Vision profiles of AVC/HEVC) and
//! MP4RA "Object Types" (`mp4a` object-type indications: `40` MPEG-4 audio
//! and `66`/`67`/`68` MPEG-2 AAC are AAC; `69` ISO 13818-3 and `6B` ISO
//! 11172-3 are MP3; `AD` is Opus; `A5`/`A6` AC-3/E-AC-3 are withdrawn in
//! favour of the `ac-3`/`ec-3` sample entries). RFC 6381 §3.3 defines the
//! `<fourcc>.<oti>.<aot>` form for `mp4a`.

use super::CodecName;

/// Whether a codec carries video or audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodecKind {
    /// A video codec.
    Video,
    /// An audio codec.
    Audio,
}

/// The `FFmpeg` codec a token names, and which kind of stream it carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodecIdentity {
    /// Canonical `FFmpeg` codec-ID name.
    pub name: CodecName,
    /// Video or audio.
    pub kind: CodecKind,
}

impl CodecIdentity {
    const fn video(name: CodecName) -> Self {
        Self {
            name,
            kind: CodecKind::Video,
        }
    }

    const fn audio(name: CodecName) -> Self {
        Self {
            name,
            kind: CodecKind::Audio,
        }
    }
}

/// Resolve one token to its codec identity.
///
/// `token` is an RFC 6381 codec string (`avc1.640028`, `mp4a.40.2`, `Opus`),
/// the bare sample-entry code (`hev1`), or a plain `FFmpeg` codec-ID name or
/// common alias (`h264`, `avc`, `h265`, `hevc`, `aac`). Matching is on the
/// whole first dotted component, case-insensitively — never a substring —
/// so `pre_input_aac`-style accidents cannot classify. `mp4a` is refined by
/// its object-type indication; an unknown OTI is unclassified rather than
/// assumed AAC.
#[must_use]
pub fn codec_identity(token: &str) -> Option<CodecIdentity> {
    let token = token.trim();
    let mut parts = token.split('.');
    let code = parts.next()?.to_ascii_lowercase();
    Some(match code.as_str() {
        // Video — MP4RA sample entries plus the plain names extractors use.
        "avc1" | "avc2" | "avc3" | "avc4" | "avc" | "h264" | "dva1" | "dvav" => {
            CodecIdentity::video(CodecName::H264)
        }
        "hvc1" | "hev1" | "hevc" | "h265" | "dvh1" | "dvhe" => {
            CodecIdentity::video(CodecName::HEVC)
        }
        "av01" | "av1" => CodecIdentity::video(CodecName::AV1),
        "vp09" | "vp9" => CodecIdentity::video(CodecName::VP9),
        "vp08" | "vp8" => CodecIdentity::video(CodecName::VP8),
        // Audio.
        "mp4a" => CodecIdentity::audio(mp4a_object_type(parts.next())?),
        "aac" => CodecIdentity::audio(CodecName::AAC),
        "opus" => CodecIdentity::audio(CodecName::OPUS),
        "ac-3" | "ac3" => CodecIdentity::audio(CodecName::AC3),
        "ec-3" | "eac3" => CodecIdentity::audio(CodecName::EAC3),
        "flac" => CodecIdentity::audio(CodecName::FLAC),
        "mp3" => CodecIdentity::audio(CodecName::from_static("mp3")),
        _ => return None,
    })
}

/// The codec behind an `mp4a` sample entry, from its object-type indication
/// (MP4RA "Object Types"). A bare `mp4a` with no OTI is treated as AAC — the
/// overwhelmingly common case and what every HLS packager means by it.
fn mp4a_object_type(oti: Option<&str>) -> Option<CodecName> {
    match oti.map(str::to_ascii_lowercase).as_deref() {
        None | Some("40" | "66" | "67" | "68") => Some(CodecName::AAC),
        Some("69" | "6b") => Some(CodecName::from_static("mp3")),
        Some("ad") => Some(CodecName::OPUS),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `"name/kind"` for terse assertions; `None` stays `None`.
    fn name(token: &str) -> Option<String> {
        codec_identity(token).map(|id| format!("{}/{:?}", id.name.as_str(), id.kind))
    }

    #[test]
    fn rfc6381_sample_entries_resolve_with_their_profile_suffix() {
        assert_eq!(name("avc1.640028"), Some("h264/Video".to_owned()));
        assert_eq!(name("hev1.1.6.L93.B0"), Some("hevc/Video".to_owned()));
        assert_eq!(name("av01.0.08M.08"), Some("av1/Video".to_owned()));
        assert_eq!(name("vp09.00.10.08"), Some("vp9/Video".to_owned()));
        assert_eq!(name("mp4a.40.2"), Some("aac/Audio".to_owned()));
        assert_eq!(name("Opus"), Some("opus/Audio".to_owned()));
        assert_eq!(name("ec-3"), Some("eac3/Audio".to_owned()));
        assert_eq!(name("fLaC"), Some("flac/Audio".to_owned()));
    }

    #[test]
    fn plain_names_and_aliases_resolve() {
        assert_eq!(name("H264"), Some("h264/Video".to_owned()));
        assert_eq!(name("avc"), Some("h264/Video".to_owned()));
        assert_eq!(name("h265"), Some("hevc/Video".to_owned()));
        assert_eq!(name("aac"), Some("aac/Audio".to_owned()));
    }

    /// MP4RA object types: `mp4a` is AAC only for the MPEG-4/MPEG-2 AAC
    /// OTIs; 69/6B are MP3, AD is Opus, anything else is unknown.
    #[test]
    fn mp4a_is_refined_by_its_object_type_indication() {
        assert_eq!(name("mp4a"), Some("aac/Audio".to_owned()));
        assert_eq!(name("mp4a.67.1"), Some("aac/Audio".to_owned()));
        assert_eq!(name("mp4a.69"), Some("mp3/Audio".to_owned()));
        assert_eq!(name("mp4a.6B"), Some("mp3/Audio".to_owned()));
        assert_eq!(name("mp4a.AD"), Some("opus/Audio".to_owned()));
        assert_eq!(name("mp4a.a5"), None, "withdrawn OTI: unclassified");
    }

    /// Exact-token matching: a token that merely contains a codec name is
    /// not that codec, and an unknown token is `None`, never a default.
    #[test]
    fn substrings_and_unknown_tokens_do_not_classify() {
        assert_eq!(name("hls-h264-fallback"), None);
        assert_eq!(name("xaac"), None);
        assert_eq!(name("aacx"), None);
        assert_eq!(name("theora"), None);
        assert_eq!(name(""), None);
    }
}
