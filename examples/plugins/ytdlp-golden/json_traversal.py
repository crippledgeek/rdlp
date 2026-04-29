"""Synthetic yt-dlp-shape extractor — JSON XHR + nested traversal path.
Exercises: _download_webpage, _parse_json, traverse_obj, int_or_none, try_get.
"""
from rdlp_ytdlp_compat import InfoExtractor, traverse_obj, int_or_none, try_get


class JsonTraversalIE(InfoExtractor):
    _VALID_URL = r'https?://api\.example\.com/v(?P<id>\d+)'

    def _real_extract(self, url):
        video_id = self._search_regex(self._VALID_URL, url, "video id", group="id")
        api_url = f"https://api.example.com/videos/{video_id}.json"
        page = self._download_webpage(api_url, video_id, note="Downloading JSON")
        data = self._parse_json(page, video_id)

        title = traverse_obj(data, ("video", "title"), default="Untitled")
        duration = int_or_none(traverse_obj(data, ("video", "duration_ms")), scale=1000)
        uploader = try_get(data, lambda d: d["uploader"]["name"], expected_type=str)

        formats = []
        for stream in traverse_obj(data, ("video", "streams"), Ellipsis) or []:
            formats.append({
                "format_id": str(traverse_obj(stream, "id", default=len(formats))),
                "url": traverse_obj(stream, "url"),
                "ext": traverse_obj(stream, "container", default="mp4"),
                "protocol": "https",
                "tbr": int_or_none(traverse_obj(stream, "bitrate_kbps")),
                "width": int_or_none(traverse_obj(stream, "width")),
                "height": int_or_none(traverse_obj(stream, "height")),
            })

        return {
            "id": video_id,
            "title": title,
            "duration": duration,
            "uploader": uploader,
            "formats": formats,
        }
