//! # rdlp-crypto
//!
//! Cryptographic utilities for rdlp including PRNG-based URL decryption.
//!
//! This crate provides cryptographic primitives used by extractors:
//!
//! - **PRNG-based decryption**: Used by `XHamster` for format URL obfuscation
//! - **Hex encoding/decoding**: Utilities for handling hex-encoded ciphertext
//!
//! ## `XHamster` URL Decryption
//!
//! `XHamster` encrypts video format URLs by embedding hex-encoded ciphertext
//! in the URL path. The encryption uses one of 7 PRNG algorithms:
//!
//! ```rust
//! use rdlp_crypto::xhamster::decipher_format_url;
//!
//! // Decipher a hex-encoded URL path
//! if let Some(decrypted) = decipher_format_url("010000000041424344") {
//!     println!("Decrypted: {decrypted}");
//! }
//! ```

#![warn(missing_docs)]
#![warn(clippy::pedantic, clippy::nursery, clippy::indexing_slicing)]

pub mod prng;
pub mod xhamster;

pub use prng::ByteGenerator;
pub use xhamster::decipher_format_url;
