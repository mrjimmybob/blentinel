# Contributing to Blentinel

Thanks for helping. This page covers **building from source, running the tests, and how changes are made**. What Blentinel is and how to use it are in the [README](README.md).

Blentinel is **pre-alpha**: breaking changes to the database, config, and protocol are allowed, and upgrades may require deleting the hub database (see [DECISIONS.md](docs/DECISIONS.md) D-30).

## Read first

| Document | Why |
|---|---|
| [docs/PRINCIPLES.md](docs/PRINCIPLES.md) | **Non-negotiable rules**: probes only push (the hub never connects to a probe), the probe is a single dependency-free binary, zero-trust, easy install and management. A change that breaks one is not accepted. |
| [docs/ENGINEERING.md](docs/ENGINEERING.md) | Definition of Done, code layering, DRY rules, testing strategy |
| [docs/ROADMAP.md](docs/ROADMAP.md) | What's planned, in priority order. Pick work from here. |
| [docs/DECISIONS.md](docs/DECISIONS.md) | Why things are the way they are. Challenge a decision by adding a new entry, never by editing an old one. |
| [rust.instructions.md](rust.instructions.md) | Rust style rules |

## Set up

1. **Rust** (stable, edition 2024): <https://rustup.rs>
   ```sh
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```
2. **WebAssembly target** (for the hub's web UI):
   ```sh
   rustup target add wasm32-unknown-unknown
   ```
3. **cargo-leptos** (builds the hub's server and web UI together):
   ```sh
   cargo install cargo-leptos --locked
   ```
   On Windows this compiles OpenSSL, which needs [Strawberry Perl](https://strawberryperl.com) first on `PATH`. The `perl` that comes with Git for Windows is missing modules, and the install fails with `Can't locate Locale/Maketext/Simple.pm`. Alternatively, build in Docker or WSL.
4. **Clone:**
   ```sh
   git clone https://github.com/mrjimmybob/blentinel.git
   cd blentinel
   ```

## Build

The workspace has four crates: `probe` (the agent), `hub` (server + Leptos UI), `common` (shared models, protocol, crypto), and `blentinelmake` (build tool).

**With the build tool** (recommended; the same commands packaging uses):

```sh
cargo build -p blentinelmake --release
./target/release/blentinelmake              # interactive menu
./target/release/blentinelmake --help       # e.g. `hub build --release`, `probe publish --target <triple>`
```

**By hand:**

```sh
cd hub && cargo leptos build --release --frontend-only                  # web UI: target/site/
BLENTINEL_EMBED_SITE=$PWD/../target/site cargo leptos build --release --server-only
                                                                        # hub: target/release/hub, web UI inside
cargo build -p probe --release              # probe: target/release/probe
```

The hub binary carries its web UI (D-51): `hub/build.rs` embeds the files from the folder `BLENTINEL_EMBED_SITE` names. Two steps, because cargo-leptos builds the front end and the server in parallel otherwise; `blentinelmake hub build` does both. Without the variable nothing is embedded and the hub serves `target/site/` from disk, which is what `cargo leptos watch` (development, live reload) and `cargo test-all` use; the log says which (`Web UI: embedded in the binary` / `served from disk`).

### Probe for other platforms

The probe must be a single static binary (PRINCIPLES FP-2), so Linux builds use the **musl** targets:

| Platform | Target |
|---|---|
| Linux x86_64 (servers, VPS) | `x86_64-unknown-linux-musl` |
| Linux ARM64 (Raspberry Pi 4/5) | `aarch64-unknown-linux-musl` |
| Windows x86_64 | `x86_64-pc-windows-msvc` |

```sh
rustup target add x86_64-unknown-linux-musl
cargo build -p probe --release --target x86_64-unknown-linux-musl
```

Cross-compiling needs a linker for the target (e.g. [`cargo-zigbuild`](https://github.com/rust-cross/cargo-zigbuild) or [`cross`](https://github.com/cross-rs/cross)). The hub can be cross-compiled too; only its web UI is WebAssembly, which doesn't depend on the target.

### Publishing

`blentinelmake <hub|probe> publish` builds in release mode and packages the binary and its `SHA256SUM` into `publish/…/app/` (plus a ZIP); the binaries install themselves (`hub install`, `probe install`, D-52, D-53). `hub publish --with-probes <triples>` adds a signed probe release.

**Release key** (D-55): `blentinelmake release keygen` creates it once, outside the source tree ([reference/cli.md](docs/reference/cli.md#blentinelmake-development)). Every hub and probe `blentinelmake` builds afterwards trusts it (it passes the public keys as `BLENTINEL_RELEASE_KEYS`); a plain `cargo build` without that variable makes binaries that accept no release. `blentinelmake release build --targets <triples>` signs a probe release for the hub's admin page; bump `[workspace.package] version` first, since probes install only newer versions.

### Manual

The PDF manual is **generated** from the Markdown in `docs/`. Never edit LaTeX by hand (D-09).

```sh
./target/release/blentinelmake docs          # → target/manual/blentinel-manual.pdf
./target/release/blentinelmake docs --tex    # → target/manual/blentinel-manual.tex
```

It needs [pandoc](https://pandoc.org) and, for PDF, a LaTeX engine with `xelatex` (MiKTeX or TeX Live). The chapter order is `docs/manual/chapters.txt`; the title page is `docs/manual/manual.yaml`; the styling is `docs/manual/header.tex`. When you add a user-facing page, add it to `chapters.txt`. Keep diagrams in plain ASCII (`+ - | > v`) so they render in any font.

## Test

```sh
cargo test-all                      # everything, including the hub's server-side tests
cargo test-all -- --ignored         # only the known_issue_* tests (expected to fail until fixed)
cargo test -p hub --features ssr --test alerts   # one test file
```

Use `cargo test-all`, not `cargo test --workspace`: the hub has no default features, so plain `cargo test --workspace` silently skips its server code. How the test harness works (in-process hub, controllable clock, test probe, recording notifier) is in [ENGINEERING.md §4](docs/ENGINEERING.md#4-testing-strategy).

### Browser tests (web UI)

`hub/end2end/` drives the real web UI in a browser with [Playwright](https://playwright.dev). It starts its own hub and a real probe in a temporary folder, so it touches nothing else: it creates a company and site, an enrollment code, and runs `probe enroll`. The hub offers that probe for download as a release signed with a test key (`hub/end2end/release-key.json`; `npm run build` compiles its public half in), so the Add probe test checks its one-line command and the admin test approves it. The probe and the browser both use HTTPS on port 4000 (the probe pins the self-signed certificate from its code; the browser accepts it). Run it after any change under `hub/src/app/`. It needs [Node.js](https://nodejs.org) 20+ and Google Chrome.

```sh
cd hub/end2end
npm ci              # once
npm run build       # the probe, and the hub with its web UI embedded (hub in target/e2e)
npm test            # ~45 s; `npm run report` opens the HTML report
```

Without Chrome, run `npx playwright install chromium` and set `E2E_BROWSER_CHANNEL=chromium`. `E2E_PORT` changes the ports (HTTP; HTTPS is the next one), and `E2E_KEEP=1` keeps the temporary folder (hub and probe logs) for debugging. A test fails on any browser console error or WASM panic.

Before every commit (CI isn't enabled yet, D-24):

```sh
cargo fmt --check
cargo lint          # clippy, warnings as errors, including the hub's server code
cargo lint-wasm     # clippy for the hub's browser (WebAssembly) code
cargo test-all
```

The aliases are defined in `.cargo/config.toml`. On Windows, also run `cargo lint` on Linux (e.g. in Docker) before touching `cfg(unix)` code, and the other way round: each OS only lints its own platform code.

## Making a change

1. Pick an item from [the roadmap](docs/ROADMAP.md), or open an issue first for anything not on it.
2. **Bug fix?** Start with a failing test. Known bugs are often already pinned by an ignored `known_issue_*` test; your fix removes its `#[ignore]`.
3. **Design change?** Write or update a proposal in `docs/proposals/` and add a `DECISIONS.md` entry *before* coding.
4. Keep the layering (ENGINEERING §2.2): HTTP handlers stay thin, rules go in `hub/src/service/`, SQL only in `hub/src/storage/sqlite/`, shared UI/server types in `hub/src/model.rs`, probe<->hub protocol in `common`.
5. **Docs in the same change:** README, `docs/`, and a `CHANGELOG.md` entry under *Unreleased*. A change isn't done until its docs are (PRINCIPLES PP-4).
6. Update the item's **Status** in `docs/ROADMAP.md`.
7. Open a pull request. The template walks you through the principle check and the Definition of Done.

### Database changes

Add a new file `hub/migrations/sqlite/NNNN_description.sql` with the next number. **Never edit a migration that has been committed.** Migrations run automatically when the hub starts.

### Never commit

Identity keys (`*.key`), setup codes (`setup.code`), tokens (`*.token`), TLS private keys, databases (`*.db`), logs, or a local `blentinel_hub.toml`. `.gitignore` covers these; don't force-add them.

## Licence

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
