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

            if let Some(vcodec) = self.vcodec.as_str() {
                parts.push(format!("vcodec:{vcodec}"));
            }

            if let Some(acodec) = self.acodec.as_str() {
                parts.push(format!("acodec:{acodec}"));
            }

            parts.push(self.ext.clone());

            parts.join(" ")
        })
    }

    /// Returns the size column text for this format (e.g. `"837.9 MB"`, `"50:16 (754 seg)"`)
    ///
    /// # Casts
    ///
    /// The casts here are intentional: `dur as u64` truncates sub-second fractions for
    /// display purposes only; `u64 as f64` may lose precision on very large file sizes but
    /// the rounding is acceptable at MB/GB display granularity.
    #[allow(
        clippy::cast_possible_truncation, // dur as u64: sub-second truncation acceptable for display
        clippy::cast_sign_loss,           // dur as u64: duration is always non-negative
        clippy::cast_precision_loss       // u64 as f64: MB/GB display tolerates precision loss
    )]
    #[must_use]
    pub fn size_text(&self) -> String {
        if self.is_hls() {
            let seg_count = self.fragments.as_ref().map(|f| f.len() as u64);
            let size = self.filesize.or(self.filesize_approx);
            let dur_str = self.duration.map(|dur| {
                let mins = dur as u64 / 60;
                let secs = dur as u64 % 60;
                format!("{mins}:{secs:02}")
            });
            let size_str = size.map(|s| {
                let mb = s as f64 / (1024.0 * 1024.0);
                if mb >= 1024.0 {
                    format!("{:.1} GB", mb / 1024.0)
                } else {
                    format!("{mb:.0} MB")
                }
            });

            match (dur_str, seg_count, size_str) {
                (Some(d), Some(s), _) => format!("{d} ({s} seg)"),
                (Some(d), None, Some(sz)) => format!("{d} (~{sz})"),
                (Some(d), None, None) => d,
                (None, Some(s), _) => format!("{s} segments"),
                (None, None, Some(sz)) => format!("~{sz}"),
                (None, None, None) => "HLS stream".to_string(),
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
