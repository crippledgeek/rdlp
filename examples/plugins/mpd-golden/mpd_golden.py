"""Minimal yt-dlp-compat plugin exercising the MPD extract path.

Issue #253 — round-trip regression guard for `extract-mpd` host helper.
The plugin fetches a webpage that embeds an MPD URL via a `data-mpd`
attribute, then calls `_extract_mpd_formats_and_subtitles` against it.
Both the page and the MPD body come from `FetchFixtures` in the Rust
test harness — no network access at test time.
"""

from __future__ import annotations

from rdlp_ytdlp_compat.info_extractor import InfoExtractor


class MpdGoldenIE(InfoExtractor):
    _VALID_URL = r"https?://mpd-test\.example\.com/(?P<id>[a-z0-9]+)"

    def _real_extract(self, url):
        video_id = self._match_id(url)
        webpage = self._download_webpage(url, video_id)
        mpd_url = self._search_regex(
            r'data-mpd="([^"]+)"', webpage, "mpd url",
        )
        formats, subtitles = self._extract_mpd_formats_and_subtitles(
            mpd_url, video_id,
        )
        return {
            "id": video_id,
            "title": "MPD Golden Round-Trip",
            "formats": formats,
            "subtitles": subtitles,
        }
