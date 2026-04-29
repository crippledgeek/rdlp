"""Synthetic yt-dlp-shape extractor — HLS master playlist + fallback HTTP.
Exercises: _download_webpage, _extract_m3u8_formats, urljoin, unified_timestamp.
"""
from rdlp_ytdlp_compat import InfoExtractor, urljoin, unified_timestamp


class M3u8WithFallbackIE(InfoExtractor):
    _VALID_URL = r'https?://stream\.example\.com/(?P<id>[a-z0-9]+)'

    def _real_extract(self, url):
        video_id = self._search_regex(self._VALID_URL, url, "video id", group="id")
        webpage = self._download_webpage(url, video_id, note="Downloading playlist page")

        title = self._html_search_meta("og:title", webpage, "title", default=video_id)
        upload_date = self._html_search_meta("uploadDate", webpage, "upload date")
        timestamp = unified_timestamp(upload_date) if upload_date else None

        m3u8_relative = self._search_regex(
            r'data-hls="([^"]+)"', webpage, "m3u8 url", default=None
        )
        formats = []
        if m3u8_relative is not None:
            m3u8_url = urljoin(url, m3u8_relative)
            formats.extend(self._extract_m3u8_formats(
                m3u8_url, video_id, ext="mp4", m3u8_id="hls"
            ))

        # Fallback progressive download
        formats.append({
            "format_id": "progressive",
            "url": f"https://stream.example.com/{video_id}.mp4",
            "ext": "mp4",
            "protocol": "https",
        })

        return {
            "id": video_id,
            "title": title,
            "timestamp": timestamp,
            "formats": formats,
        }
