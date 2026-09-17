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

/// What a token names: which kind of stream it carries, and — when the
/// registry pins it down — which `FFmpeg` codec.
///
/// `kind` is known whenever the sample-entry family is (`mp4a` is always
/// audio); `name` is `None` when the family is recognised but its sub-type
/// is not (an `mp4a` with an unregistered or withdrawn object type). A
/// consumer filling an audio/video slot uses `kind`; one naming the codec
/// uses `name`, and must not invent one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodecIdentity {
    /// Canonical `FFmpeg` codec-ID name, when known.
    pub name: Option<CodecName>,
    /// Video or audio.
    pub kind: CodecKind,
}

impl CodecIdentity {
    const fn video(name: CodecName) -> Self {
        Self {
            name: Some(name),
            kind: CodecKind::Video,
        }
    }

    const fn audio(name: Option<CodecName>) -> Self {
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
/// common alias (`h264`, `avc`, `h265`, `hevc`, `aac`). The first dotted
/// component is parsed as a [`SampleEntryCode`], else as a [`PlainName`];
/// each is an enum, so the vocabulary is closed and exact — never a
/// substring — while the function stays open: an unrecognised token is
/// `None`, not a default.
///
/// A format id split on non-alphanumerics cannot present the hyphenated
/// `ac-3` / `ec-3` codes; those only arrive whole from a `CODECS=` list.
#[must_use]
pub fn codec_identity(token: &str) -> Option<CodecIdentity> {
    let mut parts = token.trim().split('.');
    let code = parts.next()?;
    Some(match CodecToken::parse(code)? {
        CodecToken::SampleEntry(entry) => entry.identity(parts.next(), parts.next()),
        CodecToken::Plain(name) => name.identity(),
    })
}

/// The two vocabularies a token's first component can belong to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodecToken {
    /// An MP4RA sample-entry code (`avc1`, `mp4a`, `Opus`).
    SampleEntry(SampleEntryCode),
    /// A plain `FFmpeg` name or alias (`h264`, `avc`, `aac`).
    Plain(PlainName),
}

impl CodecToken {
    fn parse(code: &str) -> Option<Self> {
        match (SampleEntryCode::parse(code), PlainName::parse(code)) {
            (Some(entry), _) => Some(Self::SampleEntry(entry)),
            (None, Some(name)) => Some(Self::Plain(name)),
            (None, None) => None,
        }
    }
}

/// The MP4 sample-entry codes rdlp recognises (MP4RA "Codecs" registry;
/// RFC 6381 §3.3 puts one first in a `codecs` token). Case-insensitive on
/// parse — `Opus` and `fLaC` are registered with capitals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleEntryCode {
    /// `avc1`–`avc4`: Advanced Video Coding.
    Avc,
    /// `dva1` / `dvav`: AVC-based Dolby Vision.
    DolbyVisionAvc,
    /// `hvc1` / `hev1`: HEVC.
    Hevc,
    /// `dvh1` / `dvhe`: HEVC-based Dolby Vision.
    DolbyVisionHevc,
    /// `av01`: AOM Video Codec.
    Av01,
    /// `vp08`: VP8.
    Vp08,
    /// `vp09`: VP9.
    Vp09,
    /// `mp4a`: MPEG-4 Audio; the object type says which codec.
    Mp4a,
    /// `Opus`.
    Opus,
    /// `ac-3`: AC-3.
    Ac3,
    /// `ec-3`: Enhanced AC-3.
    Ec3,
    /// `fLaC`.
    Flac,
}

impl SampleEntryCode {
    /// Parse a sample-entry code (the part before the first `.`).
    #[must_use]
    pub fn parse(code: &str) -> Option<Self> {
        Some(match code.to_ascii_lowercase().as_str() {
            "avc1" | "avc2" | "avc3" | "avc4" => Self::Avc,
            "dva1" | "dvav" => Self::DolbyVisionAvc,
            "hvc1" | "hev1" => Self::Hevc,
            "dvh1" | "dvhe" => Self::DolbyVisionHevc,
            "av01" => Self::Av01,
            "vp08" => Self::Vp08,
            "vp09" => Self::Vp09,
            "mp4a" => Self::Mp4a,
            "opus" => Self::Opus,
            "ac-3" => Self::Ac3,
            "ec-3" => Self::Ec3,
            "flac" => Self::Flac,
            _ => return None,
        })
    }

    /// The identity, given the rest of the token (`oti`, `aot`) for `mp4a`.
    fn identity(self, oti: Option<&str>, aot: Option<&str>) -> CodecIdentity {
        match self {
            Self::Avc | Self::DolbyVisionAvc => CodecIdentity::video(CodecName::H264),
            Self::Hevc | Self::DolbyVisionHevc => CodecIdentity::video(CodecName::HEVC),
            Self::Av01 => CodecIdentity::video(CodecName::AV1),
            Self::Vp08 => CodecIdentity::video(CodecName::VP8),
            Self::Vp09 => CodecIdentity::video(CodecName::VP9),
            Self::Mp4a => CodecIdentity::audio(mp4a_codec(oti, aot)),
            Self::Opus => CodecIdentity::audio(Some(CodecName::OPUS)),
            Self::Ac3 => CodecIdentity::audio(Some(CodecName::AC3)),
            Self::Ec3 => CodecIdentity::audio(Some(CodecName::EAC3)),
            Self::Flac => CodecIdentity::audio(Some(CodecName::FLAC)),
        }
    }
}

/// The plain codec names and aliases extractors embed in format ids
/// (`hls-h264-fallback`, `hls-av1-url`) — `FFmpeg` codec-ID names plus the
/// common spellings `avc` / `h265` / `ac3` / `eac3`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlainName {
    /// `h264`, `avc`.
    H264,
    /// `hevc`, `h265`.
    Hevc,
    /// `av1`.
    Av1,
    /// `vp9`.
    Vp9,
    /// `vp8`.
    Vp8,
    /// `aac`.
    Aac,
    /// `opus`.
    Opus,
    /// `ac3`.
    Ac3,
    /// `eac3`.
    Eac3,
    /// `flac`.
    Flac,
    /// `vorbis`.
    Vorbis,
    /// `mp3`.
    Mp3,
}

impl PlainName {
    /// Parse a plain name or alias, case-insensitively.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "h264" | "avc" => Self::H264,
            "hevc" | "h265" => Self::Hevc,
            "av1" => Self::Av1,
            "vp9" => Self::Vp9,
            "vp8" => Self::Vp8,
            "aac" => Self::Aac,
            "opus" => Self::Opus,
            "ac3" => Self::Ac3,
            "eac3" => Self::Eac3,
            "flac" => Self::Flac,
            "vorbis" => Self::Vorbis,
            "mp3" => Self::Mp3,
            _ => return None,
        })
    }

    const fn identity(self) -> CodecIdentity {
        match self {
            Self::H264 => CodecIdentity::video(CodecName::H264),
            Self::Hevc => CodecIdentity::video(CodecName::HEVC),
            Self::Av1 => CodecIdentity::video(CodecName::AV1),
            Self::Vp9 => CodecIdentity::video(CodecName::VP9),
            Self::Vp8 => CodecIdentity::video(CodecName::VP8),
            Self::Aac => CodecIdentity::audio(Some(CodecName::AAC)),
            Self::Opus => CodecIdentity::audio(Some(CodecName::OPUS)),
            Self::Ac3 => CodecIdentity::audio(Some(CodecName::AC3)),
            Self::Eac3 => CodecIdentity::audio(Some(CodecName::EAC3)),
            Self::Flac => CodecIdentity::audio(Some(CodecName::FLAC)),
            Self::Vorbis => CodecIdentity::audio(Some(CodecName::VORBIS)),
            Self::Mp3 => CodecIdentity::audio(Some(CodecName::MP3)),
        }
    }
}

/// The codec behind an `mp4a` sample entry: its object-type indication
/// picks the standard, and for MPEG-4 Audio the audio object type picks the
/// codec. A bare `mp4a` is AAC — what every HLS packager means by it. A
/// value outside the registries is a known-audio, unnamed entry.
fn mp4a_codec(oti: Option<&str>, aot: Option<&str>) -> Option<CodecName> {
    use ObjectTypeIndication as Oti;
    match (oti.and_then(Oti::parse), oti.is_some(), aot) {
        // A bare `mp4a`, or MPEG-4 Audio with no audio object type: AAC.
        (None, false, _) | (Some(Oti::Mpeg4Audio), _, None) => Some(CodecName::AAC),
        // An OTI outside the registry (or withdrawn): audio, unnamed.
        (None, true, _) => None,
        (Some(Oti::Mpeg4Audio), _, Some(aot)) => Some(AudioObjectType::parse(aot)?.codec()),
        (Some(Oti::Mpeg2AacMain | Oti::Mpeg2AacLc | Oti::Mpeg2AacSsr), _, _) => {
            Some(CodecName::AAC)
        }
        (Some(Oti::Mpeg2Audio | Oti::Mpeg1Audio), _, _) => Some(CodecName::MP3),
        (Some(Oti::Opus), _, _) => Some(CodecName::OPUS),
    }
}

/// The `mp4a` object-type indications rdlp names (MP4RA "Object Types",
/// RFC 6381 §3.3: two hex digits). `A5`/`A6` (AC-3 / E-AC-3) are withdrawn
/// there in favour of the `ac-3`/`ec-3` sample entries, so they parse as
/// unknown on purpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ObjectTypeIndication {
    /// `0x40` — Audio ISO/IEC 14496-3; the audio object type decides.
    Mpeg4Audio,
    /// `0x66` — Audio ISO/IEC 13818-7 Main Profile.
    Mpeg2AacMain,
    /// `0x67` — Audio ISO/IEC 13818-7 Low Complexity Profile.
    Mpeg2AacLc,
    /// `0x68` — Audio ISO/IEC 13818-7 Scalable Sampling Rate Profile.
    Mpeg2AacSsr,
    /// `0x69` — Audio ISO/IEC 13818-3 (MPEG-2 Layer 3).
    Mpeg2Audio,
    /// `0x6B` — Audio ISO/IEC 11172-3 (MPEG-1 Layer 3).
    Mpeg1Audio,
    /// `0xAD` — Opus audio.
    Opus,
}

impl ObjectTypeIndication {
    fn parse(hex: &str) -> Option<Self> {
        Some(match u8::from_str_radix(hex, 16).ok()? {
            0x40 => Self::Mpeg4Audio,
            0x66 => Self::Mpeg2AacMain,
            0x67 => Self::Mpeg2AacLc,
            0x68 => Self::Mpeg2AacSsr,
            0x69 => Self::Mpeg2Audio,
            0x6B => Self::Mpeg1Audio,
            0xAD => Self::Opus,
            _ => return None,
        })
    }
}

/// The MPEG-4 audio object types rdlp names (ISO/IEC 14496-3 Table 1.17;
/// RFC 6381 §3.3: decimal). Everything in the AAC family is `aac` to
/// `FFmpeg`, which carries the profile separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AudioObjectType {
    /// 1 AAC Main, 2 AAC LC, 3 AAC SSR, 4 AAC LTP, 5 SBR, 6 AAC Scalable,
    /// 17 ER AAC LC, 19 ER AAC LTP, 20 ER AAC Scalable, 23 ER AAC LD, 29 PS,
    /// 39 ER AAC ELD, 42 USAC — all `aac` to `FFmpeg`. (7/21 `TwinVQ` and
    /// 22 BSAC are MPEG-4 Audio but not AAC; they parse as unknown.)
    AacFamily,
    /// 32 — MPEG-1/2 Layer 1.
    Layer1,
    /// 33 — MPEG-1/2 Layer 2.
    Layer2,
    /// 34 — MPEG-1/2 Layer 3.
    Layer3,
    /// 36 — ALS (Audio Lossless Coding); `mp4als` to `FFmpeg`.
    Als,
}

impl AudioObjectType {
    fn parse(decimal: &str) -> Option<Self> {
        Some(match decimal.parse::<u8>().ok()? {
            1..=6 | 17 | 19 | 20 | 23 | 29 | 39 | 42 => Self::AacFamily,
            32 => Self::Layer1,
            33 => Self::Layer2,
            34 => Self::Layer3,
            36 => Self::Als,
            _ => return None,
        })
    }

    const fn codec(self) -> CodecName {
        match self {
            Self::AacFamily => CodecName::AAC,
            Self::Layer1 => CodecName::from_static("mp1"),
            Self::Layer2 => CodecName::MP2,
            Self::Layer3 => CodecName::MP3,
            Self::Als => CodecName::from_static("mp4als"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `"name/kind"` for terse assertions (`?/Audio` when the family is
    /// known but the codec is not); `None` when the token is unrecognised.
    fn name(token: &str) -> Option<String> {
        codec_identity(token).map(|id| {
            format!(
                "{}/{:?}",
                id.name.as_ref().map_or("?", |n| n.as_str()),
                id.kind
            )
        })
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

    /// MP4RA object types and ISO 14496-3 audio object types: `mp4a.40` is
    /// AAC only for the AAC-family AOTs; `40.34` is MP3 (Apple's HLS
    /// authoring spec lists it), `40.33` MP2; 69/6B are MP3, AD is Opus. An
    /// unknown sub-type is still AUDIO, just not a named codec.
    #[test]
    fn mp4a_is_refined_by_object_type_indication_and_audio_object_type() {
        assert_eq!(name("mp4a"), Some("aac/Audio".to_owned()));
        assert_eq!(name("mp4a.40.2"), Some("aac/Audio".to_owned()));
        assert_eq!(name("mp4a.40.5"), Some("aac/Audio".to_owned()));
        assert_eq!(name("mp4a.40.42"), Some("aac/Audio".to_owned()));
        assert_eq!(name("mp4a.40.34"), Some("mp3/Audio".to_owned()));
        assert_eq!(
            name("mp4a.40.6"),
            Some("aac/Audio".to_owned()),
            "AAC Scalable"
        );
        assert_eq!(name("mp4a.40.36"), Some("mp4als/Audio".to_owned()));
        assert_eq!(
            name("mp4a.40.21"),
            Some("?/Audio".to_owned()),
            "ER TwinVQ is not AAC"
        );
        assert_eq!(
            name("mp4a.40.22"),
            Some("?/Audio".to_owned()),
            "ER BSAC is not AAC"
        );
        assert_eq!(name("mp4a.40.33"), Some("mp2/Audio".to_owned()));
        assert_eq!(name("mp4a.67.1"), Some("aac/Audio".to_owned()));
        assert_eq!(name("mp4a.69"), Some("mp3/Audio".to_owned()));
        assert_eq!(name("mp4a.6B"), Some("mp3/Audio".to_owned()));
        assert_eq!(name("mp4a.AD"), Some("opus/Audio".to_owned()));
        assert_eq!(
            name("mp4a.a5"),
            Some("?/Audio".to_owned()),
            "withdrawn OTI: audio, unnamed"
        );
        assert_eq!(
            name("mp4a.40.99"),
            Some("?/Audio".to_owned()),
            "unknown AOT: audio, unnamed"
        );
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
