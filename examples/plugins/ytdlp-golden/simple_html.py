"""Synthetic yt-dlp-shape extractor — simple HTML scraping path.
Exercises: _download_webpage, _html_search_meta, _search_regex.
This is NOT a real yt-dlp extractor — it's a pipeline test fixture.
"""
from rdlp_ytdlp_compat import InfoExtractor


class SimpleHtmlIE(InfoExtractor):
    _VALID_URL = r'https?://example\.com/v/(?P<id>\d+)'

    def _real_extract(self, url):
        video_id = self._search_regex(self._VALID_URL, url, "video id", group="id")
        webpage = self._download_webpage(url, video_id, note="Downloading webpage")
        title = self._html_search_meta("og:title", webpage, "title", fatal=True)
        thumbnail = self._html_search_meta("og:image", webpage, "thumbnail")
        return {
            "id": video_id,
            "title": title,
            "thumbnail": thumbnail,
            "formats": [{
                "format_id": "default",
                "url": f"https://cdn.example.com/{video_id}.mp4",
                "ext": "mp4",
                "protocol": "https",
            }],
        }
