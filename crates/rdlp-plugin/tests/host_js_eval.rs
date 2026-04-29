use rdlp_plugin::engine::{Engine, EngineConfig};
use rdlp_plugin::host::js_eval::JsEvalCtx;
use rdlp_plugin::instance::PluginStoreData;

#[test]
fn add_to_linker_succeeds() {
    let engine = Engine::new(EngineConfig::default()).expect("engine");
    let mut linker = wasmtime::component::Linker::<PluginStoreData>::new(engine.raw());
    rdlp_plugin::host::js_eval::add_to_linker(&mut linker).expect("add to linker");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn eval_simple_arithmetic() {
    let ctx = JsEvalCtx::default();
    let result = ctx.eval(&[], "1 + 2").await;
    assert!(result.is_ok(), "expected Ok, got: {result:?}");
    // boa returns numbers as floats; JSON serialises 3 as "3.0"
    let s = result.unwrap();
    assert!(
        s.contains('3'),
        "expected '3' in result, got: {s}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn eval_string_concat() {
    let ctx = JsEvalCtx::default();
    let result = ctx.eval(&[], "'hello ' + 'world'").await;
    assert!(result.is_ok(), "expected Ok, got: {result:?}");
    let s = result.unwrap();
    assert!(s.contains("hello world"), "expected 'hello world' in: {s}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn eval_sandbox_globals_injected() {
    let ctx = JsEvalCtx::default();
    let globals = vec![
        ("greeting".to_string(), "hello".to_string()),
        ("name".to_string(), "plugin".to_string()),
    ];
    let result = ctx.eval(&globals, "greeting + ' ' + name").await;
    assert!(result.is_ok(), "expected Ok, got: {result:?}");
    let s = result.unwrap();
    assert!(s.contains("hello plugin"), "expected 'hello plugin' in: {s}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn eval_syntax_error_returns_err() {
    let ctx = JsEvalCtx::default();
    let result = ctx.eval(&[], "this is not valid JS @@@").await;
    assert!(result.is_err(), "syntax error should produce Err, got: {result:?}");
}

#[test]
fn ctx_default_has_sensible_caps() {
    let ctx = JsEvalCtx::default();
    assert!(
        ctx.timeout >= std::time::Duration::from_millis(100),
        "timeout should be at least 100ms, got: {:?}",
        ctx.timeout
    );
    assert!(
        ctx.memory_cap >= 1024 * 1024,
        "memory_cap should be at least 1 MB, got: {}",
        ctx.memory_cap
    );
}
