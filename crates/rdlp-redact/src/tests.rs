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

#[cfg(feature = "log-kv")]
#[test]
fn buf_to_value_serializes_redacted_for_log_kv() {
    // Guards the owned wrapper's macro-generated `ToValue`: it MUST redact
    // (route through `redact_str` via Display), same as the borrowed wrapper.
    use log::kv::ToValue as _;
    let r = RedactedUrlBuf::new("u?token=abc");
    assert_eq!(r.to_value().to_string(), "u?token=***");
}

#[test]
fn acceptance_328_presigned_and_oauth_code_redacted_host_preserved() {
    let s = redact_str("https://cdn.example.com/seg.m4s?X-Amz-Signature=DEADBEEF");
    assert_eq!(s, "https://cdn.example.com/seg.m4s?X-Amz-Signature=***");
    let c = redact_str("https://idp/cb?code=AUTH123&state=xyz");
    assert_eq!(
        c, "https://idp/cb?code=***&state=xyz",
        "code redacted, state (anti-CSRF) kept"
    );
}

// ── Credential values and the parentheses `wreq::Error` wraps URLs in ──

#[test]
fn a_credential_parameter_inside_parentheses_keeps_its_closing_paren() {
    // `wreq::Error`'s Display renders `for uri (<url>)`, so a credential in
    // the LAST query parameter sits immediately before `)`.
    assert_eq!(
        redact_str("for uri (https://cdn.example.com/v.mp4?token=abc123)"),
        "for uri (https://cdn.example.com/v.mp4?token=***)"
    );
}

#[test]
fn a_credential_containing_a_paren_is_redacted_to_its_end() {
    // Why `)` is NOT excluded from the value class: RFC 3986 §2.2 makes it a
    // legal unencoded sub-delim, so excluding it stopped the match early and
    // left the tail of the secret in the clear.
    assert_eq!(
        redact_str("https://h/p?token=abc)REST_OF_SECRET"),
        "https://h/p?token=***"
    );
    assert_eq!(
        redact_str("for uri (https://h/p?token=abc)REST)"),
        "for uri (https://h/p?token=***)"
    );
}

#[test]
fn a_repeated_credential_parameter_is_redacted_every_time() {
    // Duplicate query keys are legal, and arise from redirect chains and from
    // retry wrappers that append their own auth parameter without checking.
    //
    // An earlier fix consumed the delimiter after the value so it could
    // re-emit it; that moved the scan cursor past the `&` the NEXT occurrence
    // needs for its own boundary anchor, and only the first was redacted. The
    // delimiter is no longer part of the match.
    assert_eq!(
        redact_str("https://h/p?token=aaa&token=bbb"),
        "https://h/p?token=***&token=***"
    );
    assert_eq!(
        redact_str("?token=a&token=b&token=c"),
        "?token=***&token=***&token=***"
    );
}

#[test]
fn a_credential_named_in_free_text_after_a_space_is_redacted() {
    // Whitespace is in the boundary set, so a credential mentioned in prose —
    // which is how these strings are assembled — is caught, not just one in a
    // query string.
    assert_eq!(redact_str("token=aaa token=bbb"), "token=*** token=***");
    assert_eq!(
        redact_str("request failed with api_key=SECRET"),
        "request failed with api_key=***"
    );
}

#[test]
fn a_hyphenated_name_is_not_matched_by_the_generic_pattern() {
    // The boundary set must never admit `-`: the generic `signature=` pattern
    // would then match inside `X-Amz-Signature` and re-emit it lower-cased.
    // This is the property any widening of that set has to preserve.
    assert_eq!(
        redact_str("https://cdn/s?X-Amz-Signature=DEADBEEF"),
        "https://cdn/s?X-Amz-Signature=***"
    );
}

#[test]
fn userinfo_and_a_query_credential_are_both_stripped_in_one_pass() {
    assert_eq!(
        redact_str("for uri (https://u:p@cdn.example.com/v?token=abc123)"),
        "for uri (https://*:*@cdn.example.com/v?token=***)"
    );
}

#[test]
fn an_adjacent_credential_parameter_of_a_different_name_still_matches() {
    assert_eq!(
        redact_str("https://h/p?token=aaa&api_key=bbb"),
        "https://h/p?token=***&api_key=***"
    );
}

#[test]
fn a_message_with_no_credential_is_left_alone() {
    // The control. Over-redaction would cost the diagnostic the message
    // exists to carry, so the host and path must survive.
    assert_eq!(
        redact_str("for uri (https://cdn.example.com/v.mp4)"),
        "for uri (https://cdn.example.com/v.mp4)"
    );
}

/// `redact_str` is idempotent: redacting twice equals redacting once.
///
/// Load-bearing, not academic. `RdlpApiError` variants redact at construction
/// AND again at render (`#[error("…", redact(message))]`, `user_message()`,
/// the custom `Debug`), and `AppError` fields redact a third time at serde
/// serialization — the project stacks independent layers rather than trusting
/// one renderer. That is only safe if a second pass is a no-op.
///
/// It holds because every replacement is a FIXED TEMPLATE that its own pattern
/// re-matches and re-emits unchanged: `***` satisfies the value class, `*:*`
/// satisfies the userinfo class, and the closure never derives output from the
/// previous match's value.
#[test]
fn redacting_twice_is_the_same_as_redacting_once() {
    for input in [
        "https://user:pw@example.com/v?token=secret",
        "boom https://a:b@h/p?api_key=k&access_token=t",
        "token=secret)",
        "?token=secret",
        "for uri (https://cdn.example.com/v.mp4)",
        "Download task panicked: task 3 panicked with message \"boom\"",
        "",
    ] {
        let once = redact_str(input);
        let twice = redact_str(&once);
        assert_eq!(once, twice, "not a fixed point for input: {input}");
    }
}
