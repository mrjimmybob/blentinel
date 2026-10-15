# CI: the gate every change passes

> Design: [proposal 0013](proposals/0013-ci-cd.md) · decisions D-102, D-103 · this page covers **phase 1** (the gate). Releases, staging and production come in later phases.

Every pull request into `main` runs the **gate**: formatting, lints, every test on Linux and Windows, a check of the dependencies, and the browser tests. `main` accepts a change only when all of it passed. The same gate runs on your machine with one command, so a green run at home means a green run on GitHub.

## The jobs

Defined in [`.github/workflows/ci.yml`](../.github/workflows/ci.yml). They run in parallel, each on a fresh GitHub-hosted machine that is thrown away afterwards.

| Job (required check) | Runs | Catches |
|---|---|---|
| `fmt` | `cargo fmt --all --check` | code not formatted by `rustfmt` |
| `lint` | `cargo lint`, `cargo lint-wasm` | every clippy warning, in the server code and in the browser (WASM) code |
| `test (ubuntu-latest)`, `test (windows-latest)` | `cargo test-all` | a failing test on either system: the probe and hub behave differently on each |
| `deny` | `cargo deny check` | a dependency with a known vulnerability, a licence we can't ship, a crate from an unknown source ([`deny.toml`](../deny.toml)) |
| `browser` | `npm ci`, `npm run build`, `npm test` in `hub/end2end` | the web UI against a real hub and probe ([CONTRIBUTING.md](../CONTRIBUTING.md#browser-tests)); a failed run keeps its Playwright report as an artifact for 14 days |

They run on every pull request into `main`, on every push to `main` (a merge), and by hand (**Actions → CI → Run workflow**).

## Run it locally

```sh
blentinelmake ci              # every step, in order, stopping at the first failure
blentinelmake ci fmt lint     # only these steps
```

Run it from the workspace root. The steps are `fmt`, `lint`, `test`, `deny` and `browser`. What each step runs is defined once:

- the cargo aliases in [`.cargo/config.toml`](../.cargo/config.toml) (`lint`, `lint-wasm`, `test-all`);
- [`deny.toml`](../deny.toml) for `cargo deny`;
- `hub/end2end/package.json` for the browser tests.

`blentinelmake ci` and the workflow only call them. A test (`the_workflow_runs_every_step` in `blentinelmake/src/ci.rs`) fails if the workflow stops running a command `blentinelmake ci` runs, so the two can't drift apart.

**Tools it needs**, beyond Rust:

- `cargo install cargo-deny --locked`
- for `browser`: `cargo install cargo-leptos --locked`, Node.js, and Google Chrome (see CONTRIBUTING.md)

It says which one is missing.

**The Rust version** is pinned in [`rust-toolchain.toml`](../rust-toolchain.toml), with `rustfmt`, `clippy` and the WASM target. `rustup` installs it the first time you build. Everyone, and every CI job, compiles with the same compiler and gets the same lints. Moving to a newer Rust is a pull request of its own: change the version and fix what its new lints find. That way they never break `main` by surprise.

## When the gate fails

- Open the pull request's **Checks** tab, then the red job, then the red step: the log is the same as you'd see locally.
- Reproduce it with `blentinelmake ci <step>`.
- **Windows only:** run `cargo test-all` on Windows. Paths, line endings, services and permissions differ, which is why both systems are in the gate.
- **`browser` only:** download the `playwright-report` artifact from the run's summary page and open `index.html`. It has the trace of each failed test (every action, the page at each step, the console).
- **`deny`:** the log names the crate and the advisory, or the licence. Update the crate (`cargo update -p <crate>`). Ignore an advisory in `deny.toml` only with a reason and a date, as for a `known_issue_*` test.
- Never re-run until it passes. A test that fails now and then is a bug: fix it or quarantine it with a note (M0-26 is one).

## How the workflow is protected

| Rule | Where | Why |
|---|---|---|
| GitHub-hosted runners only | every job's `runs-on` | nothing on the LAN can be reached by a workflow; no self-hosted runner is registered at all |
| The token can only read | `permissions: contents: read` | a compromised step can't push code or change the repository |
| No secrets in this workflow | — | a pull request, even from a fork, has nothing to steal |
| Third-party actions pinned to a full commit SHA, version in a comment | each `uses:` | a tag such as `@v2` can be moved to other code; a commit SHA can't. Dependabot (`.github/dependabot.yml`) proposes new SHAs weekly, as pull requests that the gate tests |
| Caches written only on `main` | `save-if` of `Swatinem/rust-cache` | a pull request reads the cache but can't plant something in it for `main` |
| `persist-credentials: false` | every checkout | the token isn't left on the disk for later steps |
| A new push cancels its pull request's older run | `concurrency` | no minutes spent on code that changed; `main`'s runs always finish |

Only two third-party actions are used:

- `Swatinem/rust-cache`, the build cache;
- `taiki-e/install-action`, which installs `cargo-deny` and `cargo-leptos` from their released binaries, checksums checked.

Rust itself comes from the runner's own `rustup`, and Chrome and Node.js are already on GitHub's Ubuntu image.

## Repository settings (the owner, once)

These live in GitHub's settings, not in the repository, so they are listed here to apply by hand. **Order matters:** a required check can only be chosen once it has run, so push the branch and open the pull request first (step 1), then set up the rest.

### 1. Push the work and open the pull request

```sh
git push -u origin m0-foundations
gh pr create --base main --head m0-foundations --title "M0 to M5: everything up to the CI gate" --body "See CHANGELOG.md."
```

The gate runs on the pull request. Fix what it finds on the branch; each push runs it again.

### 2. Pull requests (Settings → General → Pull Requests)

- **Allow merge commits:** on. **Allow squash merging** and **Allow rebase merging:** off. A merge commit keeps each commit, and their D-numbers stay in the history.
- **Automatically delete head branches:** on.

### 3. Actions (Settings → Actions → General)

- **Actions permissions:** *Allow mrjimmybob, and select non-mrjimmybob, actions and reusable workflows*:
  - **Allow actions created by GitHub:** on.
  - **Allow specified actions and reusable workflows:** `Swatinem/rust-cache@*, taiki-e/install-action@*`.
  - **Require actions to be pinned to a full-length commit SHA:** on.
- **Fork pull request workflows from outside collaborators:** *Require approval for all external contributors*. A stranger's pull request runs nothing until you have read it and approved the run.
- **Workflow permissions:** *Read repository contents and packages permissions*. **Allow GitHub Actions to create and approve pull requests:** off.

### 4. Protect `main` (Settings → Rules → Rulesets → New branch ruleset)

- **Name:** `main`. **Enforcement status:** Active. **Bypass list:** empty. Nobody skips the gate, including the owner.
- **Target branches:** *Include default branch*.
- **Rules:**
  - **Restrict deletions:** on.
  - **Block force pushes:** on.
  - **Require a pull request before merging:** on, with **Required approvals** 0 (GitHub doesn't let you approve your own pull request) and **Allowed merge methods** *Merge* only.
  - **Require status checks to pass:** on, with **Require branches to be up to date before merging** on. Add the checks `fmt`, `lint`, `test (ubuntu-latest)`, `test (windows-latest)`, `deny` and `browser`, each with source *GitHub Actions*.

"Up to date" means the gate ran on the branch *with* the latest `main` in it. Two changes that each pass alone can still fail together.

### 5. Protect release tags (Settings → Rules → Rulesets → New tag ruleset)

- **Name:** `releases`. **Enforcement status:** Active. **Bypass list:** *Repository admin* (you).
- **Target tags:** *Include by pattern* `v*`.
- **Rules:** **Restrict creations**, **Restrict updates**, **Restrict deletions**, **Block force pushes**: all on.

A tag is a release (phase 4): only you make one, and none is ever moved or deleted.

### 6. Security (Settings → Advanced Security)

- **Dependabot alerts:** on.
- **Secret scanning** and **Push protection:** on. A key or token pushed by mistake is refused before it lands.

### 7. Merge

When the gate is green on the pull request, merge it with **Create a merge commit**. From then on every change comes through a branch and a pull request, one per roadmap item.
