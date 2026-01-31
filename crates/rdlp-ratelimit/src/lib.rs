//! Async token-bucket rate limiter for download bandwidth throttling.
//!
//! Provides a global rate limit shared across all parallel connections.
//! Uses a token-bucket algorithm with async sleep for precise throttling.
//!
//! # Example
//!
//! ```rust
//! use rdlp_ratelimit::{RateLimiter, parse_rate_limit};
//!
//! let bps = parse_rate_limit("1M").unwrap();
//! assert_eq!(bps, 1_048_576);
//!
//! let limiter = RateLimiter::new(bps);
//! // limiter.acquire(16384).await; // throttle before next read
//! ```

#![warn(missing_docs)]

mod limiter;
mod parse;

pub use limiter::RateLimiter;
pub use parse::parse_rate_limit;
