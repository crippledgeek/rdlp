//! Sign an rdlp plugin manifest with an Ed25519 key.
//!
//! Usage: `example-sign-plugin <wasm_path> <manifest_template_path> <pem_key_path>`
//!
//! Reads the PEM-encoded Ed25519 private key, substitutes the real pubkey into
//! the template, computes `canonical_bytes(manifest) || wasm_bytes`, signs, and
//! prints the final `plugin.toml` to stdout.

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use ed25519_dalek::pkcs8::DecodePrivateKey;
use ed25519_dalek::Signer;
use rdlp_plugin::manifest::{canonical_bytes, parse_manifest_str};
use std::fs;
use std::path::Path;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        bail!(
            "usage: {} <wasm_path> <manifest_template_path> <pem_key_path>",
            args.first().map_or("example-sign-plugin", String::as_str)
        );
    }

    let wasm_path = Path::new(&args[1]);
    let manifest_path = Path::new(&args[2]);
    let key_path = Path::new(&args[3]);

    let wasm_bytes = fs::read(wasm_path)
        .with_context(|| format!("read wasm at {}", wasm_path.display()))?;
    let template = fs::read_to_string(manifest_path)
        .with_context(|| format!("read manifest template at {}", manifest_path.display()))?;
    let pem = fs::read_to_string(key_path)
        .with_context(|| format!("read key at {}", key_path.display()))?;

    let signing_key = SigningKey::from_pkcs8_pem(&pem)
        .with_context(|| format!("parse Ed25519 PEM key at {}", key_path.display()))?;
    let pubkey_bytes = signing_key.verifying_key().to_bytes();
    let pubkey_b64 = base64::engine::general_purpose::STANDARD.encode(pubkey_bytes);

    if !template.contains("PLACEHOLDER_PUBKEY") || !template.contains("PLACEHOLDER_SIGNATURE") {
        bail!("template must contain both PLACEHOLDER_PUBKEY and PLACEHOLDER_SIGNATURE");
    }

    let with_pubkey = template.replace("PLACEHOLDER_PUBKEY", &pubkey_b64);
    let manifest = parse_manifest_str(&with_pubkey)
        .context("parse manifest after pubkey substitution")?;

    let mut buf = canonical_bytes(&manifest);
    buf.extend_from_slice(&wasm_bytes);
    let sig = signing_key.sign(&buf);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());

    let final_toml = with_pubkey.replace("PLACEHOLDER_SIGNATURE", &sig_b64);

    // Round-trip sanity check: parse the final TOML to confirm validity.
    parse_manifest_str(&final_toml).context("round-trip parse of signed manifest")?;

    print!("{final_toml}");
    Ok(())
}
