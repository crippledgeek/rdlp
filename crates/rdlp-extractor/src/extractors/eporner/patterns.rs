//! URL patterns for EPorner.

use lazy_regex::{Lazy, Regex, lazy_regex};

/// Captures the id from `/video-{id}/...`, `/hd-porn/{id}/...`, `/embed/{id}/...`
/// across any EPorner lang subdomain.
pub static VIDEO_URL: Lazy<Regex> = lazy_regex!(
    r"^https?://(?:[a-z]{2}\.)?(?:www\.)?eporner\.com/(?:video-|hd-porn/|embed/)([A-Za-z0-9]+)"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_video_path() {
        assert!(VIDEO_URL.is_match("https://www.eporner.com/video-svXh0Ne27Ig/harleysummers/"));
    }

    #[test]
    fn matches_hd_porn_path() {
        assert!(VIDEO_URL.is_match("https://www.eporner.com/hd-porn/95008/some-slug/"));
    }

    #[test]
    fn matches_embed_path() {
        assert!(VIDEO_URL.is_match("https://www.eporner.com/embed/svXh0Ne27Ig/"));
    }

    #[test]
    fn matches_lang_subdomain() {
        assert!(VIDEO_URL.is_match("https://de.eporner.com/video-abc/some-slug/"));
    }

    #[test]
    fn rejects_other_site() {
        assert!(!VIDEO_URL.is_match("https://www.xvideos.com/video123/slug/"));
    }
}
