# blentinelmake

A Rust CLI tool for building and publishing Blentinel workspace components.

## Purpose

This tool replaces the existing PowerShell build and publish scripts (`build_hub.ps1`, `build_probe.ps1`, `publish_hub.ps1`, `publish_probe.ps1`) with a single, cross-platform Rust binary that maintains behavioral parity with the original scripts.

## Installation

From the workspace root:

```bash
cargo build -p blentinelmake --release
```

The binary will be available at `target/release/blentinelmake` (or `blentinelmake.exe` on Windows).

## Usage Modes

### Interactive Mode (NEW!)

Run without any arguments to enter an interactive menu-driven interface:

```bash
blentinelmake
```

This will:
1. Present arrow-key navigable menus for component, action, and target selection
2. Show a summary of your selections
3. Ask for confirmation before executing

Perfect for:
- New users learning the tool
- Quick builds without remembering exact syntax
- Visual confirmation before long operations

### CLI Mode (Original)

Use command-line arguments for scripting and automation:

```
blentinelmake <component> <action> [OPTIONS]
```

### Components
- `hub` - Leptos-based monitoring hub
- `probe` - Network monitoring probe

### Actions
- `build` - Build the component (debug or release)
- `publish` - Build in release mode and package for distribution
- `clean` - Remove build artifacts

### Options
- `--release` - Build in release mode (for `build` action only)
- `--target <triple>` - Target triple for cross-compilation (probe only)
- `--help` - Show help message
- `--version` - Show version

### Examples

```bash
# Build probe in debug mode
blentinelmake probe build

# Build probe in release mode
blentinelmake probe build --release

# Cross-compile probe for Linux
blentinelmake probe publish --target x86_64-unknown-linux-musl

# Build hub in release mode
blentinelmake hub build --release

# Publish hub (always release mode)
blentinelmake hub publish

# Clean probe build artifacts
blentinelmake probe clean

# Clean with target
blentinelmake probe clean --target x86_64-unknown-linux-musl
```

## Implementation Details

### Hub
- Build uses `cargo leptos build [--release]`
- Publish creates `publish/hub/app/` with:
  - Hub binary
  - SHA256SUM file (integrity verification)
  - Sample config (`blentinel_hub.toml`)
  - Service installation files (systemd, PowerShell, bash)
  - `publish/hub.zip` containing the full package
- Clean removes `target/front`, `target/site`, and runs `cargo clean -p hub`

### Probe
- Build uses `cargo build -p probe [--release] [--target <triple>]`
  - Uses `cargo zigbuild` for Linux targets if available
- Publish creates `publish/probe/<target>/app/` with:
  - Probe binary (stripped on non-Windows)
  - SHA256SUM file (integrity verification)
  - Sample config (`blentinel_probe.toml`)
  - Service installation files (systemd, PowerShell for Windows targets)
  - `hub_cert.pem` if available in `probe/` directory
  - `publish/probe-<target>.zip` containing the full package
- Clean runs `cargo clean -p probe [--target <triple>]`
- Auto-detects native target if `--target` is not specified

### Directory Structure
```
publish/
├── hub/
│   └── app/
│       ├── blentinel_hub.toml
│       ├── blentinel-hub.service
│       ├── install_hub_service.ps1
│       ├── install_hub_service.sh
│       ├── hub(.exe)
│       └── SHA256SUM           # NEW: Integrity verification
├── hub.zip
├── probe/
│   └── <target>/
│       └── app/
│           ├── blentinel_probe.toml
│           ├── blentinel-probe.service
│           ├── install_probe_service.ps1 (Windows targets only)
│           ├── hub_cert.pem (if available)
│           ├── probe(.exe)
│           └── SHA256SUM       # NEW: Integrity verification
└── probe-<target>.zip
```

### SHA256 Integrity Verification

Each published package includes a `SHA256SUM` file containing the cryptographic hash of the binary. This allows you to verify the integrity of downloaded binaries:

```bash
# Verify hub binary (Linux/macOS)
cd publish/hub/app
sha256sum -c SHA256SUM

# Verify hub binary (Windows PowerShell)
cd publish\hub\app
$hash = Get-FileHash hub.exe -Algorithm SHA256
$expected = (Get-Content SHA256SUM).Split()[0]
if ($hash.Hash -eq $expected) { "OK" } else { "FAILED" }

# Verify probe binary (Linux/macOS)
cd publish/probe/x86_64-unknown-linux-musl/app
sha256sum -c SHA256SUM
```

Format: `<hash>  <filename>` (standard SHA256SUM format)

## Design Principles

1. **Behavioral Parity**: Exactly replicates PowerShell script functionality
2. **Minimal Dependencies**: Only essential dependencies added
   - `dialoguer 0.11` - Interactive mode terminal UI
   - `sha2 0.10` - Cryptographic integrity verification
3. **Manual Parsing**: Follows probe's argument parsing style (no clap)
4. **Cross-Platform**: Works on Windows, Linux, and macOS
5. **Simple & Reviewable**: Single-file implementation, ~900 lines
6. **Clear Error Messages**: Follows existing error message patterns
7. **Security**: SHA256 checksums for published binaries

## Interactive Mode Implementation

The interactive mode is implemented as a thin UI layer that:
- Uses the `dialoguer` crate for terminal prompts (keyboard-only, arrow key navigation)
- Presents three selection screens in sequence:
  1. Component (hub / probe)
  2. Action (build / publish / clean)
  3. Target (only for probe: native / linux-gnu / linux-musl / aarch64 / windows)
- Shows a formatted summary box before execution
- Asks for Y/n confirmation
- Dispatches to the **exact same `run()` function** as CLI mode
- No duplication of build/publish logic
- Can be cancelled at any step (exit code 0)

**Implementation details:**
- Interactive mode is detected by `args.len() == 1` (only program name, no arguments)
- Single dependency added: `dialoguer 0.11` (small, focused, no async)
- All existing CLI behavior preserved exactly
- No changes to build/publish/clean logic

## Constraints

- Hub does NOT support cross-compilation
- Publish ALWAYS builds in release mode
- Publish ALWAYS strips binaries on non-Windows platforms (best effort)
- No watch mode support (use cargo watch directly)
- No daemon/service control (start/stop/status)
