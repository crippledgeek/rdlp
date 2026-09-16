//! Columnar transposition cipher.

/// Undo a columnar transposition keyed by `key`'s sorted character order (empty key = identity).
///
/// Padded: a partial last row is filled with spaces, so `columnar_transpose`
/// then `columnar_untranspose` round-trips exactly only when `src.len()` is a
/// multiple of `key`'s character count (see
/// `untranspose_pads_a_partial_last_row_with_spaces`).
#[must_use]
pub fn columnar_untranspose(src: &[char], key: &str) -> Vec<char> {
    let column_count = key.chars().count();
    if column_count == 0 {
        return src.to_vec();
    }
    let row_count = src.len().div_ceil(column_count);

    // Build a 2D grid filled with spaces
    let mut grid: Vec<Vec<char>> = vec![vec![' '; column_count]; row_count];

    // Build sorted key-index map (sort by char code, preserving original index)
    let mut key_map: Vec<(char, usize)> = key.chars().enumerate().map(|(i, c)| (c, i)).collect();
    key_map.sort_by_key(|&(c, _)| c);

    // Fill grid column-by-column in sorted key order
    let mut src_iter = src.iter();
    for &(_, col_idx) in &key_map {
        for row in grid.iter_mut().take(row_count) {
            let Some(&ch) = src_iter.next() else {
                break;
            };
            if let Some(cell) = row.get_mut(col_idx) {
                *cell = ch;
            }
        }
    }

    // Read grid row-by-row
    let mut result = Vec::with_capacity(src.len());
    for row in &grid {
        result.extend(row);
    }
    result
}

/// The forward direction (used by round-trip tests and any encoder).
///
/// Padded the same way as [`columnar_untranspose`]: a partial last row is
/// filled with spaces.
#[must_use]
pub fn columnar_transpose(src: &[char], key: &str) -> Vec<char> {
    let column_count = key.chars().count();
    if column_count == 0 {
        return src.to_vec();
    }
    let row_count = src.len().div_ceil(column_count);
    let mut grid: Vec<Vec<char>> = vec![vec![' '; column_count]; row_count];

    // Fill row by row
    let mut src_iter = src.iter();
    for grid_row in &mut grid {
        for cell in grid_row.iter_mut().take(column_count) {
            let Some(&ch) = src_iter.next() else {
                break;
            };
            *cell = ch;
        }
    }

    // Build sorted key-index map
    let mut key_map: Vec<(char, usize)> = key.chars().enumerate().map(|(i, c)| (c, i)).collect();
    key_map.sort_by_key(|&(c, _)| c);

    // Read column by column in sorted key order
    let mut result = Vec::with_capacity(src.len());
    for &(_, col_idx) in &key_map {
        for grid_row in &grid {
            if let Some(&ch) = grid_row.get(col_idx) {
                result.push(ch);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columnar_roundtrip() {
        let plain: Vec<char> = "The quick brown fox jumps".chars().collect();
        let key = "secret";
        // 25 chars over a 6-column key pads the grid's last row with trailing
        // spaces (see `untranspose_pads_a_partial_last_row_with_spaces`); the
        // roundtrip is exact modulo that padding.
        let mut roundtripped = columnar_untranspose(&columnar_transpose(&plain, key), key);
        while roundtripped.last() == Some(&' ') {
            roundtripped.pop();
        }
        assert_eq!(roundtripped, plain);
    }

    #[test]
    fn empty_key_is_identity() {
        let s: Vec<char> = "abc".chars().collect();
        assert_eq!(columnar_untranspose(&s, ""), s);
        assert_eq!(columnar_transpose(&s, ""), s);
    }

    #[test]
    fn untranspose_pads_a_partial_last_row_with_spaces() {
        // 5 chars, key of 3 -> 2 rows; the grid's unfilled cell reads back as ' '
        let out = columnar_untranspose(&['a', 'b', 'c', 'd', 'e'], "abc");
        assert_eq!(out.len(), 6);
        assert!(out.contains(&' '));
    }

    #[test]
    fn non_ascii_key_round_trips() {
        // "kéy" is 3 chars but 4 UTF-8 bytes (é is 2 bytes) — column_count must
        // come from key.chars().count(), not key.len(), or the grid is built
        // with the wrong column count and the round trip is silently wrong.
        // 7 chars over a 3-column key pads the last row (see
        // `untranspose_pads_a_partial_last_row_with_spaces`); trim it like
        // `columnar_roundtrip` does.
        let plain: Vec<char> = "abcdefg".chars().collect();
        let key = "kéy";
        let mut roundtripped = columnar_untranspose(&columnar_transpose(&plain, key), key);
        while roundtripped.last() == Some(&' ') {
            roundtripped.pop();
        }
        assert_eq!(roundtripped, plain);
    }
}
