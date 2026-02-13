//! Download archive for tracking completed downloads.
//!
//! Records completed downloads as `{extractor} {id}` lines in a text file.
//! On subsequent runs, already-present entries are skipped. Compatible with
//! yt-dlp's `--download-archive` format.

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// Load archive entries from a file into a `HashSet`.
///
/// Returns an empty set if the file does not exist. Blank lines and lines
/// starting with `#` are ignored.
pub fn load_archive(path: &Path) -> HashSet<String> {
    let Ok(file) = std::fs::File::open(path) else {
        return HashSet::new();
    };

    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .map(|line| line.trim().to_string())
        .collect()
}

/// Check whether a video is already recorded in the archive.
pub fn is_in_archive(archive: &HashSet<String>, extractor: &str, id: &str) -> bool {
    archive.contains(&format!("{extractor} {id}"))
}

/// Append a completed download entry to the archive file.
///
/// Creates the file (and parent directories) if they don't exist.
pub fn record_in_archive(path: &Path, extractor: &str, id: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let mut file = OpenOptions::new().append(true).create(true).open(path)?;

    writeln!(file, "{extractor} {id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn load_empty_set_when_file_missing() {
        let archive = load_archive(Path::new("/nonexistent/archive.txt"));
        assert!(archive.is_empty());
    }

    #[test]
    fn load_skips_blank_and_comment_lines() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "# rdlp download archive").unwrap();
        writeln!(tmp).unwrap();
        writeln!(tmp, "SiteA abc123").unwrap();
        writeln!(tmp, "  # another comment  ").unwrap();
        writeln!(tmp, "SiteB xyz789").unwrap();
        tmp.flush().unwrap();

        let archive = load_archive(tmp.path());
        assert_eq!(archive.len(), 2);
        assert!(archive.contains("SiteA abc123"));
        assert!(archive.contains("SiteB xyz789"));
    }

    #[test]
    fn is_in_archive_checks_extractor_and_id() {
        let mut archive = HashSet::new();
        archive.insert("SiteA abc123".to_string());

        assert!(is_in_archive(&archive, "SiteA", "abc123"));
        assert!(!is_in_archive(&archive, "SiteA", "other"));
        assert!(!is_in_archive(&archive, "SiteB", "abc123"));
    }

    #[test]
    fn record_appends_entry() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        record_in_archive(&path, "SiteA", "abc123").unwrap();
        record_in_archive(&path, "SiteB", "xyz789").unwrap();

        let archive = load_archive(&path);
        assert_eq!(archive.len(), 2);
        assert!(is_in_archive(&archive, "SiteA", "abc123"));
        assert!(is_in_archive(&archive, "SiteB", "xyz789"));
    }

    #[test]
    fn record_creates_file_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("archive.txt");

        record_in_archive(&path, "SiteA", "id1").unwrap();

        let archive = load_archive(&path);
        assert!(is_in_archive(&archive, "SiteA", "id1"));
    }
}
