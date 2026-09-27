# 0009 · Documentation Overhaul & Upkeep

| | |
|---|---|
| **Status** | Proposed |
| **Milestone** | M0 (restructure) · continuous (upkeep) |
| **Priority** | P0 (continuous, part of the Definition of Done) |
| **Decisions** | D-08, D-09, D-11, D-19 |

## 1. Problems in the current docs (review 2026-09-27)

| Problem | Where |
|---|---|
| README links to `HTTPS_SETUP_GUIDE.md`, `HTTPS_IMPLEMENTATION.md`, `HOT_RELOAD_TESTING.md`, but they are **git-ignored**, so the links are broken for everyone else | README, `.gitignore` |
| "Installation Instructions" are really **build-from-source** instructions (Rust toolchain, wasm target, cargo-leptos) | README |
| Clone URL points to Codeberg; the project has moved to GitHub | README |
| "You cannot cross compile the Hub due to it using leptos": inaccurate (D-11) | README |
| Probe onboarding is 5 manual steps with key copy-paste and a TOML edit | README Quick Start |
| Full directory tree in the README (goes stale, includes runtime files like `blentinel.db`) | README |
| Typo `belntinel_probe` in the setcap command | README |
| `hub/README.md` is the untouched Leptos starter template | hub |
| `PROGRESS.md` is badly outdated (see D-08) | root (ignored) |
| `manual/manual.tex`: a second, hand-maintained manual (1,938 lines) that will drift from the README | manual/ |
| Mixed formatting: the README uses indentation instead of Markdown headings, so GitHub renders much of it as code blocks | README |
| No CHANGELOG, no CONTRIBUTING, no API reference, no check reference | — |

## 2. Target structure

See [`ENGINEERING.md` §6](../ENGINEERING.md#6-documentation-standards). Summary:

- **README.md** (≤ ~150 lines): what/why, the architecture diagram, the **founding principles in 5 bullets**, a 3-step quick start (install hub → setup wizard → add probe), feature list, links. Proper Markdown headings.
- **docs/install/**: `hub.md`, `probe.md` (one-command flow + manual fallback + uninstall + troubleshooting).
- **docs/guide/**: admin and technician guides per feature.
- **docs/reference/**: `checks.md`, `config.md` (generated from `--init` output), `api.md`, `cli.md`.
- **docs/architecture.md**, **docs/security.md**: consolidating `HTTPS_IMPLEMENTATION.md` and the security sections of the README.
- **CONTRIBUTING.md**: build from source, `blentinelmake`, tests, Definition of Done.
- **CHANGELOG.md**.

## 3. Keeping docs current (the process)

1. **Definition of Done** (ENGINEERING §1) requires docs in the same PR. Reviewers reject PRs that change behaviour without docs.
2. **PR template** with a checkbox list: principle check, tests, docs pages touched, CHANGELOG.
3. **Link check** so broken links fail the build (local tool for now; CI once enabled, D-24).
4. **Generated reference sections** where possible (DRY):
   - config reference ← output of `hub --init` / `probe --init` (commented templates);
   - CLI reference ← `--help` output;
   - check reference tables ← `CheckSpec` metadata;
   - API reference ← route table plus DTO types (OpenAPI via `utoipa`, P2).
   A check (a `blentinelmake docs` target now, CI once enabled) regenerates them and fails if the committed docs differ.
5. **Roadmap upkeep:** `docs/ROADMAP.md` status is updated in the PR that completes an item. Proposals move from Proposed → Accepted → Implemented, with a link to the PR.
6. **Release checklist** includes "install docs tested on a clean machine".

## 4. Immediate actions (M0)

- [ ] Fix `.gitignore` so docs are tracked; move the valid content of the HTTPS and hot-reload docs into `docs/`.
- [ ] Delete `PROGRESS.md` and `tree.txt` (content superseded by ROADMAP/DECISIONS).
- [ ] Replace `hub/README.md` with a short developer note, or remove it.
- [ ] README: fix the clone URL and the cross-compile claim; convert to real Markdown headings. (Typo fixed and links to PRINCIPLES / ROADMAP / ENGINEERING added on 2026-09-27.)
- [ ] D-09 (accepted): migrate content that exists only in `manual/manual.tex` into `docs/`, then generate the manual with pandoc and a template built from the current preamble (ROADMAP M0-15).
- [ ] Add CHANGELOG.md (start at the current version, 0.1.0) and CONTRIBUTING.md.
- [x] Mark `docs/superpowers/specs/2026-03-18-smooth-ui-polling-design.md` as Superseded (D-12).
