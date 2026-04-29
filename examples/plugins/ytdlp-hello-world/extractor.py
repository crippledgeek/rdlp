"""Hello-world Python plugin — proves the sync-WIT contract works for Python guests.

componentize-py free-function exports collapse into ONE Protocol subclass named
after the world (PascalCase). Errors raise via Err(ExtractError_*), NOT returned —
the Protocol method's return type is the Ok payload only.
"""
from extractor_plugin import ExtractorPlugin as _ExtractorPluginProtocol
from extractor_plugin.types import Err
from extractor_plugin.imports import host_log, host_fetch
from extractor_plugin.imports.host_fetch import Request
from extractor_plugin.imports.types import (
    InfoDict,
    Format,
    PluginInfo,
    SearchPage,
    ExtractError_Internal,
    SearchError_Unsupported,
)


# componentize-py looks up a concrete class named after the --world-module
# (`extractor_plugin` → `ExtractorPlugin`) in the app module, instantiates it
# without arguments, and dispatches WIT exports to its methods. The Protocol
# defined in `extractor_plugin/__init__.py` is the contract — we override every
# abstract method here.
class ExtractorPlugin(_ExtractorPluginProtocol):
    def metadata(self) -> PluginInfo:
        return PluginInfo(
            name="hello-world",
            version="0.1.0",
            wit_version="0.1.0",
            matches=["https://example.com/*"],
            url_regex=None,
            priority=150,
            claims_override=[],
            supports_search=False,
        )

    def extract(self, url: str) -> InfoDict:
        host_log.log(host_log.Level.INFO, f"hello-world extract called for {url}")
        req = Request(url=url, method="GET", headers=[], body=None, timeout_ms=10000)
        try:
            resp = host_fetch.fetch(req)
        except Exception as e:
            raise Err(ExtractError_Internal(f"fetch failed: {e}"))
        host_log.log(
            host_log.Level.INFO,
            f"fetched {len(resp.body)} bytes, status {resp.status}",
        )
        return InfoDict(
            id="hello-1",
            title=f"Hello {url}",
            url=None,
            formats=[
                Format(
                    format_id="dummy",
                    url=url,
                    ext="mp4",
                    protocol="https",
                    width=None,
                    height=None,
                    fps=None,
                    tbr=None,
                    vbr=None,
                    abr=None,
                    vcodec=None,
                    acodec=None,
                    container=None,
                    filesize=None,
                    format_note=None,
                )
            ],
            subtitles=[],
            thumbnail=None,
            description=None,
            uploader=None,
            uploader_id=None,
            upload_date=None,
            duration=None,
            view_count=None,
            like_count=None,
            tags=[],
            categories=[],
        )

    def search(self, query) -> SearchPage:
        raise Err(SearchError_Unsupported())
