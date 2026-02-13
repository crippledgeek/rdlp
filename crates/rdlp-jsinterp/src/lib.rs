//! # rdlp-jsinterp
//!
//! JavaScript execution engine for rdlp.
//!
//! This crate provides JavaScript execution capabilities using boa_engine,
//! primarily for YouTube signature decryption.

#![warn(missing_docs)]

use async_trait::async_trait;
use rdlp_core::{JsEngine, RdlpError, Result};

/// Simple JS engine stub (will be properly implemented in Phase 3)
#[derive(Default)]
pub struct SimpleJsEngine;

impl SimpleJsEngine {
    /// Create a new JavaScript engine instance
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl JsEngine for SimpleJsEngine {
    async fn eval(&self, _code: &str) -> Result<serde_json::Value> {
        Err(RdlpError::JavaScript(
            "JS engine not yet implemented".to_string(),
        ))
    }

    async fn eval_with_context(
        &self,
        _code: &str,
        _context: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        Err(RdlpError::JavaScript(
            "JS engine not yet implemented".to_string(),
        ))
    }

    async fn call_function(
        &self,
        _function_name: &str,
        _args: &[serde_json::Value],
    ) -> Result<serde_json::Value> {
        Err(RdlpError::JavaScript(
            "JS engine not yet implemented".to_string(),
        ))
    }
}
