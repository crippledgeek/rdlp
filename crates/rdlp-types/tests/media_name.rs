//! Behavioural contract for the media-name newtypes (#642).
//!
//! These pin the four guarantees the types exist to provide:
//! 1. a validating constructor rejects the names that used to reach FFI as
//!    `CString::new(..)` failures or empty-string nonsense,
//! 2. the static-table constructor is usable in `const` context,
//! 3. the wire representation is unchanged (plain string), and
//! 4. codec names and encoder names are separate types.
//!
//! (4) is a compile-time property that a runtime test cannot observe, so it is
//! asserted by the `compile_fail` doctests on `rdlp_types::media_name` instead.
//! Those were verified to fail with `E0308: mismatched types` — a `compile_fail`
//! block that fails for some other reason (a typo, a bad import) passes
//! vacuously and guards nothing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use rdlp_types::media_name::{
    AudioEncoderName, CodecName, InvalidMediaName, Rfc6381Codec, VideoEncoderName,
};

// ─── Positive: the happy path for each concept ──────────────────────────────

#[test]
fn accepts_the_real_names_each_type_carries() {
    assert_eq!(CodecName::new("h264").unwrap().as_str(), "h264");
    assert_eq!(CodecName::new("pcm_s16le").unwrap().as_str(), "pcm_s16le");
    assert_eq!(CodecName::new("mpeg2video").unwrap().as_str(), "mpeg2video");

    assert_eq!(
        AudioEncoderName::new("libfdk_aac").unwrap().as_str(),
        "libfdk_aac"
    );
    assert_eq!(
        VideoEncoderName::new("libx264").unwrap().as_str(),
        "libx264"
    );

    // RFC 6381 forms carry dots and digits.
    assert_eq!(
        Rfc6381Codec::new("avc1.640028").unwrap().as_str(),
        "avc1.640028"
    );
    assert_eq!(
        Rfc6381Codec::new("mp4a.40.2").unwrap().as_str(),
        "mp4a.40.2"
    );
}

/// A single character is the shortest legal name — the boundary against the
/// empty-string rejection below.
#[test]
fn accepts_single_character_name() {
    assert_eq!(CodecName::new("x").unwrap().as_str(), "x");
}

// ─── Negative: each rejection is a distinct failure mode ────────────────────

/// The empty string is what `resolve_recode_encoder` and `RecodeStage` used to
/// filter by hand (`.filter(|s| !s.is_empty())`) because it produced a nonsense
/// error downstream. The constructor makes it unrepresentable instead.
#[test]
fn rejects_empty_name() {
    assert_eq!(CodecName::new(""), Err(InvalidMediaName::Empty));
    assert_eq!(AudioEncoderName::new(""), Err(InvalidMediaName::Empty));
    assert_eq!(VideoEncoderName::new(""), Err(InvalidMediaName::Empty));
    assert_eq!(Rfc6381Codec::new(""), Err(InvalidMediaName::Empty));
}

/// An interior NUL is the one input that makes `CString::new` fail at the FFI
/// boundary. Centralising it here is what lets the call sites drop their
/// scattered `CString::new(..).ok()?` guards.
#[test]
fn rejects_interior_nul() {
    assert_eq!(
        CodecName::new("h26\u{0}4"),
        Err(InvalidMediaName::ControlCharacter)
    );
}

/// Whitespace never appears in an FFmpeg codec or encoder name; accepting it
/// would let a stray token from a split/parse reach a lookup and silently miss.
#[test]
fn rejects_whitespace() {
    assert_eq!(
        CodecName::new("bad name"),
        Err(InvalidMediaName::Whitespace)
    );
    assert_eq!(CodecName::new(" h264"), Err(InvalidMediaName::Whitespace));
    assert_eq!(CodecName::new("h264\n"), Err(InvalidMediaName::Whitespace));
    assert_eq!(CodecName::new("\t"), Err(InvalidMediaName::Whitespace));
}

#[test]
fn rejects_non_ascii() {
    assert_eq!(CodecName::new("h264é"), Err(InvalidMediaName::NonAscii));
    assert_eq!(CodecName::new("𝓍264"), Err(InvalidMediaName::NonAscii));
}

#[test]
fn rejects_other_control_characters() {
    assert_eq!(
        CodecName::new("h264\u{7}"),
        Err(InvalidMediaName::ControlCharacter)
    );
}

// ─── Wire representation must be unchanged ──────────────────────────────────

/// The migration must not alter `config.toml` or the desktop IPC contract:
/// these serialise as plain strings, exactly as the `String` fields did.
#[test]
fn serialises_transparently_as_a_plain_string() {
    let codec = CodecName::new("h264").unwrap();
    assert_eq!(serde_json::to_string(&codec).unwrap(), "\"h264\"");

    let enc = VideoEncoderName::new("libx264").unwrap();
    assert_eq!(serde_json::to_string(&enc).unwrap(), "\"libx264\"");
}

#[test]
fn deserialises_from_a_plain_string() {
    let codec: CodecName = serde_json::from_str("\"vp9\"").unwrap();
    assert_eq!(codec.as_str(), "vp9");
}

/// Deserialisation must enforce the same invariant as the constructor —
/// otherwise serde becomes a hole straight past the validation.
#[test]
fn deserialisation_rejects_an_invalid_name() {
    assert!(serde_json::from_str::<CodecName>("\"\"").is_err());
    assert!(serde_json::from_str::<CodecName>("\"bad name\"").is_err());
}

#[test]
fn round_trips_through_serde() {
    let original = CodecName::new("pcm_s16le").unwrap();
    let json = serde_json::to_string(&original).unwrap();
    let back: CodecName = serde_json::from_str(&json).unwrap();
    assert_eq!(original, back);
}

// ─── Const construction keeps the static tables allocation-free ─────────────

static CODEC_TABLE: &[CodecName] = &[
    CodecName::from_static("h264"),
    CodecName::from_static("aac"),
];

static ENCODER_TABLE: &[VideoEncoderName] = &[VideoEncoderName::from_static("libx264")];

#[test]
fn const_constructor_works_in_a_static_table() {
    assert_eq!(CODEC_TABLE[0].as_str(), "h264");
    assert_eq!(CODEC_TABLE[1].as_str(), "aac");
    assert_eq!(ENCODER_TABLE[0].as_str(), "libx264");
}

/// `from_static` must not allocate — the table entries stay borrowed.
#[test]
fn const_constructor_borrows_rather_than_allocating() {
    assert!(CODEC_TABLE[0].is_borrowed());
    assert!(!CodecName::new("h264").unwrap().is_borrowed());
}

// ─── Equality / lookup semantics ────────────────────────────────────────────

/// A borrowed and an owned value naming the same codec must compare equal,
/// or registry lookups would depend on how the value happened to be built.
#[test]
fn borrowed_and_owned_compare_equal() {
    assert_eq!(
        CodecName::from_static("h264"),
        CodecName::new("h264").unwrap()
    );
}

/// FFmpeg descriptor lookup is exact and case-sensitive (`muxer_can_represent`
/// documents this), so the type must not silently case-fold.
#[test]
fn equality_is_case_sensitive() {
    assert_ne!(
        CodecName::from_static("h264"),
        CodecName::new("H264").unwrap()
    );
}

#[test]
fn display_renders_the_bare_name() {
    assert_eq!(CodecName::from_static("h264").to_string(), "h264");
    assert_eq!(
        Rfc6381Codec::new("avc1.640028").unwrap().to_string(),
        "avc1.640028"
    );
}
