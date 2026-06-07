use super::*;

#[test]
fn redacts_existing_and_new_credential_params() {
    assert_eq!(redact_str("u?token=abc&x=1"), "u?token=***&x=1");
    assert_eq!(redact_str("https://a:b@h/p"), "https://*:*@h/p");
    assert_eq!(
        redact_str("u?X-Amz-Security-Token=ZZ"),
        "u?X-Amz-Security-Token=***"
    );
    assert_eq!(redact_str("u?code=AUTHCODE"), "u?code=***");
    assert_eq!(redact_str("u?id_token=JWT.payload.sig"), "u?id_token=***");
    assert_eq!(redact_str("u?client_secret=SHH"), "u?client_secret=***");
    assert_eq!(redact_str("u?otp=123456"), "u?otp=***");
    assert_eq!(redact_str("u?otp_code=123456"), "u?otp_code=***");
}

#[test]
fn preserves_host_path_and_non_sensitive_params() {
    assert_eq!(
        redact_str("https://cdn.example.com/seg.m4s?range=0-99&quality=hd"),
        "https://cdn.example.com/seg.m4s?range=0-99&quality=hd"
    );
}

#[test]
fn redacts_bare_fragment_and_does_not_mangle_hyphenated_names() {
    // Bare fragment (no leading ?/&) is redacted — parity with old sanitize_for_logging.
    assert_eq!(redact_str("token=secret"), "token=***");
    assert_eq!(redact_str("password=hunter2"), "password=***");
    // Hyphenated AWS name must NOT be mangled by the generic signature= pattern.
    assert_eq!(
        redact_str("https://cdn/s.m4s?X-Amz-Signature=DEADBEEF"),
        "https://cdn/s.m4s?X-Amz-Signature=***"
    );
}

#[test]
fn redacts_x_amz_signature_within_full_presigned_url() {
    let got = redact_str(
        "https://cdn/s.m4s?X-Amz-Algorithm=AWS4&X-Amz-Signature=DEADBEEF&X-Amz-Date=20260607",
    );
    assert!(
        got.contains("https://cdn/s.m4s"),
        "host/path visible: {got}"
    );
    assert!(
        got.contains("X-Amz-Signature=***"),
        "signature redacted: {got}"
    );
    assert!(
        got.contains("X-Amz-Date=20260607"),
        "non-secret SigV4 param kept: {got}"
    );
}

#[test]
fn redacted_url_display_and_debug_redact_but_expose_is_raw() {
    let raw = "https://cdn/s.m4s?X-Amz-Signature=DEADBEEF";
    let r = RedactedUrl::new(raw);
    assert_eq!(format!("{r}"), "https://cdn/s.m4s?X-Amz-Signature=***");
    assert_eq!(format!("{r:?}"), "https://cdn/s.m4s?X-Amz-Signature=***");
    assert_eq!(r.expose(), raw, "expose() returns the unredacted value");
}

#[test]
fn redacted_url_buf_display_and_debug_redact_but_expose_is_raw() {
    let raw = "https://cdn/s.m4s?token=SECRET".to_string();
    let r = RedactedUrlBuf::from(raw.clone());
    assert_eq!(format!("{r}"), "https://cdn/s.m4s?token=***");
    assert_eq!(format!("{r:?}"), "https://cdn/s.m4s?token=***");
    assert_eq!(r.expose(), raw);
}

#[test]
fn redacted_url_new_accepts_str_and_string_refs_uniformly() {
    // Footgun guard: log sites pass owned String (&String) and borrowed
    // &String (e.g. a `for x in &Vec<String>` loop binding) — both must work
    // without the caller juggling & vs no-&.
    let owned: String = "u?key=K".to_string();
    let borrowed: &String = &owned;
    assert_eq!(format!("{}", RedactedUrl::new(&owned)), "u?key=***"); // &String
    assert_eq!(format!("{}", RedactedUrl::new(borrowed)), "u?key=***"); // &String (already a ref)
    assert_eq!(format!("{}", RedactedUrl::new(&borrowed)), "u?key=***"); // &&String
    assert_eq!(format!("{}", RedactedUrl::new("u?key=K")), "u?key=***"); // &str literal
}

#[test]
fn redacted_url_buf_from_str_and_new_redact() {
    // From<&str> is the path RdlpError's constructors use (url: &str).
    let a = RedactedUrlBuf::from("u?token=X");
    assert_eq!(format!("{a}"), "u?token=***");
    assert_eq!(a.expose(), "u?token=X");
    // new(impl Into<String>) accepts &str and String alike.
    let b = RedactedUrlBuf::new("u?sig=Y");
    assert_eq!(format!("{b}"), "u?sig=***");
    let c = RedactedUrlBuf::new("u?key=Z".to_string());
    assert_eq!(format!("{c:?}"), "u?key=***");
}

#[cfg(feature = "log-kv")]
#[test]
fn to_value_serializes_redacted_for_log_kv() {
    use log::kv::ToValue as _;
    let r = RedactedUrl::new("u?token=abc");
    assert_eq!(r.to_value().to_string(), "u?token=***");
}
