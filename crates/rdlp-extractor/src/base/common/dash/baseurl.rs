//! BaseURL chain resolution per DASH §5.6.2.

use url::Url;

/// Resolve a BaseURL chain. Each level (MPD, AdaptationSet, Representation)
/// contributes at most one `<BaseURL>` — the first entry, since the DASH spec
/// lets a level carry several for CDN failover and CDN-rotation is not in
/// scope. Picking that first entry is the *caller's* responsibility: each
/// level is passed as an `Option<&str>` (`None` = the level adds no BaseURL),
/// so no owned copies of the unused failover entries are ever made. Each
/// present entry is joined against the previous endpoint via RFC 3986.
/// Returns the final base URL the segment URLs should be resolved against.
///
/// `mpd_url` is the URL the MPD itself was fetched from (the implicit
/// "level 0" BaseURL).
pub fn resolve_chain<'a, I>(mpd_url: &Url, levels: I) -> Url
where
    I: IntoIterator<Item = Option<&'a str>>,
{
    let mut current = mpd_url.clone();
    for first in levels.into_iter().flatten() {
        match current.join(first) {
            Ok(joined) => current = joined,
            Err(e) => {
                log::warn!(
                    "DASH BaseURL chain: failed to resolve {first:?} against {current}: {e}; skipping this level"
                );
            }
        }
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_chain_returns_mpd_url() {
        let mpd = Url::parse("https://cdn.example.com/manifest.mpd").unwrap();
        let resolved = resolve_chain(&mpd, std::iter::empty::<Option<&str>>());
        assert_eq!(resolved.as_str(), "https://cdn.example.com/manifest.mpd");
    }

    #[test]
    fn mpd_level_baseurl_replaces() {
        let mpd = Url::parse("https://cdn.example.com/manifest.mpd").unwrap();
        let resolved = resolve_chain(&mpd, [Some("https://cdn.example.com/segments/")]);
        assert_eq!(resolved.as_str(), "https://cdn.example.com/segments/");
    }

    #[test]
    fn relative_baseurl_resolves_against_mpd() {
        let mpd = Url::parse("https://cdn.example.com/path/manifest.mpd").unwrap();
        let resolved = resolve_chain(&mpd, [Some("segments/")]);
        assert_eq!(resolved.as_str(), "https://cdn.example.com/path/segments/");
    }

    #[test]
    fn three_level_chain() {
        let mpd = Url::parse("https://a.com/m.mpd").unwrap();
        let resolved = resolve_chain(&mpd, [Some("base/"), Some("video/"), Some("1080p/")]);
        assert_eq!(resolved.as_str(), "https://a.com/base/video/1080p/");
    }

    #[test]
    fn empty_inner_level_is_skipped() {
        let mpd = Url::parse("https://a.com/m.mpd").unwrap();
        // `None` level adds nothing; chain proceeds with the next level.
        let resolved = resolve_chain(&mpd, [None, Some("video/")]);
        assert_eq!(resolved.as_str(), "https://a.com/video/");
    }

    #[test]
    fn caller_selects_first_entry_of_multi_baseurl_level() {
        let mpd = Url::parse("https://a.com/m.mpd").unwrap();
        // A level may carry several <BaseURL> entries (CDN failover). The
        // caller selects the first via `.first()` — mirroring the exact
        // expression used in expand.rs — and resolve_chain resolves that one;
        // the fallback entry is never touched (and never cloned).
        let cdn_failover = ["primary/".to_string(), "fallback/".to_string()];
        let resolved = resolve_chain(&mpd, [cdn_failover.first().map(String::as_str)]);
        assert_eq!(resolved.as_str(), "https://a.com/primary/");
    }
}
