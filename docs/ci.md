# CI: the gate every change passes

> Design: [proposal 0013](proposals/0013-ci-cd.md) · decisions D-102, D-103, D-104 · this page covers **phase 1** (the gate) and **phase 2** ([build once](#build-once-every-target-built-once)). Candidates, staging and production come in later phases.

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

Only these third-party actions are used:

- `Swatinem/rust-cache`, the build cache;
- `taiki-e/install-action`, which installs `cargo-deny`, `cargo-leptos` and `cargo-zigbuild` from their released binaries, checksums checked;

Rust itself comes from the runner's own `rustup`, Chrome and Node.js are already on GitHub's Ubuntu image, and `zig` (the cross-compiler) is downloaded from ziglang.org and checked against the SHA-256 pinned in [`install-zig.sh`](../.github/scripts/install-zig.sh).

## Build once: every target, built once

Defined in [`.github/workflows/build.yml`](../.github/workflows/build.yml) (phase 2, D-104). It runs on every push to `main`, every night at 03:23 UTC, by hand, and on a pull request that changes how releases are made (the workflow, its scripts, `blentinelmake`, the release formats, `hub upgrade`, `Cargo.lock`). It isn't a required check: the gate is.

| Job | Does |
|---|---|
| `probe (<target>)` | builds the probe with `blentinelmake release binaries` for Linux x86_64, ARM64 (`aarch64`), ARMv7, ARMv6 (`arm-unknown-linux-musleabihf`, the Pi Zero) and Windows x86_64. Linux targets cross-compile with `cargo-zigbuild`. |
| `hub (x86_64-unknown-linux-musl)` | builds the hub, web UI embedded. |
| `binaries` | checks each part against its `SHA256SUMS`; checks every Linux binary is **static** (no interpreter, no shared library) and built for the **processor** its name says, the ARMv6 one with no ARMv7 code in it ([`check-binaries.sh`](../.github/scripts/check-binaries.sh)); keeps them all as the artifact `binaries-<commit>`, with one `SHA256SUMS`, for 30 days. The run's summary lists each file's size and SHA-256. |
| `tests (<target> on <cpu>)` | runs the tests of `common` and `probe` built for ARMv6, ARMv7 and ARM64 under `qemu`, emulating the lab's processors: the Pi Zero's ARM1176, a Cortex-A7, a Cortex-A72. |
| `test-build` → `test-sign` → `install and rollback (systemd)` | the install and rollback test, below |
| `macOS` | `cargo test -p common -p probe`, **informational**: a failure shows and blocks nothing (0013 §12). FreeBSD, informational too, waits for its VM action to be allowed (below) |

### Built once, signed apart

A hub release is made in three steps, which `blentinelmake release hub` runs in one go on the owner's machine, and CI runs in separate jobs:

1. **`blentinelmake release binaries`** compiles. Every dependency's build script runs here, so this job holds **no key**. It reads the public keys to compile in from `$BLENTINEL_RELEASE_KEYS` (the repository variable of that name: the production public keys).
2. **`blentinelmake release assemble`** writes the release around those bytes, unsigned: the hub, `probes/` with its `manifest.json`, `release.json` with the version and the **commit**, and `hub.sh`.
3. **Signing** writes `manifest.json.sig`, `probes.tar.gz`, `SHA256SUMS` and `SHA256SUMS.sig` in the existing formats (D-55, D-57). In CI it is [`sign-release.sh`](../.github/scripts/sign-release.sh): `openssl`, `sha256sum` and `tar` only. The job sets up no Rust and runs no code from the build, so a malicious dependency can't reach the key. That matters from phase 4 on, when the staging key signs candidates.

### The install and rollback test

The runner is a real Ubuntu with systemd, thrown away afterwards: the right place to install services as root. [`systemd-test.sh`](../.github/scripts/systemd-test.sh):

1. runs `hub.sh` on the signed release folder, as a user would (`BLENTINEL_RELEASE_URL=file://…`): the signature is checked with `openssl`, the hub installed as a service, the probes added;
2. creates the first Owner, a company, a site and an enrollment code through the API;
3. installs the probe the hub added with `probe install --code`, and waits for its first report;
4. runs `hub upgrade` to a **deliberately broken release** ([`broken-release.sh`](../.github/scripts/broken-release.sh)): newer and properly signed, its hub passes the self-test, then ruins the database and exits. The upgrade must fail and put back the old program **and** the old database: the Owner can log in, the companies are the same, the probe reports again;
5. uninstalls both.

These releases are a **test build**: they trust only the public test key of the browser tests ([`hub/end2end/release-key.json`](../hub/end2end/release-key.json)), whose private half is in the repository on purpose. A binary that trusts it can never pass for a release.

### When the build fails

- `binaries` names the file and what is wrong with it: dynamically linked, the wrong processor, ARMv7 code in the ARMv6 build.
- `tests (…)`: a test that fails only under one CPU, or `SIGILL` (illegal instruction), means code for a newer processor got in.
- `install and rollback (systemd)`: the log ends with the hub's and the probe's journal.
- Reproduce a target locally with `blentinelmake release binaries --out dist --probes <target>` (Linux targets need `cargo-zigbuild` and `zig`).

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

**For the Build workflow (phase 2):**

- **Settings → Secrets and variables → Actions → Variables → New repository variable:** `BLENTINEL_RELEASE_KEYS`, the production release public key in hex (`release.pub` in the key folder; several separated by commas). A variable, not a secret: public keys aren't secret, and the builds compile it in. Unset, the binaries trust no release key and the run's summary says so.
- **Optional, for FreeBSD:** allow `vmactions/freebsd-vm@*` (a FreeBSD VM on the Linux runner); a pull request then adds the informational FreeBSD job. Not before: an action outside the allow-list doesn't fail its job, it stops the **whole workflow** before any job starts.

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
