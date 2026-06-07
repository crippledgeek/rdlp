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
