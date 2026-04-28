use rdlp_plugin::trust_store::{CapabilityCheck, IdentityCheck, TrustEntry, TrustStore};
use std::collections::BTreeSet;
use tempfile::TempDir;

fn caps(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn first_install_records_identity() {
    let dir = TempDir::new().unwrap();
    let mut ts = TrustStore::open(dir.path().join("trust.toml")).unwrap();
    assert!(ts.lookup("youtube").is_none());

    ts.record(TrustEntry {
        name: "youtube".into(),
        identity: "sigstore:github:johndoe/yt".into(),
        approved_capabilities: caps(&["fetch", "log"]),
    })
    .unwrap();

    let entry = ts.lookup("youtube").expect("recorded");
    assert_eq!(entry.identity, "sigstore:github:johndoe/yt");
    assert_eq!(entry.approved_capabilities, caps(&["fetch", "log"]));
}

#[test]
fn round_trip_through_disk() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("trust.toml");
    {
        let mut ts = TrustStore::open(&path).unwrap();
        ts.record(TrustEntry {
            name: "x".into(),
            identity: "ed25519:abcd1234".into(),
            approved_capabilities: caps(&["fetch"]),
        })
        .unwrap();
    }
    let ts2 = TrustStore::open(&path).unwrap();
    let e = ts2.lookup("x").expect("persisted");
    assert_eq!(e.identity, "ed25519:abcd1234");
    assert_eq!(e.approved_capabilities, caps(&["fetch"]));
}

#[test]
fn identity_check_detects_new_name() {
    let dir = TempDir::new().unwrap();
    let ts = TrustStore::open(dir.path().join("trust.toml")).unwrap();
    assert!(matches!(
        ts.check_identity_match("never-seen", "sigstore:github:any/any"),
        IdentityCheck::NewName
    ));
}

#[test]
fn identity_check_detects_match() {
    let dir = TempDir::new().unwrap();
    let mut ts = TrustStore::open(dir.path().join("trust.toml")).unwrap();
    ts.record(TrustEntry {
        name: "x".into(),
        identity: "sigstore:github:alice/x".into(),
        approved_capabilities: caps(&["log"]),
    })
    .unwrap();
    assert!(matches!(
        ts.check_identity_match("x", "sigstore:github:alice/x"),
        IdentityCheck::Match
    ));
}

#[test]
fn identity_check_detects_mismatch() {
    let dir = TempDir::new().unwrap();
    let mut ts = TrustStore::open(dir.path().join("trust.toml")).unwrap();
    ts.record(TrustEntry {
        name: "x".into(),
        identity: "sigstore:github:alice/x".into(),
        approved_capabilities: caps(&["log"]),
    })
    .unwrap();
    let result = ts.check_identity_match("x", "sigstore:github:bob/x");
    match result {
        IdentityCheck::Mismatch { recorded, presented } => {
            assert_eq!(recorded, "sigstore:github:alice/x");
            assert_eq!(presented, "sigstore:github:bob/x");
        }
        other => panic!("expected mismatch, got {other:?}"),
    }
}

#[test]
fn capability_check_all_approved_when_subset() {
    let dir = TempDir::new().unwrap();
    let mut ts = TrustStore::open(dir.path().join("trust.toml")).unwrap();
    ts.record(TrustEntry {
        name: "x".into(),
        identity: "ed25519:aabbccdd".into(),
        approved_capabilities: caps(&["fetch", "log", "cookie-jar"]),
    })
    .unwrap();
    let requested = caps(&["fetch", "log"]);
    assert!(matches!(
        ts.check_capabilities("x", &requested),
        CapabilityCheck::AllApproved
    ));
}

#[test]
fn capability_check_flags_new_capabilities() {
    let dir = TempDir::new().unwrap();
    let mut ts = TrustStore::open(dir.path().join("trust.toml")).unwrap();
    ts.record(TrustEntry {
        name: "x".into(),
        identity: "ed25519:aabbccdd".into(),
        approved_capabilities: caps(&["fetch"]),
    })
    .unwrap();
    let requested = caps(&["fetch", "log", "cookie-jar"]);
    match ts.check_capabilities("x", &requested) {
        CapabilityCheck::NewCapabilitiesRequested(new_caps) => {
            let new_set: BTreeSet<String> = new_caps.into_iter().collect();
            assert_eq!(new_set, caps(&["log", "cookie-jar"]));
        }
        other => panic!("expected new caps, got {other:?}"),
    }
}

#[test]
fn capability_check_for_new_name_returns_all_requested_as_new() {
    let dir = TempDir::new().unwrap();
    let ts = TrustStore::open(dir.path().join("trust.toml")).unwrap();
    let requested = caps(&["fetch", "log"]);
    match ts.check_capabilities("never-seen", &requested) {
        CapabilityCheck::NewCapabilitiesRequested(new_caps) => {
            let new_set: BTreeSet<String> = new_caps.into_iter().collect();
            assert_eq!(new_set, caps(&["fetch", "log"]));
        }
        other => panic!("expected new caps, got {other:?}"),
    }
}

#[test]
fn forget_removes_entry() {
    let dir = TempDir::new().unwrap();
    let mut ts = TrustStore::open(dir.path().join("trust.toml")).unwrap();
    ts.record(TrustEntry {
        name: "x".into(),
        identity: "ed25519:aabbccdd".into(),
        approved_capabilities: caps(&["log"]),
    })
    .unwrap();
    assert!(ts.lookup("x").is_some());
    ts.forget("x").unwrap();
    assert!(ts.lookup("x").is_none());
}
