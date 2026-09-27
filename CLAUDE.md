# Blentinel — Instructions for AI assistants

Read before making changes:

1. **[docs/PRINCIPLES.md](docs/PRINCIPLES.md) is non-negotiable.**
   - The hub never connects to a probe (push-only).
   - The probe stays a single static binary with no runtime, library, or external tool dependencies.
   - Zero-trust: pinned hub identity, signed and encrypted reports, and the hub assigns the tenant.
   - Install and manage from the UI, not by editing files.
   If a request conflicts with these, say so and propose an alternative. Do not implement it.
2. **[docs/ENGINEERING.md](docs/ENGINEERING.md)** defines the Definition of Done, layering, DRY rules, and the testing strategy. Rust style: [rust.instructions.md](rust.instructions.md).
3. **[docs/ROADMAP.md](docs/ROADMAP.md)** is the backlog. Work on the item the user asks for, and update its **Status** in the same change.
4. **Docs are part of every change.** Update the README, `docs/install/*`, reference docs, and `CHANGELOG.md` whenever behaviour, config, CLI, or API changes.
5. Record design decisions and challenges to earlier specs in **[docs/DECISIONS.md](docs/DECISIONS.md)**. Never delete entries; supersede them.

Build and test: `cargo fmt --check && cargo lint && cargo lint-wasm && cargo test-all` (aliases in `.cargo/config.toml`). Testing conventions (harness, `known_issue_*` tests): docs/ENGINEERING.md §4.
The hub uses cargo-leptos (`cargo leptos build`). `blentinelmake` wraps the builds.

Never commit secrets: `*.key`, `*.token`, local `blentinel_*.toml`, or `*.db`.

Do **not** create or modify CI configuration (`.woodpecker.yml`, `.github/workflows/*`) unless the owner explicitly asks (D-24).
Storage: services use the `storage::Storage` traits only, never SQL or a concrete backend (proposal 0011).
The LaTeX manual is generated from `docs/` (D-09). Never edit a `.tex` file by hand.
