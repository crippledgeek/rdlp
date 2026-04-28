//! KVS (Kernel Video Sharing) shared utilities.
//!
//! KVS is the upstream PHP platform powering many tube sites. Several
//! conventions are stock across deployments:
//!
//! - **`flashvars`** — inline JS object embedded in HTML. See
//!   [`flashvars`].
//! - **`/api/videofile.php`** obfuscation — Cyrillic homoglyph
//!   substitution + comma-split base64 on the playable `video_url`. See
//!   [`url_obfuscation`].
//! - **`/api/json/video/{lifetime}/{e6}/{e3}/{id}.json`** — per-video
//!   metadata endpoint with millions/thousands id-bucketing. See
//!   [`api`].
//! - **`file_formats`** packed string — `||{ext}|{WxH}|{duration}|
//!   {filesize}|{tnc}|{tnt}|{flag}` per variant; `_tr*` is the trailer
//!   preview. See [`file_formats`].
//!
//! Each submodule is a standalone unit: site extractors compose what
//! they need.

pub(crate) mod api;
pub(crate) mod file_formats;
pub(crate) mod flashvars;
pub(crate) mod url_obfuscation;

// Re-export the existing flashvars surface so callers don't need to
// adjust import paths after the directory split.
#[allow(unused_imports)]
pub(crate) use flashvars::{
    KvsFlashvars, KvsFormat, KvsMetadata, extract_kvs_formats, extract_kvs_metadata, is_kvs_page,
    parse_kvs_flashvars,
};
