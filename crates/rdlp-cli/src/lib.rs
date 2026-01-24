//! rdlp-cli library
//!
//! This library provides the high-level orchestration layer for rdlp,
//! coordinating extraction, download, and post-processing workflows.

#![warn(missing_docs)]

pub mod orchestrator;

pub use orchestrator::Orchestrator;
