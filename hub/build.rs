//! Rebuilds the hub when a migration is added, and embeds the web UI's
//! files (WASM, JS, CSS, favicon) in the hub binary
//! (M2-07, D-11, D-51), so a deployed hub is one file.
//!
//! Only when `BLENTINEL_EMBED_SITE` names the folder the front end was built
//! into (cargo-leptos' `site-root`). `blentinelmake hub build` sets it in the
//! second of two steps: `cargo leptos build --frontend-only`, then
//! `--server-only` with the variable. Two steps because cargo-leptos builds
//! the front end and the server in parallel by default, and the server must
//! embed finished files. Without the variable (`cargo leptos watch`, `cargo
//! test`) nothing is embedded and the hub serves the folder from disk.
//!
//! Generates `$OUT_DIR/web_assets.rs`: `FILES: &[WebAsset]`, one per file,
//! with an ETag (FNV-1a of the content) for browser caching.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const ENV: &str = "BLENTINEL_EMBED_SITE";

fn main() {
    // sqlx::migrate! embeds hub/migrations at compile time. Without this, adding
    // a new migration file would not trigger a rebuild, and the hub would
    // silently ship without it.
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-env-changed={ENV}");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let mut code = String::from("pub static FILES: &[WebAsset] = &[\n");

    let site = std::env::var(ENV).ok().filter(|v| !v.is_empty());
    if let (Some(site), true) = (site, std::env::var("CARGO_FEATURE_SSR").is_ok()) {
        let root = PathBuf::from(&site);
        let files = files_under(&root);
        assert!(
            files
                .iter()
                .any(|f| f.extension().is_some_and(|e| e == "wasm")),
            "{ENV}={site} holds no .wasm file: build the front end first (cargo leptos build --frontend-only)"
        );
        println!("cargo:rerun-if-changed={site}");
        for file in files {
            println!("cargo:rerun-if-changed={}", file.display());
            let bytes = std::fs::read(&file).expect("read a web asset");
            let path = file
                .strip_prefix(&root)
                .expect("under the site root")
                .to_string_lossy()
                .replace('\\', "/");
            writeln!(
                code,
                "    WebAsset {{ path: {path:?}, etag: \"\\\"{:016x}\\\"\", bytes: include_bytes!({:?}) }},",
                fnv1a(&bytes),
                file.canonicalize().expect("canonical path")
            )
            .expect("write to a string");
        }
    }
    code.push_str("];\n");
    std::fs::write(out.join("web_assets.rs"), code).expect("write web_assets.rs");
}

/// Every file under `dir`, sorted, recursively.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let entries = std::fs::read_dir(&next)
            .unwrap_or_else(|e| panic!("{ENV}: can't read {}: {e}", next.display()));
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// 64-bit FNV-1a: a content fingerprint for the ETag (not security).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}
