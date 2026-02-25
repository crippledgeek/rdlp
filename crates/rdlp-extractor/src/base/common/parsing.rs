//! Date/time and duration parsing utilities
//!
//! Helper methods for parsing ISO 8601 durations, dates, and various
//! human-readable duration formats.

use super::BaseExtractor;
use super::selectors::{ISO8601_DATE_PATTERN, ISO8601_DURATION_PATTERN};

impl BaseExtractor {
    // ========================================================================
    // Date/Time Parsing
    // ========================================================================

    /// Parse ISO 8601 duration to seconds
    ///
    /// Supports formats like:
    /// - PT30S (30 seconds)
    /// - PT5M (5 minutes)
    /// - PT1H (1 hour)
    /// - PT1H30M45S (1 hour, 30 minutes, 45 seconds)
    ///
    /// # Arguments
    /// * `duration_str` - ISO 8601 duration string
    ///
    /// # Returns
    /// Duration in seconds, `None` if parsing fails
    pub fn parse_iso8601_duration(duration_str: &str) -> Option<f64> {
        if !duration_str.starts_with("PT") {
            return None;
        }

        let caps = ISO8601_DURATION_PATTERN.captures(duration_str)?;

        let hours: f64 = caps
            .get(1)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0.0);
        let minutes: f64 = caps
            .get(2)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0.0);
        let seconds: f64 = caps
            .get(3)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0.0);

        Some(hours * 3600.0 + minutes * 60.0 + seconds)
    }

    /// Parse ISO 8601 date to YYYYMMDD format
    ///
    /// Supports formats like:
    /// - 2024-01-15
    /// - 2024-01-15T10:30:00Z
    ///
    /// # Arguments
    /// * `date_str` - ISO 8601 date string
    ///
    /// # Returns
    /// Date in YYYYMMDD format, `None` if parsing fails
    pub fn parse_iso8601_date(date_str: &str) -> Option<String> {
        let caps = ISO8601_DATE_PATTERN.captures(date_str)?;

        let year = caps.get(1)?.as_str();
        let month = caps.get(2)?.as_str();
        let day = caps.get(3)?.as_str();

        Some(format!("{year}{month}{day}"))
    }

    /// Parse duration from various string formats.
    ///
    /// Supports:
    /// - ISO 8601: `PT1H2M30S`, `PT5M`, `PT30S`
    /// - Colon-separated: `1:30`, `1:02:30`
    /// - Text format: `28min 18sec`, `5min`, `30sec`
    /// - Plain seconds as string: `893`
    ///
    /// # Arguments
    /// * `s` - Duration string in any supported format
    ///
    /// # Returns
    /// Duration in seconds, `None` if parsing fails
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// assert_eq!(BaseExtractor::parse_duration("PT1H2M30S"), Some(3750.0));
    /// assert_eq!(BaseExtractor::parse_duration("1:30"), Some(90.0));
    /// assert_eq!(BaseExtractor::parse_duration("28min 18sec"), Some(1698.0));
    /// assert_eq!(BaseExtractor::parse_duration("893"), Some(893.0));
    /// ```
    #[must_use]
    pub fn parse_duration(s: &str) -> Option<f64> {
        let s = s.trim();

        // Try ISO 8601 first
        if let Some(duration) = Self::parse_iso8601_duration(s) {
            return Some(duration);
        }

        // Try colon-separated format (1:30, 1:02:30)
        if s.contains(':') {
            let parts: Vec<&str> = s.split(':').collect();
            return match parts.len() {
                2 => {
                    let mins: f64 = parts[0].parse().ok()?;
                    let secs: f64 = parts[1].parse().ok()?;
                    Some(mins * 60.0 + secs)
                }
                3 => {
                    let hours: f64 = parts[0].parse().ok()?;
                    let mins: f64 = parts[1].parse().ok()?;
                    let secs: f64 = parts[2].parse().ok()?;
                    Some(hours * 3600.0 + mins * 60.0 + secs)
                }
                _ => None,
            };
        }

        // Try text format (28min 18sec)
        if let Some(duration) = Self::parse_text_duration(s) {
            return Some(duration);
        }

        // Try plain number
        s.parse().ok()
    }

    /// Parse text duration format like "28min 18sec" into seconds.
    ///
    /// Supports:
    /// - `28min 18sec`
    /// - `5min`
    /// - `30sec`
    /// - `1h 30min` (hour support)
    ///
    /// # Arguments
    /// * `text` - Duration text
    ///
    /// # Returns
    /// Duration in seconds, `None` if parsing fails
    #[must_use]
    pub fn parse_text_duration(text: &str) -> Option<f64> {
        let text = text.trim().to_lowercase();
        let mut total_seconds = 0.0;
        let mut found_any = false;

        // Extract hours
        if let Some(h_idx) = text.find('h') {
            let before = text[..h_idx].trim();
            if let Some(num_str) = before.split_whitespace().next_back()
                && let Ok(hours) = num_str.parse::<f64>()
            {
                total_seconds += hours * 3600.0;
                found_any = true;
            }
        }

        // Extract minutes
        if let Some(min_idx) = text.find("min") {
            let before = text[..min_idx].trim();
            if let Some(num_str) = before.split_whitespace().next_back()
                && let Ok(mins) = num_str.parse::<f64>()
            {
                total_seconds += mins * 60.0;
                found_any = true;
            }
        }

        // Extract seconds
        if let Some(sec_idx) = text.find("sec") {
            let before = text[..sec_idx].trim();
            if let Some(num_str) = before.split_whitespace().next_back()
                && let Ok(secs) = num_str.parse::<f64>()
            {
                total_seconds += secs;
                found_any = true;
            }
        }

        if found_any && total_seconds > 0.0 {
            Some(total_seconds)
        } else {
            None
        }
    }
}
