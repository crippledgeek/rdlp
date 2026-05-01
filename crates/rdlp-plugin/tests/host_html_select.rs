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

use rdlp_plugin::engine::{Engine, EngineConfig};
use rdlp_plugin::host::html_select::HtmlSelectCtx;
use rdlp_plugin::instance::PluginStoreData;
use tokio_util::sync::CancellationToken;

const SAMPLE_HTML: &str = r#"
<html>
  <body>
    <div class="title">First</div>
    <div class="title">Second</div>
    <a href="/foo" class="link">Link 1</a>
    <a href="/bar" class="link">Link 2</a>
    <span data-id="42">Tagged</span>
  </body>
</html>
"#;

#[test]
fn add_to_linker_succeeds() {
    let engine = Engine::new(EngineConfig::default()).expect("engine");
    let mut linker = wasmtime::component::Linker::<PluginStoreData>::new(engine.raw());
    rdlp_plugin::host::html_select::add_to_linker(&mut linker).expect("add to linker");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn select_returns_text_of_matching_elements() {
    let cancel = CancellationToken::new();
    let mut data = PluginStoreData::new("test", cancel);
    data.html_select = Some(HtmlSelectCtx);

    let titles = HtmlSelectCtx::select_text(SAMPLE_HTML, ".title");
    assert_eq!(titles, vec!["First", "Second"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn select_attr_returns_attribute_values() {
    let hrefs = HtmlSelectCtx::select_attr_values(SAMPLE_HTML, ".link", "href");
    assert_eq!(hrefs, vec!["/foo", "/bar"]);
}

#[test]
fn invalid_selector_returns_empty_list_not_error() {
    let result = HtmlSelectCtx::select_text(SAMPLE_HTML, ":::invalid:::");
    assert!(
        result.is_empty(),
        "invalid selector should be empty, not panic"
    );
}

#[test]
fn missing_attribute_returns_empty_list() {
    let result = HtmlSelectCtx::select_attr_values(SAMPLE_HTML, ".title", "nonexistent");
    assert!(result.is_empty());
}

#[test]
fn empty_html_returns_empty_list() {
    let result = HtmlSelectCtx::select_text("", ".any");
    assert!(result.is_empty());
}
