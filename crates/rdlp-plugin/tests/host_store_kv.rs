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

use rdlp_plugin::host::store_kv::StoreKvCtx;
use tempfile::TempDir;

fn open_db(dir: &TempDir) -> sled::Db {
    sled::open(dir.path().join("kv")).expect("sled open")
}

#[test]
fn add_to_linker_succeeds() {
    let engine = rdlp_plugin::engine::Engine::new(Default::default()).expect("engine");
    let mut linker =
        wasmtime::component::Linker::<rdlp_plugin::instance::PluginStoreData>::new(engine.raw());
    rdlp_plugin::host::store_kv::add_to_linker(&mut linker).expect("link");
}

#[test]
fn set_and_get_round_trip() {
    let dir = TempDir::new().unwrap();
    let db = open_db(&dir);
    let ctx = StoreKvCtx::open(&db, "youtube").unwrap();
    ctx.set_blocking(b"key1", b"value1").unwrap();
    let v = ctx.get_blocking(b"key1");
    assert_eq!(v.as_deref(), Some(&b"value1"[..]));
}

#[test]
fn missing_key_returns_none() {
    let dir = TempDir::new().unwrap();
    let db = open_db(&dir);
    let ctx = StoreKvCtx::open(&db, "x").unwrap();
    assert!(ctx.get_blocking(b"missing").is_none());
}

#[test]
fn delete_removes_value() {
    let dir = TempDir::new().unwrap();
    let db = open_db(&dir);
    let ctx = StoreKvCtx::open(&db, "x").unwrap();
    ctx.set_blocking(b"k", b"v").unwrap();
    ctx.delete_blocking(b"k");
    assert!(ctx.get_blocking(b"k").is_none());
}

#[test]
fn plugin_namespaces_are_isolated() {
    let dir = TempDir::new().unwrap();
    let db = open_db(&dir);
    let a = StoreKvCtx::open(&db, "alpha").unwrap();
    let b = StoreKvCtx::open(&db, "beta").unwrap();
    a.set_blocking(b"shared-key", b"alpha-value").unwrap();
    b.set_blocking(b"shared-key", b"beta-value").unwrap();
    assert_eq!(
        a.get_blocking(b"shared-key").as_deref(),
        Some(&b"alpha-value"[..])
    );
    assert_eq!(
        b.get_blocking(b"shared-key").as_deref(),
        Some(&b"beta-value"[..])
    );
}

#[test]
fn quota_enforced() {
    let dir = TempDir::new().unwrap();
    let db = open_db(&dir);
    let mut ctx = StoreKvCtx::open(&db, "small").unwrap();
    ctx.quota_bytes = 100; // override default 10 MB for the test
    let small = vec![0u8; 60];
    ctx.set_blocking(b"a", &small).expect("under quota");
    let big = vec![0u8; 60];
    let err = ctx.set_blocking(b"b", &big).unwrap_err();
    assert!(err.contains("quota"));
}
