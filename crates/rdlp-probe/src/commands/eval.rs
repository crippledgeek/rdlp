//! `rdlp-probe eval` — run JavaScript through the boa engine.
//!
//! Useful for inspecting obfuscated player JS, decoding encrypted format URLs,
//! and validating extraction expressions before wiring them into an extractor.

use anyhow::{Context, Result};
use clap::Parser;
use rdlp_core::JsEngine;
use rdlp_jsinterp::BoaJsEngine;
use std::path::PathBuf;
use tokio::io::AsyncReadExt;

#[derive(Parser, Debug)]
pub struct Args {
    /// Path to a .js file. Mutually exclusive with `--inline`.
    pub script: Option<PathBuf>,

    /// Inline JavaScript expression (alternative to passing a script file).
    #[arg(long = "inline", short = 'e')]
    pub inline: Option<String>,

    /// Optional context JSON file. Available in the script as the `context` global.
    /// Useful for probing decoders with captured page state.
    #[arg(long = "context", short = 'c')]
    pub context: Option<PathBuf>,

    /// Read script from stdin if neither a file nor `--inline` is given.
    #[arg(long = "stdin")]
    pub stdin: bool,
}

pub async fn run(args: Args) -> Result<()> {
    let code = match (args.script, args.inline, args.stdin) {
        (Some(path), None, false) => tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("read script {}", path.display()))?,
        (None, Some(inline), false) => inline,
        (None, None, true) => {
            let mut buf = String::new();
            tokio::io::stdin()
                .read_to_string(&mut buf)
                .await
                .context("read script from stdin")?;
            buf
        }
        (None, None, false) => {
            anyhow::bail!("provide a script path, --inline EXPR, or --stdin")
        }
        _ => anyhow::bail!("script path and --inline/--stdin are mutually exclusive"),
    };

    let engine = BoaJsEngine::new();

    let result = if let Some(ctx_path) = args.context {
        let raw = tokio::fs::read_to_string(&ctx_path)
            .await
            .with_context(|| format!("read context {}", ctx_path.display()))?;
        let value: serde_json::Value =
            serde_json::from_str(&raw).context("--context file is not valid JSON")?;
        engine
            .eval_with_context(&code, &value)
            .await
            .context("JavaScript evaluation failed")?
    } else {
        engine
            .eval(&code)
            .await
            .context("JavaScript evaluation failed")?
    };

    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
