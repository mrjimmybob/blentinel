# hub

The Blentinel hub: HTTP API, web UI (Leptos: server-rendered, hydrated with WebAssembly) and the services behind them.

- How it's structured: [../docs/architecture.md](../docs/architecture.md#hub)
- Building and testing: [../CONTRIBUTING.md](../CONTRIBUTING.md) (`cd hub && cargo leptos build`, `cargo test-all`)
- Layout: `src/api.rs`, `src/ingest.rs` (HTTP) → `src/service/` (rules) → `src/storage/` (database); `src/app.rs` (UI); `src/model.rs` (types shared with the UI); `migrations/sqlite/` (schema); `tests/` (integration tests)
