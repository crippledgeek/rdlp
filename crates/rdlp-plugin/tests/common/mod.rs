// `missing_docs` exempt because integration test helpers aren't public API.
#![allow(missing_docs)]

//! Re-exports of the crate's own test-support seams for this crate's
//! integration test binaries.
//!
//! `write_signed_plugin`/`SignedPluginSpec` used to exist as six
//! near-identical copies across five test files here; they now live once,
//! in `rdlp_plugin::test_support` (not `cfg(test)`, so an integration test
//! in *another* crate — `rdlp-api`'s bootstrap tests — can reuse the exact
//! same signer instead of hand-rolling a seventh copy). This module is kept
//! only so the many `mod common; use common::{...}` call sites in this
//! crate's own `tests/` binaries don't all need rewriting to the new path.

pub use rdlp_plugin::test_support::{SignedPluginSpec, extraction_ctx, write_signed_plugin};
