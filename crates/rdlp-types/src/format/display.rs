//! Display and table formatting methods for [`Format`].
//!
//! Contains the cached description builder, size text formatter,
//! and the interactive selection table row renderer.

use std::fmt::Write;

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
    ///
    /// Call this on every format to compute `max` width, then pass that to [`Self::table_row`].
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

    /// Format as table row for interactive selection UI
    ///
    /// Returns a formatted string suitable for display in selection menus:
    /// `"720p         | 1280x720   | 245.3 MB     | MP4    | h264/aac"`
    ///
    /// `size_width` controls the size column padding (use [`Self::size_text`] across
    /// all formats to compute the appropriate value).
    ///
    /// Optimized to minimize heap allocations using pre-allocated buffer.
    pub fn table_row(&self, size_width: usize) -> String {
        // Pre-allocate buffer for typical row length (~80 chars)
        let mut buf = String::with_capacity(80);

        // Quality column: append fps when non-standard (e.g. "1080p60")
        let quality_base = self.format_note.as_deref().unwrap_or("unknown");
        let col_start = buf.len();
        match self.fps {
            Some(fps) if fps > 0.0 && (fps - 30.0).abs() > 1.0 => {
                let _ = write!(buf, "{quality_base}{fps:.0}");
                let col_len = buf.len() - col_start;
                for _ in col_len..rdlp_table::QUALITY_WIDTH {
                    buf.push(' ');
                }
                buf.push_str(" | ");
            }
            _ => {
                let _ = write!(buf, "{quality_base:<w$} | ", w = rdlp_table::QUALITY_WIDTH);
            }
        }

        // Resolution: avoid intermediate String allocation
        match (self.width, self.height) {
            (Some(w), Some(h)) => {
                let res_start = buf.len();
                let _ = write!(buf, "{w}x{h}");
                let res_len = buf.len() - res_start;
                for _ in res_len..rdlp_table::RESOLUTION_WIDTH {
                    buf.push(' ');
                }
            }
            _ => {
                let _ = write!(buf, "{:<w$}", "N/A", w = rdlp_table::RESOLUTION_WIDTH);
            }
        }
        buf.push_str(" | ");

        // Size column: write directly, pad to dynamic width
        let size_start = buf.len();
        buf.push_str(&self.size_text());
        let size_len = buf.len() - size_start;
        for _ in size_len..size_width {
            buf.push(' ');
        }
        buf.push_str(" | ");

        // Format type column: avoid to_uppercase() allocation for HLS
        let format_start = buf.len();
        if self.is_hls() {
            buf.push_str("HLS");
        } else {
            // Write uppercase directly
            for c in self.ext.chars() {
                buf.push(c.to_ascii_uppercase());
            }
        }
        let format_len = buf.len() - format_start;
        for _ in format_len..rdlp_table::TYPE_WIDTH {
            buf.push(' ');
        }
        buf.push_str(" | ");

        // Codecs column: write directly
        match (&self.vcodec, &self.acodec) {
            (Some(v), Some(a)) => {
                let _ = write!(buf, "{v}/{a}");
            }
            (Some(v), None) => {
                let _ = write!(buf, "{v} (video only)");
            }
            (None, Some(a)) => {
                let _ = write!(buf, "{a} (audio only)");
            }
            (None, None) => buf.push_str("Unknown"),
        }

        if self.has_drm.unwrap_or(false) {
            buf.push_str(" [DRM]");
        }

        buf
    }
}
