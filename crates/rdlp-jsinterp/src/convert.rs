//! Conversion between Boa `JsValue` and `serde_json::Value`.
//!
//! Boa's `JsValue` lives in a `Context` and cannot be serialized directly.
//! These helpers convert between the two representations so the `JsEngine`
//! trait can return standard JSON.

use boa_engine::{Context, JsNativeError, JsResult, JsString, JsValue, Source};
use serde_json::Value as JsonValue;

/// Convert a Boa `JsValue` to a `serde_json::Value`.
///
/// Handles primitives, arrays, and plain objects. Functions and symbols
/// are converted to `null` since they have no JSON representation.
pub fn js_to_json(value: &JsValue, ctx: &mut Context) -> JsResult<JsonValue> {
    if value.is_undefined() || value.is_null() {
        return Ok(JsonValue::Null);
    }

    if let Some(b) = value.as_boolean() {
        return Ok(JsonValue::Bool(b));
    }

    if let Some(n) = value.as_number() {
        return Ok(serde_json::Number::from_f64(n)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null));
    }

    if value.is_string() {
        if let Ok(s) = value.to_string(ctx) {
            return Ok(JsonValue::String(s.to_std_string_escaped()));
        }
    }

    if let Some(obj) = value.as_object() {
        // Check if it's an array
        if obj.is_array() {
            let len = obj.get(JsString::from("length"), ctx)?.to_u32(ctx)?;
            let mut arr = Vec::with_capacity(len as usize);
            for i in 0..len {
                let elem = obj.get(i, ctx)?;
                arr.push(js_to_json(&elem, ctx)?);
            }
            return Ok(JsonValue::Array(arr));
        }

        // Plain object — use JSON.stringify for reliable conversion
        let json_global = ctx.global_object().get(JsString::from("JSON"), ctx)?;
        if let Some(json_obj) = json_global.as_object() {
            let stringify = json_obj.get(JsString::from("stringify"), ctx)?;
            if let Some(stringify_fn) = stringify.as_callable() {
                let result = stringify_fn.call(&JsValue::undefined(), &[value.clone()], ctx)?;
                if let Ok(s) = result.to_string(ctx) {
                    let json_str = s.to_std_string_escaped();
                    if let Ok(parsed) = serde_json::from_str(&json_str) {
                        return Ok(parsed);
                    }
                }
            }
        }

        // Fallback: return null for unserializable objects
        return Ok(JsonValue::Null);
    }

    Ok(JsonValue::Null)
}

/// Convert a `serde_json::Value` to a Boa `JsValue`.
///
/// Injects the value into the given context. Objects and arrays are
/// created by evaluating a `JSON.parse()` call for simplicity.
pub fn json_to_js(value: &JsonValue, ctx: &mut Context) -> JsResult<JsValue> {
    match value {
        JsonValue::Null => Ok(JsValue::null()),
        JsonValue::Bool(b) => Ok(JsValue::from(*b)),
        JsonValue::Number(n) => {
            if let Some(f) = n.as_f64() {
                Ok(JsValue::from(f))
            } else {
                Ok(JsValue::from(0.0))
            }
        }
        JsonValue::String(s) => Ok(JsValue::from(boa_engine::JsString::from(s.as_str()))),
        JsonValue::Array(_) | JsonValue::Object(_) => {
            // Use JSON.parse for complex types
            let json_str = serde_json::to_string(value).map_err(|e| {
                JsNativeError::typ().with_message(format!("JSON serialization failed: {e}"))
            })?;
            let code = format!("JSON.parse({json_str:?})");
            ctx.eval(Source::from_bytes(code.as_bytes()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primitives_round_trip() {
        let mut ctx = Context::default();

        // null
        let js_null = JsValue::null();
        assert_eq!(js_to_json(&js_null, &mut ctx).unwrap(), JsonValue::Null);

        // boolean
        let js_true = JsValue::from(true);
        assert_eq!(
            js_to_json(&js_true, &mut ctx).unwrap(),
            JsonValue::Bool(true)
        );

        // number
        let js_num = JsValue::from(42.0);
        let json_num = js_to_json(&js_num, &mut ctx).unwrap();
        assert_eq!(json_num, serde_json::json!(42.0));

        // string
        let js_str = JsValue::from(boa_engine::JsString::from("hello"));
        assert_eq!(
            js_to_json(&js_str, &mut ctx).unwrap(),
            JsonValue::String("hello".to_string())
        );
    }

    #[test]
    fn test_json_to_js_primitives() {
        let mut ctx = Context::default();

        let js = json_to_js(&JsonValue::Null, &mut ctx).unwrap();
        assert!(js.is_null());

        let js = json_to_js(&JsonValue::Bool(true), &mut ctx).unwrap();
        assert_eq!(js.as_boolean(), Some(true));

        let js = json_to_js(&serde_json::json!(3.14), &mut ctx).unwrap();
        assert_eq!(js.as_number(), Some(3.14));
    }

    #[test]
    fn test_json_to_js_object() {
        let mut ctx = Context::default();
        let obj = serde_json::json!({"key": "value", "num": 42});
        let js = json_to_js(&obj, &mut ctx).unwrap();
        assert!(js.is_object());

        let json_back = js_to_json(&js, &mut ctx).unwrap();
        assert_eq!(json_back, obj);
    }

    #[test]
    fn test_json_to_js_array() {
        let mut ctx = Context::default();
        let arr = serde_json::json!([1, "two", true, null]);
        let js = json_to_js(&arr, &mut ctx).unwrap();
        assert!(js.as_object().unwrap().is_array());

        let json_back = js_to_json(&js, &mut ctx).unwrap();
        // Boa returns all numbers as f64, so 1 becomes 1.0
        assert_eq!(json_back, serde_json::json!([1.0, "two", true, null]));
    }
}
