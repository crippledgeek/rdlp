// Lint-tightening for the binary entrypoint. `pedantic` / `nursery` are
// stylistic; `indexing_slicing` prevents silent out-of-bounds panics.
// See `Cargo.toml` `[lints.clippy]` for crate-level baseline.
#![warn(clippy::pedantic, clippy::nursery, clippy::indexing_slicing)]
//! `rdlp-probe` — extractor authoring toolkit.
//!
//! Every command runs through the same code paths the production extractors
//! use (`HttpClientFactory`, `BoaJsEngine`), so anything this tool can do, a
//! real extractor can do too. See the README for the suggested workflow when
//! adding a new site.
//!
//! Built only on demand — `rdlp-probe` is not in `default-members`. Build
//! with `cargo build --release -p rdlp-probe` and run via
//! `cargo run --release -p rdlp-probe -- <subcommand>`.

mod commands;

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "rdlp-probe",
    version,
    about = "Authoring toolkit for rdlp extractors",
    long_about = "Reuses rdlp's production HTTP stack (wreq + BoringSSL with browser \
                  emulation) and boa JavaScript engine, so new extractor authors can \
                  inspect a site exactly as the live extractors will see it."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Fetch a URL and print the body to stdout (status + headers to stderr).
    Fetch(commands::fetch::Args),
    /// Evaluate JavaScript with the boa engine and print the result as JSON.
    Eval(commands::eval::Args),
    /// Apply a regex, CSS selector, or JSON pointer to stdin/file.
    Extract(commands::extract::Args),
    /// Fetch a URL and save the (request, response) pair as a JSON cassette.
    Record(commands::record::Args),
    /// Report the HTTP version (ALPN) a host negotiates with the emulating client.
    Protocol(commands::protocol::Args),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Fetch(a) => commands::fetch::run(a).await,
        Command::Eval(a) => commands::eval::run(a).await,
        Command::Extract(a) => commands::extract::run(a).await,
        Command::Record(a) => commands::record::run(a).await,
        Command::Protocol(a) => commands::protocol::run(a).await,
    }
}
