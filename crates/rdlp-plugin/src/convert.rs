//! One place for every WIT⇄rdlp conversion (`Format`, `Fragment`, `InfoDict`).
//!
//! Shared by [`crate::adapter`] (guest `extract` results) and
//! [`crate::host::extract_helpers`] (host-side `extract-mpd` fragment map).
//! Previously three of these functions were duplicated inline across both
//! call sites; this module is the single source of truth so a fix to one
//! conversion reaches every caller.

use crate::bindings::rdlp::plugin::host_extract_helpers::MpdFragment;
use crate::bindings::rdlp::plugin::types::Format as WitFormat;
use rdlp_types::DownloadProtocol;

/// Narrow an `f64` to `f32` at the WIT boundary.
///
/// `types.wit` declares `fps`/`tbr`/`vbr`/`abr` as `f32`; rdlp-types uses
/// `f64` internally. This is the one site the narrowing happens, so
/// `clippy::cast_possible_truncation` has one rationale to check rather than
/// one per call site.
///
/// Only `format_to_wit` calls this today (production caller: the search
/// adapter, a later task); the `dead_code` `#[expect]` is self-cleaning so a
/// real caller turns the unfulfilled expectation into a `-D warnings` error
/// rather than silently staying suppressed like `#[allow]` would.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "first production caller: search adapter task")
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "WIT `types.format` declares fps/tbr/vbr/abr as f32 \
              (crates/rdlp-plugin/wit/types.wit); narrowing at the boundary \
              is the contract, and `expect` fails the build if the lint \
              ever stops firing"
)]
const fn narrow_f64(v: f64) -> f32 {
    v as f32
}

/// Convert a bindgen-generated `InfoDict` to the rdlp-types `InfoDict`.
///
/// The WIT `InfoDict` does not carry `extractor` or `webpage_url` — those are
/// filled in from the call context (`plugin_name` and `url`).
pub(crate) fn info_dict_from_wit(
    w: crate::bindings::rdlp::plugin::types::InfoDict,
    url: &str,
    plugin_name: &str,
) -> rdlp_types::InfoDict {
    let mut out = rdlp_types::InfoDict::new(
        w.id,
        w.title,
        plugin_name,
        // Prefer the URL the plugin returned; fall back to the request URL.
        w.url.as_deref().unwrap_or(url),
    );
    out.thumbnail = w.thumbnail;
    out.description = w.description;
    out.uploader = w.uploader;
    out.uploader_id = w.uploader_id;
    out.upload_date = w.upload_date;
    // WIT duration is Option<u32> (whole seconds); rdlp-types uses Option<f64>.
    out.duration = w.duration.map(f64::from);
    out.view_count = w.view_count;
    out.like_count = w.like_count;
    out.tags = if w.tags.is_empty() {
        None
    } else {
        Some(w.tags)
    };
    out.categories = if w.categories.is_empty() {
        None
    } else {
        Some(w.categories)
    };
    out.formats = w.formats.into_iter().map(format_from_wit).collect();
    // Convert subtitle list → InfoDict's `HashMap<lang, Vec<Subtitle>>` format.
    if !w.subtitles.is_empty() {
        use rdlp_types::info_dict::Subtitle;
        use std::collections::HashMap;
        let mut map: HashMap<String, Vec<Subtitle>> = HashMap::new();
        for s in w.subtitles {
            map.entry(s.language).or_default().push(Subtitle {
                url: s.url,
                ext: s.ext,
                name: None,
            });
        }
        out.subtitles = Some(map);
    }
    out
}

/// Sanitise a plugin-supplied string before it enters a filesystem path.
///
/// Plugin output is untrusted: a malicious extractor could return
/// `format_id = "/etc/cron.d/evil"` or `ext = "../../../home/user/.bashrc"`
/// to escape the configured output directory via downstream
/// `PathBuf::join` (which on POSIX *replaces* the buffer when the joined
/// segment is absolute — exactly the path-injection vector security review
/// M1 of PR #221 flagged).
///
/// Strip:
/// - Path separators (`/`, `\`) — neutralises both POSIX and Windows
///   traversal.
/// - Drive-letter prefix (`C:` etc) and namespace prefix (`\\?\`) — Windows
///   absolute-path forms.
/// - Null bytes — defensive against C-string truncation in any FFI path.
/// - Leading dots and whitespace — collapse `..`, `.foo`, ` foo` to safe
///   forms before joining.
///
/// Empty results collapse to a single underscore so downstream filename
/// formatters never receive a zero-length component.
///
/// This mirrors yt-dlp's `sanitize_filename` semantics conservatively
/// (strict-only mode; no Unicode look-alike substitution) since these
/// strings flow into rdlp's archive identity, not into user-visible
/// titles.
pub(crate) fn sanitise_for_path(s: &str) -> String {
    if s.is_empty() {
        return "_".to_string();
    }
    let cleaned: String = s
        .chars()
        .filter(|c| !matches!(*c, '/' | '\\' | '\0' | ':'))
        .collect();
    let trimmed = cleaned.trim_matches(|c: char| c.is_whitespace() || c == '.');
    if trimmed.is_empty() {
        "_".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Convert a WIT `Format` to `rdlp_types::Format`.
///
/// Numeric widening: WIT uses `f32` for `fps`/`tbr`/`vbr`/`abr`; rdlp-types
/// uses `f64`. `format_id` and `ext` are sanitised before they enter the
/// type — they're consumed by downstream filename formatters and must not
/// carry path separators or drive-letter prefixes (security review M1).
///
/// `parse::<DownloadProtocol>()` is `Infallible` (see
/// `rdlp_types::protocol::DownloadProtocol::from_str`): any string not
/// matching one of the five named variants becomes `Other(s)`, verbatim, by
/// design — that's how a plugin-supplied protocol like `"rtmp"` survives
/// this boundary at all. The `.unwrap_or(Https)` below therefore has no
/// live `Err` branch to fall back from; it is dead code, unchanged from the
/// pre-Task-5 `adapter.rs::convert_format` it was moved from.
pub(crate) fn format_from_wit(w: WitFormat) -> rdlp_types::Format {
    let protocol = w
        .protocol
        .parse::<DownloadProtocol>()
        .unwrap_or(DownloadProtocol::Https);
    let format_id = sanitise_for_path(&w.format_id);
    let ext = sanitise_for_path(&w.ext);
    let mut f = rdlp_types::Format::new(format_id, w.url, ext, protocol);
    f.width = w.width;
    f.height = w.height;
    f.fps = w.fps.map(f64::from);
    f.tbr = w.tbr.map(f64::from);
    f.vbr = w.vbr.map(f64::from);
    f.abr = w.abr.map(f64::from);
    f.vcodec = rdlp_types::Codec::from(w.vcodec);
    f.acodec = rdlp_types::Codec::from(w.acodec);
    f.container = w.container.as_deref().map(sanitise_for_path);
    f.filesize = w.filesize;
    f.format_note = w.format_note;
    f
}

/// Convert `rdlp_types::Format` to the WIT `Format` (used by the search
/// adapter and by tests that round-trip a format through the boundary).
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "first production caller: search adapter task")
)]
pub(crate) fn format_to_wit(f: &rdlp_types::Format) -> WitFormat {
    WitFormat {
        format_id: f.format_id.clone(),
        url: f.url.clone(),
        ext: f.ext.clone(),
        protocol: f.protocol.to_string(),
        width: f.width,
        height: f.height,
        fps: f.fps.map(narrow_f64),
        tbr: f.tbr.map(narrow_f64),
        vbr: f.vbr.map(narrow_f64),
        abr: f.abr.map(narrow_f64),
        vcodec: f.vcodec.as_str().map(str::to_owned),
        acodec: f.acodec.as_str().map(str::to_owned),
        container: f.container.clone(),
        filesize: f.filesize,
        format_note: f.format_note.clone(),
    }
}

/// Convert `rdlp_types::Fragment` to the WIT `host:extract-helpers`
/// `mpd-fragment` record.
///
/// `(start, end_exclusive)` byte-range convention mirrors
/// `rdlp_types::Fragment.byte_range` exactly — both tuples are passed
/// through verbatim. Plugins see the same end-exclusive semantics as the
/// host's downloader (see the WIT doc-comments on `mpd-fragment.byte-range`
/// / `.init-byte-range`).
pub(crate) fn fragment_to_wit(fr: rdlp_types::Fragment) -> MpdFragment {
    MpdFragment {
        url: fr.url,
        duration: fr.duration,
        byte_range: fr.byte_range,
        init_url: fr.init_url,
        init_byte_range: fr.init_byte_range,
    }
}

/// Convert the WIT `mpd-fragment` record back to `rdlp_types::Fragment`.
///
/// `mpd-fragment` has no `filesize` field, so the round-tripped `Fragment`
/// always carries `filesize: None` — pre-known segment size is rarely
/// populated and is not part of this boundary's contract.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "first production caller: a future guest-side extract-mpd consumer"
    )
)]
pub(crate) fn fragment_from_wit(w: MpdFragment) -> rdlp_types::Fragment {
    rdlp_types::Fragment {
        url: w.url,
        byte_range: w.byte_range,
        init_url: w.init_url,
        init_byte_range: w.init_byte_range,
        duration: w.duration,
        filesize: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{format_from_wit, format_to_wit, fragment_from_wit, fragment_to_wit};

    #[test]
    fn format_round_trips_through_wit_except_sanitised_fields() {
        let mut f = rdlp_types::Format::new(
            "hls-720p",
            "https://cdn.example/v.m3u8",
            "mp4",
            rdlp_types::DownloadProtocol::M3u8Native,
        );
        f.width = Some(1280);
        f.height = Some(720);
        f.fps = Some(29.97);
        f.tbr = Some(1280.0);
        f.vcodec = rdlp_types::Codec::from(Some("avc1.64001f".to_string()));
        f.acodec = rdlp_types::Codec::from(Some("mp4a.40.2".to_string()));
        f.filesize = Some(1234);
        f.format_note = Some("720p".into());
        f.container = Some("mp4".into());
        let back = format_from_wit(format_to_wit(&f));
        assert_eq!(back.format_id, f.format_id);
        assert_eq!(back.url, f.url);
        assert_eq!(back.protocol, f.protocol);
        assert_eq!(back.width, f.width);
        assert_eq!(back.height, f.height);
        assert_eq!(back.vcodec, f.vcodec);
        assert_eq!(back.acodec, f.acodec);
        assert_eq!(back.filesize, f.filesize);
        assert_eq!(back.format_note, f.format_note);
        assert!(
            (back.fps.unwrap() - 29.97).abs() < 1e-3,
            "f32 narrowing at the boundary"
        );
    }

    #[test]
    fn fragment_round_trips_including_ranges() {
        let fr = rdlp_types::Fragment {
            url: "https://cdn.example/s1.m4s".into(),
            byte_range: Some((0, 100)),
            init_url: Some("https://cdn.example/init.mp4".into()),
            init_byte_range: Some((0, 10)),
            duration: Some(6.0),
            filesize: None,
        };
        let back = fragment_from_wit(fragment_to_wit(fr.clone()));
        assert_eq!(back.url, fr.url);
        assert_eq!(back.byte_range, fr.byte_range);
        assert_eq!(back.init_url, fr.init_url);
        assert_eq!(back.init_byte_range, fr.init_byte_range);
        assert_eq!(back.duration, fr.duration);
    }

    #[test]
    fn unrecognised_protocol_string_round_trips_as_other() {
        // `DownloadProtocol::from_str` is `Infallible` (see
        // `rdlp_types::protocol`): a string matching none of the five named
        // variants becomes `Other(s)`, verbatim, by design — that's how a
        // plugin-supplied protocol like "rtmp" survives the boundary at all
        // (`rdlp_types::protocol`'s own tests assert `Other("rtmp")` as the
        // intended outcome). `format_from_wit`'s `.unwrap_or(Https)` can
        // never fire for that reason and is not a live fallback; asserting
        // the round-trip preserves the unrecognised string, rather than
        // asserting it collapses to `Https`, is the correct invariant here.
        let mut w = format_to_wit(&rdlp_types::Format::new(
            "x",
            "https://h/x",
            "mp4",
            rdlp_types::DownloadProtocol::Https,
        ));
        w.protocol = "not-a-protocol".into();
        assert_eq!(
            format_from_wit(w).protocol,
            rdlp_types::DownloadProtocol::Other("not-a-protocol".to_string())
        );
    }
}

#[cfg(test)]
mod sanitise_for_path_tests {
    use super::sanitise_for_path;

    #[test]
    fn empty_collapses_to_underscore() {
        assert_eq!(sanitise_for_path(""), "_");
    }

    #[test]
    fn pure_dots_or_whitespace_collapse_to_underscore() {
        assert_eq!(sanitise_for_path("..."), "_");
        assert_eq!(sanitise_for_path("   "), "_");
        assert_eq!(sanitise_for_path(". . . "), "_");
    }

    #[test]
    fn leading_slash_stripped_blocks_absolute_path_injection() {
        // The motivating M1 attack: malicious format_id = "/etc/passwd".
        // After sanitisation, downstream PathBuf::join cannot escape.
        assert_eq!(sanitise_for_path("/etc/passwd"), "etcpasswd");
        assert_eq!(sanitise_for_path("/"), "_");
    }

    #[test]
    fn windows_drive_letter_neutralised() {
        // `C:\Windows\System32` would PathBuf::join as an absolute Windows
        // path. Stripping `:` plus separators reduces it to a relative segment.
        assert_eq!(
            sanitise_for_path("C:\\Windows\\System32"),
            "CWindowsSystem32"
        );
    }

    #[test]
    fn null_bytes_stripped() {
        assert_eq!(sanitise_for_path("foo\0bar"), "foobar");
    }

    #[test]
    fn parent_directory_traversal_neutralised() {
        // "../../etc/passwd" — separators removed, leading dots stripped.
        assert_eq!(sanitise_for_path("../../etc/passwd"), "etcpasswd");
    }

    #[test]
    fn legitimate_format_ids_unchanged() {
        assert_eq!(sanitise_for_path("hls-1280"), "hls-1280");
        assert_eq!(sanitise_for_path("video-720p"), "video-720p");
        assert_eq!(sanitise_for_path("dash-fragments"), "dash-fragments");
        assert_eq!(sanitise_for_path("h264_aac_128k"), "h264_aac_128k");
    }
}
