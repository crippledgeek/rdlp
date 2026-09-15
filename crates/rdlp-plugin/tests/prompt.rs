// Integration tests aren't covered by clippy's `allow-unwrap-in-tests`
// (rust-clippy#13981) — re-allow at file scope. `disallowed_methods` permitted
// for `std::fs` test fixtures per clippy.toml policy (c). `missing_docs`
// exempt because integration tests aren't public API.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_methods,
    missing_docs
)]

// Lints suppressed for test code — panicking on unexpected errors is intentional here.

use rdlp_plugin::manifest::SearchClaims;
use rdlp_plugin::prompt::{
    AlwaysApprove, AlwaysDeny, ConfirmRequest, ConfirmResponse, PreTrustedIdentities, Prompter,
};
use rdlp_plugin::test_support::RecordingPrompter;

fn first_install(name: &str) -> ConfirmRequest {
    ConfirmRequest::FirstInstall {
        plugin_name: name.into(),
        version: "1.0.0".into(),
        identity: format!("sigstore:github:user/{name}"),
        capabilities: vec!["fetch".into(), "log".into()],
        claims_override: vec![],
        search: SearchClaims::default(),
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
    let p = RecordingPrompter::answering(ConfirmResponse::ApprovePersist);
    let req = first_install("foo");
    let _ = p.confirm(req);
    match p.requests().as_slice() {
        [ConfirmRequest::FirstInstall { plugin_name, .. }] => {
            assert_eq!(plugin_name, "foo");
        }
        other => panic!("wrong variant: {other:?}"),
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
#[allow(
    clippy::redundant_clone,
    reason = "Intentional: exercises Clone impl as a compile-time check"
)]
fn confirm_request_is_clone() {
    let r = first_install("x");
    let _r2 = r.clone();
}

#[test]
fn pre_trusted_identities_denies_a_search_claim_change_unconditionally() {
    // A plugin that starts claiming a built-in's search after it was
    // trusted needs explicit re-trust, exactly like capability creep.
    let p = PreTrustedIdentities {
        trusted: vec!["sigstore:github:user/foo".into()],
    };
    let req = ConfirmRequest::SearchClaimsChange {
        plugin_name: "foo".into(),
        new_version: "1.1.0".into(),
        previously_approved: SearchClaims::default(),
        requested: SearchClaims {
            search_site: Some("pornhub".into()),
            search_claims_override: vec!["pornhub".into()],
        },
    };
    assert!(matches!(p.confirm(req), ConfirmResponse::Deny));
}
