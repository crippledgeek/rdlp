//! One place for every WIT⇄rdlp conversion (`Format`, `Fragment`, `InfoDict`).
//!
//! Shared by [`crate::adapter`] (guest `extract` results) and
//! [`crate::host::extract_helpers`] (host-side `extract-mpd` fragment map).
//! Before this module existed, `adapter.rs` held its own `Format`/`InfoDict`
//! conversions inline, and `extract_mpd` (plus its own regression test)
//! separately built an `MpdFragment` struct literal by hand — two divergent
//! copies of the same WIT record construction. This module converges both
//! onto one path, so a fix to a conversion reaches every caller.

use crate::bindings::rdlp::plugin::host_extract_helpers::MpdFragment;
use crate::bindings::rdlp::plugin::types::Format as WitFormat;
use crate::metadata_adapter::MetadataCaps;
use crate::metadata_adapter::extras::extras_from_wit;
use rdlp_types::DownloadProtocol;

/// Upper bound on the `format` rows one WIT call may carry across the
/// boundary — a plugin's `extract` result, or the input list of the
/// `expand-hls` / `probe-format-sizes` host imports.
///
/// The rows are plugin-controlled and every one of them costs host work
/// downstream that runs OUTSIDE the plugin's own per-call timeout: the
/// orchestrator's `finish_extracted_formats` boundary fetches a playlist
/// for each fragments-less HLS row, and the host imports fan out one probe
/// per row. 256 is headroom over the largest ladder in common use —
/// yt-dlp's `YouTube` extractor lists on the order of a hundred formats for
/// one video — while keeping that downstream work bounded by a constant
/// rather than by the plugin. Excess rows are dropped from the tail with
/// one warning on the plugin's log target.
pub(crate) const MAX_PLUGIN_FORMATS: usize = 256;

/// Upper bound on the entries one `extract-playlist` page may carry across
/// the boundary — the host-owned playlist loop's own overall cap
/// (`rdlp_extractor::base::common::MAX_PLAYLIST_SIZE`) across every page of
/// one playlist, so a single plugin-controlled page can never itself exceed
/// what the loop would ever keep. Excess rows are dropped from the tail
/// with one warning on the plugin's log target, same as
/// [`MAX_PLUGIN_FORMATS`].
pub(crate) const MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES: usize =
    rdlp_extractor::base::common::MAX_PLAYLIST_SIZE;

/// Whom a conversion's diagnostics name and where they go: the plugin's
/// name for identity, its `log` target for the warning channel `host:log`
/// already uses (so a plugin author reading their plugin's log sees the
/// host's refusals next to their own lines).
#[derive(Debug, Clone, Copy)]
pub(crate) struct PluginOrigin<'a> {
    /// The plugin's manifest name — identity, never rendered to the user.
    pub plugin_name: &'a str,
    /// The plugin's `log` target (`PluginStoreData::log_target`).
    pub log_target: &'a str,
    /// `Manifest::display_name()` — what [`info_dict_from_wit`] puts in
    /// `InfoDict::extractor`. Display-only; identity stays on `plugin_name`.
    pub display_name: &'a str,
}

/// What varies between [`cap_plugin_formats`] and
/// [`cap_plugin_playlist_entries`]: which WIT call is reporting the row
/// count, how many rows are kept, and what to call a row in the warning.
/// The truncate-then-warn-once mechanism itself is shared in [`cap_rows`].
struct CapSpec<'a> {
    /// Names the WIT call in the warning, so an author can tell which list
    /// was cut.
    import: &'a str,
    /// Rows beyond this many are dropped.
    bound: usize,
    /// What to call one row in the warning (`"format rows"`, `"playlist
    /// entries"`).
    noun: &'a str,
}

/// Enforce `spec.bound` on a plugin-supplied row list, keeping the first
/// `spec.bound` rows and warning once, on `origin`'s log target, when any
/// are dropped.
fn cap_rows<T>(mut rows: Vec<T>, spec: &CapSpec<'_>, origin: &PluginOrigin<'_>) -> Vec<T> {
    if rows.len() > spec.bound {
        log::warn!(
            target: origin.log_target,
            "{}: plugin {} supplied {} {}; keeping the first {} and dropping the rest",
            spec.import,
            origin.plugin_name,
            rows.len(),
            spec.noun,
            spec.bound,
        );
        rows.truncate(spec.bound);
    }
    rows
}

/// Enforce [`MAX_PLUGIN_FORMATS`] on a plugin-supplied row list, keeping
/// the first `MAX_PLUGIN_FORMATS` rows. `import` names the WIT call in the
/// warning so an author can tell which list was cut.
pub(crate) fn cap_plugin_formats<T>(
    rows: Vec<T>,
    import: &str,
    origin: &PluginOrigin<'_>,
) -> Vec<T> {
    cap_rows(
        rows,
        &CapSpec {
            import,
            bound: MAX_PLUGIN_FORMATS,
            noun: "format rows",
        },
        origin,
    )
}

/// Enforce [`MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES`] on one `extract-playlist`
/// page's entries, keeping the first that many. `import` names the WIT
/// call in the warning so an author can tell which list was cut.
pub(crate) fn cap_plugin_playlist_entries<T>(
    rows: Vec<T>,
    import: &str,
    origin: &PluginOrigin<'_>,
) -> Vec<T> {
    cap_rows(
        rows,
        &CapSpec {
            import,
            bound: MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES,
            noun: "playlist entries",
        },
        origin,
    )
}

/// Narrow an `f64` to `f32` at the WIT boundary.
///
/// `types.wit` declares `fps`/`tbr`/`vbr`/`abr` as `f32`; rdlp-types uses
/// `f64` internally. Both `format_to_wit` and `extract_mpd`'s `MpdFormat`
/// construction route their narrowing through this one function, so it is
/// genuinely the one site the cast happens — `clippy::cast_possible_truncation`
/// has one rationale to check rather than one per call site.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the `record format` in `interface types` \
              (crates/rdlp-plugin/wit/types.wit) declares fps/tbr/vbr/abr as \
              f32; narrowing at the boundary is the contract, and `expect` \
              fails the build if the lint ever stops firing"
)]
pub(crate) const fn narrow_f64(v: f64) -> f32 {
    v as f32
}

/// Convert a bindgen-generated `InfoDict` to the rdlp-types `InfoDict`.
///
/// The WIT `InfoDict` does not carry `extractor` or `webpage_url` — those are
/// filled in from the call context (`origin.plugin_name` and `url`). The
/// format list is capped at [`MAX_PLUGIN_FORMATS`] here, at the boundary,
/// so nothing downstream ever sees more rows than that from a plugin.
pub(crate) fn info_dict_from_wit(
    w: crate::bindings::rdlp::plugin::types::InfoDict,
    url: &str,
    origin: &PluginOrigin<'_>,
) -> rdlp_types::InfoDict {
    let mut out = rdlp_types::InfoDict::new(
        w.id,
        w.title,
        origin.display_name,
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
    out.formats = cap_plugin_formats(w.formats, "extract", origin)
        .into_iter()
        .map(format_from_wit)
        .collect();
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

/// The per-call context [`info_dict_from_extraction`] needs beyond the
/// `extraction` payload: the request URL and diagnostics origin
/// [`info_dict_from_wit`] already takes, plus the metadata caps
/// [`extras_from_wit`] bounds `WitInfoDictExtra::extras` with. Grouped
/// into one value so the function stays at two positional parameters
/// instead of growing a fourth — see
/// `~/.claude/rules/limit-function-arguments.md`.
pub(crate) struct ExtractionSite<'a> {
    /// The request URL, passed through to [`info_dict_from_wit`].
    pub url: &'a str,
    /// The calling plugin's diagnostics origin.
    pub origin: PluginOrigin<'a>,
    /// Bounds for the `extras` mapping, from the call's `Config`.
    pub caps: &'a MetadataCaps,
}

/// Convert a bindgen-generated `WitThumbnail` (0.5.2 `extract-with-metadata`
/// extra) to the rdlp-types `Thumbnail`. Field-for-field: both sides agree
/// on `url`/`id`/`width`/`height`/`preference`.
fn thumbnail_from_wit(t: crate::metadata_adapter::WitThumbnail) -> rdlp_types::Thumbnail {
    rdlp_types::Thumbnail {
        url: t.url,
        id: t.id,
        width: t.width,
        height: t.height,
        preference: t.preference,
    }
}

/// Convert a 0.5.2 `extraction` (frozen `info-dict` core plus the typed
/// `info-dict-extra`) to the rdlp-types `InfoDict`.
///
/// Reuses [`info_dict_from_wit`] for the core so that conversion continues
/// to exist in exactly one place; the extra's typed fields are then copied
/// onto the result directly. `actors` is a bare `Vec` on both sides;
/// `thumbnails` collapses an empty list to `None`, matching
/// [`info_dict_from_wit`]'s existing `tags`/`categories` convention. The
/// open `extras` key/value tail goes through [`extras_from_wit`] under
/// `site.caps`, landing in `InfoDict::extra` — which is
/// `#[serde(flatten)]`, so each kept key is a top-level key of the dict's
/// JSON.
pub(crate) fn info_dict_from_extraction(
    w: crate::metadata_adapter::WitExtraction,
    site: &ExtractionSite<'_>,
) -> rdlp_types::InfoDict {
    let mut out = info_dict_from_wit(w.core, site.url, &site.origin);
    out.actors = w.extra.actors;
    out.channel = w.extra.channel;
    out.channel_url = w.extra.channel_url;
    out.age_limit = w.extra.age_limit;
    out.thumbnails = if w.extra.thumbnails.is_empty() {
        None
    } else {
        Some(
            w.extra
                .thumbnails
                .into_iter()
                .map(thumbnail_from_wit)
                .collect(),
        )
    };
    out.extra = extras_from_wit(w.extra.extras, site.caps, &site.origin);
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
/// this boundary at all.
pub(crate) fn format_from_wit(w: WitFormat) -> rdlp_types::Format {
    let Ok(protocol) = w.protocol.parse::<DownloadProtocol>();
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

/// Convert `rdlp_types::Format` to the WIT `Format`.
///
/// The outbound half of the boundary: `expand-hls` and `probe-format-sizes`
/// return their rows through it (`host::extract_helpers`).
#[must_use]
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

#[cfg(test)]
mod tests {
    use super::{format_from_wit, format_to_wit, fragment_to_wit};

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
        // Four distinct fractional values so a mixed-up mapping (e.g. a
        // vbr/abr swap in `format_to_wit`) fails this test instead of
        // silently passing on a coincidental equal value.
        f.tbr = Some(1280.11);
        f.vbr = Some(1000.22);
        f.abr = Some(128.33);
        f.vcodec = rdlp_types::Codec::from(Some("avc1.64001f".to_string()));
        f.acodec = rdlp_types::Codec::from(Some("mp4a.40.2".to_string()));
        f.filesize = Some(1234);
        f.format_note = Some("720p".into());
        f.container = Some("mp4".into());
        let back = format_from_wit(format_to_wit(&f));
        assert_eq!(back.format_id, f.format_id);
        assert_eq!(back.url, f.url);
        assert_eq!(back.ext, f.ext);
        assert_eq!(back.protocol, f.protocol);
        assert_eq!(back.width, f.width);
        assert_eq!(back.height, f.height);
        assert_eq!(back.vcodec, f.vcodec);
        assert_eq!(back.acodec, f.acodec);
        assert_eq!(back.filesize, f.filesize);
        assert_eq!(back.format_note, f.format_note);
        assert_eq!(back.container, f.container);
        // f32 narrowing at the boundary: tolerance, not exact equality.
        assert!((back.fps.unwrap() - 29.97).abs() < 1e-3, "fps");
        assert!((back.tbr.unwrap() - 1280.11).abs() < 1e-1, "tbr");
        assert!((back.vbr.unwrap() - 1000.22).abs() < 1e-1, "vbr");
        assert!((back.abr.unwrap() - 128.33).abs() < 1e-1, "abr");
    }

    #[test]
    fn fragment_to_wit_passes_every_field_through_verbatim() {
        let fr = rdlp_types::Fragment {
            url: "https://cdn.example/s1.m4s".into(),
            byte_range: Some((0, 100)),
            init_url: Some("https://cdn.example/init.mp4".into()),
            init_byte_range: Some((0, 10)),
            duration: Some(6.0),
            filesize: None,
        };
        let wit = fragment_to_wit(fr.clone());
        assert_eq!(wit.url, fr.url);
        assert_eq!(wit.byte_range, fr.byte_range);
        assert_eq!(wit.init_url, fr.init_url);
        assert_eq!(wit.init_byte_range, fr.init_byte_range);
        assert_eq!(wit.duration, fr.duration);
    }

    #[test]
    fn unrecognised_protocol_string_round_trips_as_other() {
        // `DownloadProtocol::from_str` is `Infallible` (see
        // `rdlp_types::protocol`): a string matching none of the five named
        // variants becomes `Other(s)`, verbatim, by design — that's how a
        // plugin-supplied protocol like "rtmp" survives the boundary at all
        // (`rdlp_types::protocol`'s own tests assert `Other("rtmp")` as the
        // intended outcome), so the invariant is that the round-trip
        // preserves the unrecognised string rather than collapsing it.
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
mod format_cap_tests {
    use super::{MAX_PLUGIN_FORMATS, format_to_wit, info_dict_from_wit};
    use crate::test_support::unit::{
        TEST_LOG_TARGET, captured_entry_containing, captured_logs, test_origin,
    };

    fn wit_info_with_formats(n: usize) -> crate::bindings::rdlp::plugin::types::InfoDict {
        let formats = (0..n)
            .map(|i| {
                format_to_wit(&rdlp_types::Format::new(
                    format!("f{i}"),
                    format!("https://cdn.example/{i}.mp4"),
                    "mp4",
                    rdlp_types::DownloadProtocol::Https,
                ))
            })
            .collect();
        crate::bindings::rdlp::plugin::types::InfoDict {
            id: "1".into(),
            title: "t".into(),
            url: None,
            thumbnail: None,
            description: None,
            uploader: None,
            uploader_id: None,
            upload_date: None,
            duration: None,
            view_count: None,
            like_count: None,
            tags: vec![],
            categories: vec![],
            formats,
            subtitles: vec![],
        }
    }

    /// Exactly `MAX_PLUGIN_FORMATS` rows all cross the boundary; one more is
    /// cut back to the bound (first rows kept) and reported on the plugin's
    /// log target, naming the call and the plugin.
    #[test]
    fn extract_result_formats_are_capped_at_the_bound_inclusive() {
        let logs = captured_logs();
        let at = info_dict_from_wit(
            wit_info_with_formats(MAX_PLUGIN_FORMATS),
            "https://example.com/v",
            &test_origin(),
        );
        assert_eq!(at.formats.len(), MAX_PLUGIN_FORMATS);

        let over = info_dict_from_wit(
            wit_info_with_formats(MAX_PLUGIN_FORMATS + 1),
            "https://example.com/v",
            &test_origin(),
        );
        assert_eq!(over.formats.len(), MAX_PLUGIN_FORMATS);
        assert_eq!(
            over.formats.first().map(|f| f.format_id.as_str()),
            Some("f0")
        );
        assert_eq!(
            over.formats.last().map(|f| f.format_id.as_str()),
            Some(format!("f{}", MAX_PLUGIN_FORMATS - 1).as_str())
        );
        let (target, msg) = captured_entry_containing(
            &logs,
            &format!(
                "extract: plugin test supplied {} format rows",
                MAX_PLUGIN_FORMATS + 1
            ),
        );
        assert_eq!(target, TEST_LOG_TARGET);
        assert!(msg.contains(&MAX_PLUGIN_FORMATS.to_string()), "{msg}");
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
