//! BaseURL chain resolution per DASH §5.6.2.

use url::Url;

/// Resolve a BaseURL chain. Each level (MPD, AdaptationSet, Representation)
/// may add zero or more `<BaseURL>` entries. Each is joined against the
/// previous endpoint via RFC 3986. Returns the final base URL the
/// segment URLs should be resolved against.
///
/// `mpd_url` is the URL the MPD itself was fetched from (the implicit
/// "level 0" BaseURL).
pub fn resolve_chain<'a, I>(mpd_url: &Url, levels: I) -> Url
where
    I: IntoIterator<Item = &'a [String]>,
{
    let mut current = mpd_url.clone();
    for level in levels {
        // DASH spec: each level may have multiple <BaseURL> entries
        // (CDN failover). We pick the first; CDN-rotation is not in scope.
        if let Some(first) = level.first() {
            match current.join(first) {
                Ok(joined) => current = joined,
                Err(_) => continue,
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
        let resolved = resolve_chain(&mpd, std::iter::empty());
        assert_eq!(resolved.as_str(), "https://cdn.example.com/manifest.mpd");
    }

    #[test]
    fn mpd_level_baseurl_replaces() {
        let mpd = Url::parse("https://cdn.example.com/manifest.mpd").unwrap();
        let mpd_levels: Vec<String> = vec!["https://cdn.example.com/segments/".into()];
        let resolved = resolve_chain(&mpd, [mpd_levels.as_slice()]);
        assert_eq!(resolved.as_str(), "https://cdn.example.com/segments/");
    }

    #[test]
    fn relative_baseurl_resolves_against_mpd() {
        let mpd = Url::parse("https://cdn.example.com/path/manifest.mpd").unwrap();
        let mpd_levels: Vec<String> = vec!["segments/".into()];
        let resolved = resolve_chain(&mpd, [mpd_levels.as_slice()]);
        assert_eq!(resolved.as_str(), "https://cdn.example.com/path/segments/");
    }

    #[test]
    fn three_level_chain() {
        let mpd = Url::parse("https://a.com/m.mpd").unwrap();
        let mpd_lvl: Vec<String> = vec!["base/".into()];
        let adapt_lvl: Vec<String> = vec!["video/".into()];
        let repr_lvl: Vec<String> = vec!["1080p/".into()];
        let resolved = resolve_chain(
            &mpd,
            [mpd_lvl.as_slice(), adapt_lvl.as_slice(), repr_lvl.as_slice()],
        );
        assert_eq!(resolved.as_str(), "https://a.com/base/video/1080p/");
    }
}
