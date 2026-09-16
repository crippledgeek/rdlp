//! `Config::playlist_items` as a value object: which 1-based playlist
//! positions to resolve.
//!
//! The syntax is the subset of yt-dlp's `--playlist-items` that needs no
//! knowledge of the playlist's length: `N`, `A-B`, `A:B` (inclusive),
//! `A-`/`A:` (open). Negative indices and `::step` need the total count
//! before any page is fetched, which the host-driven page loop deliberately
//! does not have; they are rejected.

use std::fmt;

/// Which 1-based playlist positions [`Config::playlist_items`](crate::Config::playlist_items)
/// selects, as a set of inclusive ranges. Construct via [`PlaylistItems::parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistItems {
    ranges: Vec<(usize, Option<usize>)>,
}

/// Why a `playlist_items` spec was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaylistItemsError {
    /// An item between commas (or the whole spec) was empty.
    Empty,
    /// An index was not a base-10 unsigned integer.
    NotANumber(String),
    /// An index was `0`; positions are 1-based.
    Zero,
    /// A range's end was before its start.
    Reversed {
        /// The range's start.
        start: usize,
        /// The range's end.
        end: usize,
    },
    /// A syntax form this subset does not support (a negative index or a
    /// `::step`) — see the module doc for why.
    Unsupported(String),
}

impl fmt::Display for PlaylistItemsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "empty item in playlist_items spec"),
            Self::NotANumber(s) => write!(f, "{s:?} is not a valid playlist index"),
            Self::Zero => write!(f, "playlist indices are 1-based; 0 is not valid"),
            Self::Reversed { start, end } => {
                write!(f, "range {start}-{end} is reversed")
            }
            Self::Unsupported(s) => {
                write!(
                    f,
                    "{s:?} is not supported (negative indices and ::step need the playlist length, which is not known before any page is fetched)"
                )
            }
        }
    }
}

impl std::error::Error for PlaylistItemsError {}

impl PlaylistItems {
    /// Parse a comma-separated `playlist_items` spec.
    ///
    /// # Errors
    ///
    /// Returns [`PlaylistItemsError`] if any item is empty, non-numeric,
    /// zero, a reversed range, or a negative-index / `::step` form this
    /// subset does not support.
    pub fn parse(spec: &str) -> Result<Self, PlaylistItemsError> {
        let mut ranges = Vec::new();
        for item in spec.split(',') {
            let item = item.trim();
            if item.is_empty() {
                return Err(PlaylistItemsError::Empty);
            }
            if item.starts_with('-') {
                return Err(PlaylistItemsError::Unsupported(item.to_string()));
            }
            if item.matches(['-', ':']).count() > 1 {
                return Err(PlaylistItemsError::Unsupported(item.to_string()));
            }
            let (a, b) = match item.split_once(['-', ':']) {
                None => (item, Some(item)),
                Some((a, "")) => (a, None),
                Some((a, b)) => (a, Some(b)),
            };
            let start = parse_index(a)?;
            let end = b.map(parse_index).transpose()?;
            if let Some(e) = end
                && e < start
            {
                return Err(PlaylistItemsError::Reversed { start, end: e });
            }
            ranges.push((start, end));
        }
        Ok(Self { ranges })
    }

    /// Whether the 1-based position `i` is selected.
    #[must_use]
    pub fn contains(&self, i: usize) -> bool {
        self.ranges
            .iter()
            .any(|&(s, e)| i >= s && e.is_none_or(|e| i <= e))
    }

    /// The highest position any range bounds, or `None` if any range is
    /// open-ended (`A-`/`A:`).
    #[must_use]
    pub fn max_index(&self) -> Option<usize> {
        self.ranges
            .iter()
            .map(|&(_, e)| e)
            .collect::<Option<Vec<_>>>()
            .and_then(|v| v.into_iter().max())
    }
}

fn parse_index(s: &str) -> Result<usize, PlaylistItemsError> {
    let n: usize = s
        .parse()
        .map_err(|_| PlaylistItemsError::NotANumber(s.to_string()))?;
    if n == 0 {
        return Err(PlaylistItemsError::Zero);
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_and_ranges_parse() {
        let p = PlaylistItems::parse("1,3-5,8:9,12-").unwrap();
        for i in [1, 3, 4, 5, 8, 9, 12, 500] {
            assert!(p.contains(i), "{i}");
        }
        for i in [2, 6, 7, 10, 11] {
            assert!(!p.contains(i), "{i}");
        }
        assert_eq!(p.max_index(), None);
    }

    #[test]
    fn bounded_max_index() {
        assert_eq!(PlaylistItems::parse("2,4-6").unwrap().max_index(), Some(6));
    }

    #[test]
    fn zero_is_rejected() {
        assert!(PlaylistItems::parse("0").is_err());
    }

    #[test]
    fn reversed_range_rejected() {
        assert!(PlaylistItems::parse("5-3").is_err());
    }

    #[test]
    fn negative_rejected() {
        assert!(PlaylistItems::parse("-3").is_err());
    }

    #[test]
    fn step_rejected() {
        assert!(PlaylistItems::parse("1:9:2").is_err());
    }

    #[test]
    fn empty_item_rejected() {
        assert!(PlaylistItems::parse("1,,2").is_err());
    }

    #[test]
    fn garbage_rejected() {
        assert!(PlaylistItems::parse("a-b").is_err());
    }

    /// A second `-`/`:` separator (mixed or not) must be refused the same
    /// deliberate way `::step` is, not fall through to a `NotANumber` on the
    /// leftover fragment (`"1-2-3"` used to parse `a="1"`, then fail parsing
    /// `"2-3"` as an integer — an accident, not a decision).
    #[test]
    fn multi_separator_rejected_as_unsupported() {
        for spec in ["1-2-3", "1:2-3", "1-2:3"] {
            let err = PlaylistItems::parse(spec).expect_err(spec);
            assert!(
                matches!(err, PlaylistItemsError::Unsupported(_)),
                "{spec}: got {err:?}"
            );
        }
    }

    /// The refusal is decided before any page is fetched (`Config::validate`
    /// and the playlist loop's `validate_selection` both run first), and
    /// the message says so — not "this page", which named a page that does
    /// not exist yet.
    #[test]
    fn unsupported_message_names_the_moment_it_is_decided() {
        let err = PlaylistItems::parse("1::2").expect_err("::step is unsupported");
        assert!(
            err.to_string()
                .contains("not known before any page is fetched"),
            "{err}"
        );
    }
}
