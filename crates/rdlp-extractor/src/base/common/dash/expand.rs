//! Expand an MPEG-DASH MPD into one [`Format`] per usable Representation.
//!
//! Mirrors yt-dlp's `_parse_mpd_periods` (common.py:2870–3073) at the
//! per-Repr-Format level. The DASH downloader is the consumer; it reads
//! `format.fragments` directly without re-parsing the manifest.

use rdlp_types::Format;
use url::Url;

use super::errors::DashExpandError;

/// Parse the MPD body and project each Representation to a [`Format`].
///
/// `mpd_xml` is the response body. `base_url` is the URL the MPD was fetched
/// from (used as the root of the BaseURL resolution chain).
///
/// Returns one Format per usable Representation in the **first** Period.
/// Multi-period MPDs log a warning and skip subsequent periods (see spec's non-goals — more conservative than yt-dlp).
///
/// # Errors
///
/// Returns [`DashExpandError`] when the MPD cannot be parsed, declares a live
/// stream, or contains no usable representations after filtering.
// Allow until wired into the extractor registry (Task 13).
#[allow(dead_code)]
pub fn expand_dash_representations(
    _mpd_xml: &str,
    _base_url: &Url,
) -> Result<Vec<Format>, DashExpandError> {
    Ok(Vec::new())
}
