//! Browser API polyfills for Boa execution contexts.
//!
//! Injects minimal browser-compatible globals so that player scripts
//! referencing `window`, `atob`, `btoa`, `navigator`, and `document`
//! don't throw `ReferenceError`.

use boa_engine::{Context, JsResult, Source};

/// JavaScript polyfill code injected into every fresh context.
///
/// Provides:
/// - `window` as alias for `globalThis`
/// - `self` as alias for `globalThis`
/// - `atob(str)` — base64 decode
/// - `btoa(str)` — base64 encode
/// - `navigator` — minimal stub with `userAgent`
/// - `document` — empty stub to prevent reference errors
/// - `location` — stub with empty `href`
const POLYFILLS: &str = r#"
var window = globalThis;
var self = globalThis;

var navigator = {
    userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) rdlp/0.1",
    language: "en-US",
    languages: ["en-US", "en"],
    platform: "Win32"
};

var document = {
    cookie: "",
    createElement: function() { return { style: {} }; },
    getElementsByTagName: function() { return []; },
    querySelector: function() { return null; },
    querySelectorAll: function() { return []; },
    getElementById: function() { return null; },
    head: { appendChild: function() {} },
    body: { appendChild: function() {} }
};

var location = {
    href: "",
    hostname: "",
    pathname: "",
    search: "",
    hash: "",
    protocol: "https:",
    origin: ""
};

// Base64 encode/decode using built-in Boa capabilities
function btoa(str) {
    var chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";
    var result = "";
    var i = 0;
    while (i < str.length) {
        var a = str.charCodeAt(i++);
        var bRaw = i < str.length ? str.charCodeAt(i++) : NaN;
        var cRaw = i < str.length ? str.charCodeAt(i++) : NaN;
        var b = isNaN(bRaw) ? 0 : bRaw;
        var c = isNaN(cRaw) ? 0 : cRaw;
        var enc1 = a >> 2;
        var enc2 = ((a & 3) << 4) | (b >> 4);
        var enc3 = ((b & 15) << 2) | (c >> 6);
        var enc4 = c & 63;
        if (isNaN(bRaw)) { enc3 = enc4 = 64; }
        else if (isNaN(cRaw)) { enc4 = 64; }
        result += chars.charAt(enc1) + chars.charAt(enc2) + chars.charAt(enc3) + chars.charAt(enc4);
    }
    return result;
}

function atob(str) {
    var chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";
    var result = "";
    str = str.replace(/[^A-Za-z0-9+/=]/g, "");
    var i = 0;
    while (i < str.length) {
        var enc1 = chars.indexOf(str.charAt(i++));
        var enc2 = chars.indexOf(str.charAt(i++));
        var enc3 = chars.indexOf(str.charAt(i++));
        var enc4 = chars.indexOf(str.charAt(i++));
        var a = (enc1 << 2) | (enc2 >> 4);
        var b = ((enc2 & 15) << 4) | (enc3 >> 2);
        var c = ((enc3 & 3) << 6) | enc4;
        result += String.fromCharCode(a);
        if (enc3 !== 64) { result += String.fromCharCode(b); }
        if (enc4 !== 64) { result += String.fromCharCode(c); }
    }
    return result;
}
"#;

/// Inject browser polyfills into a Boa context.
///
/// This should be called on every fresh context before evaluating
/// user-provided JavaScript.
pub fn inject_polyfills(ctx: &mut Context) -> JsResult<()> {
    ctx.eval(Source::from_bytes(POLYFILLS.as_bytes()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_polyfills_inject_without_error() {
        let mut ctx = Context::default();
        inject_polyfills(&mut ctx).unwrap();
    }

    #[test]
    fn test_window_is_global() {
        let mut ctx = Context::default();
        inject_polyfills(&mut ctx).unwrap();
        let result = ctx
            .eval(Source::from_bytes(b"window === globalThis"))
            .unwrap();
        assert_eq!(result.as_boolean(), Some(true));
    }

    #[test]
    fn test_atob_btoa_round_trip() {
        let mut ctx = Context::default();
        inject_polyfills(&mut ctx).unwrap();
        let result = ctx
            .eval(Source::from_bytes(b"atob(btoa('hello world'))"))
            .unwrap();
        let s = result.to_string(&mut ctx).unwrap();
        assert_eq!(s.to_std_string_escaped(), "hello world");
    }

    #[test]
    fn test_btoa_known_value() {
        let mut ctx = Context::default();
        inject_polyfills(&mut ctx).unwrap();
        let result = ctx.eval(Source::from_bytes(b"btoa('Hello')")).unwrap();
        let s = result.to_string(&mut ctx).unwrap();
        assert_eq!(s.to_std_string_escaped(), "SGVsbG8=");
    }

    #[test]
    fn test_atob_known_value() {
        let mut ctx = Context::default();
        inject_polyfills(&mut ctx).unwrap();
        let result = ctx.eval(Source::from_bytes(b"atob('SGVsbG8=')")).unwrap();
        let s = result.to_string(&mut ctx).unwrap();
        assert_eq!(s.to_std_string_escaped(), "Hello");
    }

    #[test]
    fn test_navigator_exists() {
        let mut ctx = Context::default();
        inject_polyfills(&mut ctx).unwrap();
        let result = ctx
            .eval(Source::from_bytes(b"typeof navigator.userAgent"))
            .unwrap();
        let s = result.to_string(&mut ctx).unwrap();
        assert_eq!(s.to_std_string_escaped(), "string");
    }

    #[test]
    fn test_document_stubs() {
        let mut ctx = Context::default();
        inject_polyfills(&mut ctx).unwrap();
        let result = ctx
            .eval(Source::from_bytes(b"document.getElementById('test')"))
            .unwrap();
        assert!(result.is_null());
    }
}
