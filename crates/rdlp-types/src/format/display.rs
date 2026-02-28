//! Display and table formatting methods for [`Format`].
//!
//! Contains the cached description builder and size text formatter.

use super::Format;

impl Format {
    /// Get a human-readable format description
    ///
    /// This method caches the description after first computation.
    /// Subsequent calls return a reference to the cached value.
    pub fn description(&self) -> &str {
        self.cached_description.get_or_init(|| {
            // At most 6 parts: note, resolution, fps, vcodec, acodec, ext
            let mut parts = Vec::with_capacity(6);

            if let Some(note) = &self.format_note {
                parts.push(note.clone());
            }

            if let Some(res) = self.resolution_string() {
                parts.push(res);
            }

            if let Some(fps) = self.fps {
                parts.push(format!("{fps}fps"));
            }

            if let Some(vcodec) = self.vcodec.as_deref().filter(|c| *c != "none") {
                parts.push(format!("vcodec:{vcodec}"));
            }

            if let Some(acodec) = self.acodec.as_deref().filter(|c| *c != "none") {
                parts.push(format!("acodec:{acodec}"));
            }

            parts.push(self.ext.clone());

            parts.join(" ")
        })
    }

    /// Returns the size column text for this format (e.g. `"837.9 MB"`, `"50:16 (754 seg)"`)
    #[must_use]
    pub fn size_text(&self) -> String {
        if self.is_hls() {
            let seg_count = self
                .fragments
                .as_ref()
                .map(|f| f.len() as u64)
                .or(self.filesize_approx);

            match (self.duration, seg_count) {
                (Some(dur), Some(segs)) => {
                    let mins = dur as u64 / 60;
                    let secs = dur as u64 % 60;
                    format!("{mins}:{secs:02} ({segs} seg)")
                }
                (Some(dur), None) => {
                    let mins = dur as u64 / 60;
                    let secs = dur as u64 % 60;
                    format!("{mins}:{secs:02}")
                }
                (None, Some(segs)) => format!("{segs} segments"),
                (None, None) => "HLS stream".to_string(),
            }
        } else if let Some(filesize) = self.filesize {
            let dur_suffix = self
                .duration
                .map(|dur| {
                    let mins = dur as u64 / 60;
                    let secs = dur as u64 % 60;
                    format!(" ({mins}:{secs:02})")
                })
                .unwrap_or_default();
            format!("{:.1} MB{dur_suffix}", filesize as f64 / (1024.0 * 1024.0))
        } else if let Some(filesize_approx) = self.filesize_approx {
            format!("~{:.0} MB", filesize_approx as f64 / (1024.0 * 1024.0))
        } else if let Some(dur) = self.duration {
            let mins = dur as u64 / 60;
            let secs = dur as u64 % 60;
            format!("{mins}:{secs:02}")
        } else {
            "Unknown".to_string()
        }
    }
}
