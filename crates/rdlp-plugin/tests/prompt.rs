// Integration tests aren't covered by clippy's `allow-unwrap-in-tests`
// (rust-clippy#13981) — re-allow at file scope. `disallowed_methods` permitted
// for `std::fs` test fixtures per clippy.toml policy (c). `missing_docs`
// exempt because integration tests aren't public API.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_methods,
    missing_docs,
)]

// Lints suppressed for test code — panicking on unexpected errors is intentional here.

use rdlp_plugin::prompt::{
    AlwaysApprove, AlwaysDeny, ConfirmRequest, ConfirmResponse, PreTrustedIdentities, Prompter,
};
use std::sync::Mutex;

struct Recording {
    last: Mutex<Option<ConfirmRequest>>,
    answer: ConfirmResponse,
}

impl Prompter for Recording {
    fn confirm(&self, request: ConfirmRequest) -> ConfirmResponse {
        *self.last.lock().unwrap() = Some(request);
        self.answer.clone()
    }
}

fn first_install(name: &str) -> ConfirmRequest {
    ConfirmRequest::FirstInstall {
        plugin_name: name.into(),
        version: "1.0.0".into(),
        identity: format!("sigstore:github:user/{name}"),
        capabilities: vec!["fetch".into(), "log".into()],
        claims_override: vec![],
    }
}

#[test]
fn always_approve_says_yes() {
    let p = AlwaysApprove;
    // AlwaysApprove now returns ApprovePersist (durable approval).
    assert!(matches!(
        p.confirm(first_install("foo")),
        ConfirmResponse::ApprovePersist
    ));
}

#[test]
fn always_deny_says_no() {
    let p = AlwaysDeny;
    assert!(matches!(
        p.confirm(first_install("foo")),
        ConfirmResponse::Deny
    ));
}

#[test]
fn recording_prompter_captures_request() {
    let p = Recording {
        last: Default::default(),
        answer: ConfirmResponse::ApprovePersist,
    };
    let req = first_install("foo");
    let _ = p.confirm(req);
    let captured = p.last.lock().unwrap();
    match captured.as_ref().unwrap() {
        ConfirmRequest::FirstInstall { plugin_name, .. } => {
            assert_eq!(plugin_name, "foo");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn pre_trusted_identities_approves_known() {
    let p = PreTrustedIdentities {
        trusted: vec!["sigstore:github:user/foo".into()],
    };
    // PreTrustedIdentities returns ApprovePersist for known identities.
    assert!(matches!(
        p.confirm(first_install("foo")),
        ConfirmResponse::ApprovePersist
    ));
}

#[test]
fn pre_trusted_identities_denies_unknown() {
    let p = PreTrustedIdentities {
        trusted: vec!["sigstore:github:other/x".into()],
    };
    assert!(matches!(
        p.confirm(first_install("foo")),
        ConfirmResponse::Deny
    ));
}

#[test]
fn pre_trusted_identities_denies_capability_creep_unconditionally() {
    // Capability creep should require explicit re-trust regardless of
    // pre-trusted identity list — defensive default.
    let p = PreTrustedIdentities {
        trusted: vec!["sigstore:github:user/foo".into()],
    };
    let req = ConfirmRequest::CapabilityCreep {
        plugin_name: "foo".into(),
        new_version: "1.1.0".into(),
        previously_approved: vec!["fetch".into()],
        new_capabilities: vec!["cookie-jar".into()],
    };
    assert!(matches!(p.confirm(req), ConfirmResponse::Deny));
}

#[test]
fn confirm_request_is_clone() {
    let r = first_install("x");
    let _r2 = r.clone(); // compile-time check
}
