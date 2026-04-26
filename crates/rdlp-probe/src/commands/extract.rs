//! `rdlp-probe extract` — apply a regex, CSS selector, or JSON pointer.
//!
//! Reads from stdin (or `--file`) so it can be piped from `rdlp-probe fetch`.

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use regex::Regex;
use scraper::{Html, Selector};
use std::path::PathBuf;
use tokio::io::AsyncReadExt;

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Mode {
    /// Treat the pattern as a Rust regex; print all captures (or full match if no group).
    Regex,
    /// Treat the pattern as a CSS selector; print each matched element's outer HTML.
    Css,
    /// Treat the pattern as a JSON pointer (RFC 6901); print the resolved value.
    Json,
    /// Treat the pattern as a key name; recursively walk JSON and print every matching value.
    JsonKey,
}

#[derive(Parser, Debug)]
pub struct Args {
    /// Extraction mode.
    #[arg(value_enum, long = "mode", short = 'm', default_value_t = Mode::Regex)]
    pub mode: Mode,

    /// Pattern (regex, selector, JSON pointer, or key — depends on `--mode`).
    pub pattern: String,

    /// Read input from this file instead of stdin.
    #[arg(long = "file", short = 'f')]
    pub file: Option<PathBuf>,

    /// Print only the first match (default: all).
    #[arg(long = "first")]
    pub first: bool,
}

pub async fn run(args: Args) -> Result<()> {
    let input = match args.file {
        Some(p) => tokio::fs::read_to_string(&p)
            .await
            .with_context(|| format!("read {}", p.display()))?,
        None => {
            let mut s = String::new();
            tokio::io::stdin()
                .read_to_string(&mut s)
                .await
                .context("read stdin")?;
            s
        }
    };

    match args.mode {
        Mode::Regex => extract_regex(&input, &args.pattern, args.first)?,
        Mode::Css => extract_css(&input, &args.pattern, args.first)?,
        Mode::Json => extract_json_pointer(&input, &args.pattern)?,
        Mode::JsonKey => extract_json_key(&input, &args.pattern, args.first)?,
    }

    Ok(())
}

fn extract_regex(input: &str, pattern: &str, first: bool) -> Result<()> {
    let re = Regex::new(pattern).context("invalid regex")?;
    let print = |caps: regex::Captures| {
        if let Some(g1) = caps.get(1) {
            println!("{}", g1.as_str());
        } else if let Some(m) = caps.get(0) {
            println!("{}", m.as_str());
        }
    };
    if first {
        if let Some(c) = re.captures(input) {
            print(c);
        }
    } else {
        for c in re.captures_iter(input) {
            print(c);
        }
    }
    Ok(())
}

fn extract_css(input: &str, selector: &str, first: bool) -> Result<()> {
    let doc = Html::parse_document(input);
    let sel = Selector::parse(selector)
        .map_err(|e| anyhow::anyhow!("invalid CSS selector: {e:?}"))?;
    let mut iter = doc.select(&sel);
    if first {
        if let Some(el) = iter.next() {
            println!("{}", el.html());
        }
    } else {
        for el in iter {
            println!("{}", el.html());
        }
    }
    Ok(())
}

fn extract_json_pointer(input: &str, pointer: &str) -> Result<()> {
    let value: serde_json::Value =
        serde_json::from_str(input).context("input is not valid JSON")?;
    match value.pointer(pointer) {
        Some(serde_json::Value::String(s)) => println!("{s}"),
        Some(v) => println!("{}", serde_json::to_string_pretty(v)?),
        None => anyhow::bail!("JSON pointer not found: {pointer}"),
    }
    Ok(())
}

fn extract_json_key(input: &str, key: &str, first: bool) -> Result<()> {
    let value: serde_json::Value =
        serde_json::from_str(input).context("input is not valid JSON")?;
    let mut hits: Vec<&serde_json::Value> = Vec::new();
    walk(&value, key, &mut hits);
    let render = |v: &serde_json::Value| -> Result<String> {
        Ok(match v {
            serde_json::Value::String(s) => s.clone(),
            other => serde_json::to_string_pretty(other)?,
        })
    };
    if first {
        if let Some(v) = hits.first() {
            println!("{}", render(v)?);
        }
    } else {
        for v in hits {
            println!("{}", render(v)?);
        }
    }
    Ok(())
}

fn walk<'a>(value: &'a serde_json::Value, key: &str, out: &mut Vec<&'a serde_json::Value>) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if k == key {
                    out.push(v);
                }
                walk(v, key, out);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                walk(v, key, out);
            }
        }
        _ => {}
    }
}
