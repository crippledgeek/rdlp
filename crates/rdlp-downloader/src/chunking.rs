//! Intelligent chunk size calculation for optimal download performance
//!
//! This module implements power-of-two chunk sizing for:
//! - Memory alignment (OS pages, allocators, NTFS clusters)
//! - Predictable performance
//! - Better scalability across file sizes

use std::fmt;

/// Strategy for determining chunk size
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkSizeStrategy {
    /// Automatically calculate based on file size (recommended)
    Auto,
    /// Fixed chunk size in bytes (must be power of two)
    ///
    /// # Panics
    ///
    /// Panics if `size` is not a power of two. Use `is_power_of_two()` to validate.
    Fixed(usize),
    /// Legacy mode (chunk_count-based, deprecated)
    ///
    /// **Warning**: This mode does NOT guarantee power-of-two chunk sizes.
    /// Use only for backward compatibility with existing code.
    Legacy {
        /// Number of chunks to divide the file into
        chunk_count: usize,
    },
}

impl Default for ChunkSizeStrategy {
    fn default() -> Self {
        Self::Auto
    }
}

impl fmt::Display for ChunkSizeStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Fixed(size) => {
                if size % (1024 * 1024) == 0 {
                    write!(f, "fixed ({}MB)", size / (1024 * 1024))
                } else {
                    write!(f, "fixed ({}KB)", size / 1024)
                }
            }
            Self::Legacy { chunk_count } => {
                write!(f, "legacy ({chunk_count} chunks)")
            }
        }
    }
}

/// Calculate optimal chunk size based on file size
///
/// # Algorithm
///
/// 1. Target ~1024 chunks per file
/// 2. Clamp to sane bounds (64 KB - 8 MB)
/// 3. Round up to next power of two
///
/// # Benefits
///
/// - **Memory Alignment**: Power-of-two sizes align with:
///   - Windows page size (4 KB)
///   - NTFS cluster size (64 KB)
///   - Allocator boundaries
/// - **Predictable**: Same file size → same chunk size
/// - **Scalable**: Small files get small chunks, large files get larger chunks
///
/// # Examples
///
/// ```
/// use rdlp_downloader::chunk_size_for_file;
///
/// assert_eq!(chunk_size_for_file(5 * 1024 * 1024), 64 * 1024);        // 5 MB → 64 KB
/// assert_eq!(chunk_size_for_file(200 * 1024 * 1024), 256 * 1024);     // 200 MB → 256 KB
/// assert_eq!(chunk_size_for_file(1024 * 1024 * 1024), 1024 * 1024);   // 1 GB → 1 MB
/// assert_eq!(chunk_size_for_file(5 * 1024 * 1024 * 1024), 8 * 1024 * 1024); // 5 GB → 8 MB
/// ```
pub fn chunk_size_for_file(file_size: u64) -> usize {
    const MIN_CHUNK: u64 = 64 * 1024;        // 64 KB (NTFS cluster size)
    const MAX_CHUNK: u64 = 8 * 1024 * 1024;  // 8 MB (reasonable upper bound)

    // Aim for ~1024 chunks per file
    let target = file_size / 1024;

    // Clamp to sane bounds
    let clamped = target.clamp(MIN_CHUNK, MAX_CHUNK);

    // Round up to next power of two
    clamped.next_power_of_two() as usize
}

/// Calculate chunk information for a file
///
/// Returns (chunk_size, total_chunks)
pub fn calculate_chunks(file_size: u64, strategy: ChunkSizeStrategy) -> (usize, usize) {
    let chunk_size = match strategy {
        ChunkSizeStrategy::Auto => chunk_size_for_file(file_size),
        ChunkSizeStrategy::Fixed(size) => {
            // Validate power of two
            assert!(size.is_power_of_two(), "Fixed chunk size must be power of two");
            size
        }
        ChunkSizeStrategy::Legacy { chunk_count } => {
            // Legacy: divide file by chunk count (non-power-of-two)
            (file_size / chunk_count as u64) as usize
        }
    };

    let total_chunks = file_size.div_ceil(chunk_size as u64) as usize;

    (chunk_size, total_chunks)
}

/// Check if a number is a power of two
#[inline]
#[allow(dead_code)] // Used in tests and property tests
pub(crate) const fn is_power_of_two(n: usize) -> bool {
    n != 0 && (n & (n - 1)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_size_small_files() {
        // Files ≤ 50 MB should get 64 KB chunks
        assert_eq!(chunk_size_for_file(1024 * 1024), 64 * 1024);
        assert_eq!(chunk_size_for_file(5 * 1024 * 1024), 64 * 1024);
        assert_eq!(chunk_size_for_file(50 * 1024 * 1024), 64 * 1024);
    }

    #[test]
    fn test_chunk_size_medium_files() {
        // 200 MB should get 256 KB chunks
        assert_eq!(chunk_size_for_file(200 * 1024 * 1024), 256 * 1024);
    }

    #[test]
    fn test_chunk_size_large_files() {
        // 1 GB should get 1 MB chunks
        assert_eq!(chunk_size_for_file(1024 * 1024 * 1024), 1024 * 1024);

        // 5 GB should get 8 MB chunks (close to cap)
        assert_eq!(chunk_size_for_file(5 * 1024 * 1024 * 1024), 8 * 1024 * 1024);
    }

    #[test]
    fn test_chunk_size_huge_files() {
        // Files > 8 GB should cap at 8 MB chunks
        assert_eq!(chunk_size_for_file(20 * 1024 * 1024 * 1024), 8 * 1024 * 1024);
        assert_eq!(chunk_size_for_file(100 * 1024 * 1024 * 1024), 8 * 1024 * 1024);
    }

    #[test]
    fn test_all_chunk_sizes_are_power_of_two() {
        let test_sizes = vec![
            1024 * 1024,       // 1 MB
            10 * 1024 * 1024,      // 10 MB
            100 * 1024 * 1024,     // 100 MB
            500 * 1024 * 1024,     // 500 MB
            1024 * 1024 * 1024,    // 1 GB
            5 * 1024 * 1024 * 1024, // 5 GB
        ];

        for file_size in test_sizes {
            let chunk_size = chunk_size_for_file(file_size);
            assert!(
                is_power_of_two(chunk_size),
                "Chunk size {chunk_size} for file {file_size} is not power of two"
            );
        }
    }

    #[test]
    fn test_chunk_size_min_bound() {
        // Even tiny files should get at least 64 KB chunks
        assert_eq!(chunk_size_for_file(1024), 64 * 1024);
        assert_eq!(chunk_size_for_file(10 * 1024), 64 * 1024);
    }

    #[test]
    fn test_chunk_size_max_bound() {
        // Very large files should cap at 8 MB
        assert_eq!(chunk_size_for_file(u64::MAX), 8 * 1024 * 1024);
    }

    #[test]
    fn test_chunk_size_zero_file() {
        // Zero-size files should still return valid chunk size
        let (chunk_size, total_chunks) = calculate_chunks(0, ChunkSizeStrategy::Auto);
        assert_eq!(chunk_size, 64 * 1024); // Min chunk size
        assert_eq!(total_chunks, 0);        // No chunks needed
    }

    #[test]
    fn test_calculate_chunks_auto() {
        let file_size = 1024 * 1024 * 1024; // 1 GB
        let (chunk_size, total_chunks) = calculate_chunks(file_size, ChunkSizeStrategy::Auto);

        assert_eq!(chunk_size, 1024 * 1024); // 1 MB
        assert_eq!(total_chunks, 1024);
    }

    #[test]
    fn test_calculate_chunks_fixed() {
        let file_size = 1024 * 1024 * 1024; // 1 GB
        let (chunk_size, total_chunks) = calculate_chunks(
            file_size,
            ChunkSizeStrategy::Fixed(2 * 1024 * 1024) // 2 MB
        );

        assert_eq!(chunk_size, 2 * 1024 * 1024);
        assert_eq!(total_chunks, 512);
    }

    #[test]
    #[should_panic(expected = "must be power of two")]
    fn test_calculate_chunks_fixed_non_power_of_two() {
        calculate_chunks(1024 * 1024, ChunkSizeStrategy::Fixed(1000)); // Not power of two
    }

    #[test]
    fn test_calculate_chunks_legacy() {
        let file_size = 1024 * 1024 * 1024; // 1 GB
        let (chunk_size, total_chunks) = calculate_chunks(
            file_size,
            ChunkSizeStrategy::Legacy { chunk_count: 4 }
        );

        assert_eq!(total_chunks, 4);
        assert_eq!(chunk_size as u64 * 4, file_size);
    }

    #[test]
    fn test_calculate_chunks_handles_remainder() {
        let file_size = 100 * 1024 * 1024 + 512; // 100 MB + 512 bytes
        let chunk_size = chunk_size_for_file(file_size);
        let (_, total_chunks) = calculate_chunks(file_size, ChunkSizeStrategy::Auto);

        // Should round up to cover all bytes
        assert!(total_chunks * chunk_size >= file_size as usize);
    }

    #[test]
    fn test_is_power_of_two() {
        assert!(is_power_of_two(1));
        assert!(is_power_of_two(2));
        assert!(is_power_of_two(64));
        assert!(is_power_of_two(1024));
        assert!(is_power_of_two(1024 * 1024));

        assert!(!is_power_of_two(0));
        assert!(!is_power_of_two(3));
        assert!(!is_power_of_two(100));
        assert!(!is_power_of_two(1000));
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_chunk_size_always_power_of_two(
            file_size in 1u64..100_000_000_000u64 // 1 byte to 100 GB
        ) {
            let chunk_size = chunk_size_for_file(file_size);
            prop_assert!(is_power_of_two(chunk_size));
        }

        #[test]
        fn test_chunk_size_within_bounds(
            file_size in 1u64..100_000_000_000u64
        ) {
            let chunk_size = chunk_size_for_file(file_size);
            prop_assert!(chunk_size >= 64 * 1024);
            prop_assert!(chunk_size <= 8 * 1024 * 1024);
        }

        #[test]
        fn test_calculate_chunks_covers_file(
            file_size in 1u64..10_000_000_000u64,
            strategy in prop_oneof![
                Just(ChunkSizeStrategy::Auto),
                (1usize..=10).prop_map(|n| ChunkSizeStrategy::Legacy { chunk_count: n }),
            ]
        ) {
            let (chunk_size, total_chunks) = calculate_chunks(file_size, strategy);

            // Total chunks * chunk_size should cover at least file_size
            prop_assert!(total_chunks as u64 * chunk_size as u64 >= file_size);

            // But not be more than one chunk too large
            prop_assert!(((total_chunks - 1) as u64 * chunk_size as u64) < file_size);
        }

        #[test]
        fn test_chunk_size_stable_for_same_input(
            file_size in 1u64..100_000_000_000u64
        ) {
            let chunk1 = chunk_size_for_file(file_size);
            let chunk2 = chunk_size_for_file(file_size);
            prop_assert_eq!(chunk1, chunk2, "Chunk size should be deterministic");
        }
    }
}
