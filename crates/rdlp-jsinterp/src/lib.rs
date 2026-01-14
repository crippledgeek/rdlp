//! # rdlp-jsinterp
//!
//! JavaScript execution engine for rdlp.
//!
//! This crate provides JavaScript execution capabilities using boa_engine,
//! primarily for YouTube signature decryption.

use async_trait::async_trait;
use rdlp_core::{JsEngine, Result, RdlpError};

/// Simple JS engine stub (will be properly implemented in Phase 3)
pub struct SimpleJsEngine {
    // Will use boa_engine in Phase 3
}

impl SimpleJsEngine {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for SimpleJsEngine {
    fn default() -> Self {
        Self::new()
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
