//! # rdlp-jsinterp
//!
//! JavaScript execution engine for rdlp, backed by [Boa](https://boajs.dev/).
//!
//! Provides the [`BoaJsEngine`] implementation of the [`JsEngine`] trait
//! from `rdlp-core`. Each evaluation creates a fresh Boa `Context` with
//! browser polyfills (`window`, `atob`, `btoa`, `navigator`, `document`)
//! and runs synchronously inside [`tokio::task::spawn_blocking`].
//!
//! # Example
//!
//! ```no_run
//! use rdlp_jsinterp::BoaJsEngine;
//! use rdlp_core::JsEngine;
//!
//! # async fn example() -> rdlp_core::Result<()> {
//! let engine = BoaJsEngine::new();
//! let result = engine.eval("1 + 2").await?;
//! assert_eq!(result, serde_json::json!(3.0));
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

mod convert;
mod polyfills;

use async_trait::async_trait;
use boa_engine::{Context, JsString, JsValue, Source};
use log::debug;
use rdlp_core::{JsEngine, RdlpError, Result};

/// Boa-backed JavaScript engine.
///
/// Each call to `eval`, `eval_with_context`, or `call_function` creates
/// a fresh [`Context`] with browser polyfills injected. This avoids
/// cross-contamination between evaluations and sidesteps Boa's `!Send`
/// constraint by keeping the context confined to a single blocking task.
#[derive(Default)]
pub struct BoaJsEngine;

impl BoaJsEngine {
    /// Create a new Boa JavaScript engine instance.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Create a fresh Boa context with polyfills injected.
    fn make_context() -> std::result::Result<Context, String> {
        let mut ctx = Context::default();
        polyfills::inject_polyfills(&mut ctx)
            .map_err(|e| format!("Failed to inject polyfills: {e}"))?;
        Ok(ctx)
    }

    /// Evaluate code in a fresh context, returning the result as JSON.
    fn eval_sync(code: &str) -> std::result::Result<serde_json::Value, String> {
        let mut ctx = Self::make_context()?;
        let result = ctx
            .eval(Source::from_bytes(code.as_bytes()))
            .map_err(|e| format!("{e}"))?;
        convert::js_to_json(&result, &mut ctx).map_err(|e| format!("{e}"))
    }

    /// Evaluate code with context variables set as globals.
    fn eval_with_context_sync(
        code: &str,
        context: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let mut ctx = Self::make_context()?;

        // Set each top-level key in the context object as a global variable
        if let serde_json::Value::Object(map) = context {
            let global = ctx.global_object();
            for (key, value) in map {
                let js_value = convert::json_to_js(value, &mut ctx)
                    .map_err(|e| format!("Failed to convert context var '{key}': {e}"))?;
                global
                    .set(JsString::from(key.as_str()), js_value, true, &mut ctx)
                    .map_err(|e| format!("Failed to set global '{key}': {e}"))?;
            }
        }

        let result = ctx
            .eval(Source::from_bytes(code.as_bytes()))
            .map_err(|e| format!("{e}"))?;
        convert::js_to_json(&result, &mut ctx).map_err(|e| format!("{e}"))
    }

    /// Evaluate code that defines a function, then call it with args.
    fn call_function_sync(
        code: &str,
        function_name: &str,
        args: &[serde_json::Value],
    ) -> std::result::Result<serde_json::Value, String> {
        let mut ctx = Self::make_context()?;

        // Evaluate the code that defines the function
        ctx.eval(Source::from_bytes(code.as_bytes()))
            .map_err(|e| format!("Failed to evaluate function code: {e}"))?;

        // Look up the function on the global object
        let global = ctx.global_object();
        let func_value = global
            .get(JsString::from(function_name), &mut ctx)
            .map_err(|e| format!("Failed to get function '{function_name}': {e}"))?;

        let func = func_value
            .as_callable()
            .ok_or_else(|| format!("'{function_name}' is not a function"))?;

        // Convert args
        let js_args: Vec<JsValue> = args
            .iter()
            .map(|a| convert::json_to_js(a, &mut ctx))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to convert function args: {e}"))?;

        let result = func
            .call(&JsValue::undefined(), &js_args, &mut ctx)
            .map_err(|e| format!("Function '{function_name}' threw: {e}"))?;

        convert::js_to_json(&result, &mut ctx).map_err(|e| format!("{e}"))
    }
}

#[async_trait]
impl JsEngine for BoaJsEngine {
    async fn eval(&self, code: &str) -> Result<serde_json::Value> {
        let code = code.to_owned();
        debug!(len = code.len(); "Evaluating JavaScript");
        tokio::task::spawn_blocking(move || Self::eval_sync(&code))
            .await
            .map_err(|e| RdlpError::JavaScript(format!("Task join error: {e}")))?
            .map_err(RdlpError::JavaScript)
    }

    async fn eval_with_context(
        &self,
        code: &str,
        context: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let code = code.to_owned();
        let context = context.clone();
        debug!(len = code.len(); "Evaluating JavaScript with context");
        tokio::task::spawn_blocking(move || Self::eval_with_context_sync(&code, &context))
            .await
            .map_err(|e| RdlpError::JavaScript(format!("Task join error: {e}")))?
            .map_err(RdlpError::JavaScript)
    }

    async fn call_function(
        &self,
        function_name: &str,
        args: &[serde_json::Value],
    ) -> Result<serde_json::Value> {
        let function_name = function_name.to_owned();
        let args = args.to_vec();
        debug!(function:? = function_name; "Calling JavaScript function");

        // For call_function, the first arg is expected to be the code defining
        // the function, and remaining args are the function arguments.
        // However, the trait signature passes function_name separately from args.
        // We need to pass the code as part of args[0] or rethink.
        //
        // Convention: args[0] is the JS source code that defines the function,
        // args[1..] are the actual function arguments.
        if args.is_empty() {
            return Err(RdlpError::JavaScript(
                "call_function requires at least one arg (the JS source code)".to_string(),
            ));
        }

        let code = args[0]
            .as_str()
            .ok_or_else(|| {
                RdlpError::JavaScript(
                    "First arg to call_function must be a string (JS code)".into(),
                )
            })?
            .to_owned();
        let func_args = args[1..].to_vec();

        tokio::task::spawn_blocking(move || {
            Self::call_function_sync(&code, &function_name, &func_args)
        })
        .await
        .map_err(|e| RdlpError::JavaScript(format!("Task join error: {e}")))?
        .map_err(RdlpError::JavaScript)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_eval_arithmetic() {
        let engine = BoaJsEngine::new();
        let result = engine.eval("1 + 2").await.unwrap();
        assert_eq!(result, serde_json::json!(3.0));
    }

    #[tokio::test]
    async fn test_eval_string_concat() {
        let engine = BoaJsEngine::new();
        let result = engine.eval("'hello' + ' ' + 'world'").await.unwrap();
        assert_eq!(result, serde_json::json!("hello world"));
    }

    #[tokio::test]
    async fn test_eval_boolean() {
        let engine = BoaJsEngine::new();
        let result = engine.eval("true && false").await.unwrap();
        assert_eq!(result, serde_json::json!(false));
    }

    #[tokio::test]
    async fn test_eval_null() {
        let engine = BoaJsEngine::new();
        let result = engine.eval("null").await.unwrap();
        assert_eq!(result, serde_json::Value::Null);
    }

    #[tokio::test]
    async fn test_eval_object() {
        let engine = BoaJsEngine::new();
        let result = engine.eval("({name: 'test', value: 42})").await.unwrap();
        assert_eq!(result["name"], "test");
        assert_eq!(result["value"], 42.0);
    }

    #[tokio::test]
    async fn test_eval_array() {
        let engine = BoaJsEngine::new();
        let result = engine.eval("[1, 2, 3]").await.unwrap();
        assert_eq!(result, serde_json::json!([1.0, 2.0, 3.0]));
    }

    #[tokio::test]
    async fn test_eval_with_context() {
        let engine = BoaJsEngine::new();
        let ctx = serde_json::json!({"x": 10, "y": 20});
        let result = engine.eval_with_context("x + y", &ctx).await.unwrap();
        assert_eq!(result, serde_json::json!(30.0));
    }

    #[tokio::test]
    async fn test_eval_with_string_context() {
        let engine = BoaJsEngine::new();
        let ctx = serde_json::json!({"name": "world"});
        let result = engine
            .eval_with_context("'hello ' + name", &ctx)
            .await
            .unwrap();
        assert_eq!(result, serde_json::json!("hello world"));
    }

    #[tokio::test]
    async fn test_call_function() {
        let engine = BoaJsEngine::new();
        let code = "function add(a, b) { return a + b; }";
        let result = engine
            .call_function(
                "add",
                &[
                    serde_json::json!(code),
                    serde_json::json!(3),
                    serde_json::json!(4),
                ],
            )
            .await
            .unwrap();
        assert_eq!(result, serde_json::json!(7.0));
    }

    #[tokio::test]
    async fn test_call_function_with_string_manipulation() {
        let engine = BoaJsEngine::new();
        let code = r#"
            function reverse(s) {
                return s.split("").reverse().join("");
            }
        "#;
        let result = engine
            .call_function(
                "reverse",
                &[serde_json::json!(code), serde_json::json!("hello")],
            )
            .await
            .unwrap();
        assert_eq!(result, serde_json::json!("olleh"));
    }

    #[tokio::test]
    async fn test_polyfills_available() {
        let engine = BoaJsEngine::new();
        let result = engine.eval("typeof window !== 'undefined'").await.unwrap();
        assert_eq!(result, serde_json::json!(true));
    }

    #[tokio::test]
    async fn test_atob_btoa_via_engine() {
        let engine = BoaJsEngine::new();
        let result = engine.eval("atob(btoa('test string'))").await.unwrap();
        assert_eq!(result, serde_json::json!("test string"));
    }

    #[tokio::test]
    async fn test_eval_syntax_error() {
        let engine = BoaJsEngine::new();
        let result = engine.eval("function(").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_eval_reference_error() {
        let engine = BoaJsEngine::new();
        let result = engine.eval("undefinedVariable.property").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_context_isolation() {
        let engine = BoaJsEngine::new();

        // Set a variable in one eval
        engine.eval("var testVar = 42").await.unwrap();

        // It should NOT persist to the next eval (fresh context)
        let result = engine.eval("typeof testVar").await.unwrap();
        assert_eq!(result, serde_json::json!("undefined"));
    }
}
