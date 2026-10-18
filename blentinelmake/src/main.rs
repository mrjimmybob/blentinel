mod ci;
mod hub_release;
mod release;

use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, exit};

use dialoguer::{Confirm, Select, theme::ColorfulTheme};
use sha2::{Digest, Sha256};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
Usage: blentinelmake <component> <action> [OPTIONS]

Build and publish tool for Blentinel workspace components.

Components:
  hub       Leptos-based monitoring hub
  probe     Network monitoring probe

Actions:
  build     Build the component
  publish   Build in release mode, package for distribution
  clean     Remove build artifacts

Options:
  --target <triple>    Target triple for cross-compilation (probe only)
  --release            Build in release mode (build action only)
  --with-probes <triple,...>
                       hub publish: also build these probes, signed, into probes/
                       in the package; `hub install` adds them as a release
                       (Add probe offers them; needs a release key)
  -h, --help           Print this help message
  --version            Print version

Examples:
  blentinelmake probe build
  blentinelmake probe build --release
  blentinelmake probe publish --target x86_64-unknown-linux-musl
  blentinelmake hub publish
  blentinelmake hub publish --with-probes x86_64-unknown-linux-musl,x86_64-pc-windows-msvc
  blentinelmake probe clean --target x86_64-unknown-linux-musl

Probe releases (signed; probes update themselves to them, D-55):
  blentinelmake release keygen
                       Create the release key pair (once; the private key stays
                       on this machine). Every hub and probe built afterwards
                       trusts it. Folder: $BLENTINEL_RELEASE_KEY_DIR, else
                       blentinel/release-keys in your configuration folder.
  blentinelmake release build --targets <triple,...>
                       Build these probes, sign them as the workspace version:
                       publish/probe-release-<version>/ and its .tar.gz (add it
                       on the hub: admin page, or `hub release add`)
  blentinelmake release sign <folder> [--version <v>]
                       Sign the probe-<target> binaries already in a folder
  blentinelmake release rollback --to <version> --from <version,...>
                       Let probes on the --from versions go back to the older
                       release --to (probes never downgrade otherwise, D-74):
                       publish/probe-rollback-<to>/ and its .tar.gz (add it on
                       the hub like a release, then approve <to>)

Hub releases (for the one-line install, hub.sh, D-57; run on Linux):
  blentinelmake release hub --targets <triple,...> --probes <triple,...>
                       Build the hub for these Linux targets
                       (x86_64-unknown-linux-musl, aarch64-unknown-linux-musl)
                       and the probes, assemble them and sign them with the
                       release key: hub.sh with the release keys, a signed
                       SHA256SUMS:
                       publish/github-release-<version>/, every file of which
                       goes into GitHub release v<version>. Needs a clean tree:
                       release.json names the commit it was built from
  The same in steps, as CI runs them (compiling and signing apart, D-104):
  blentinelmake release binaries --out <folder> [--hub <triple,...>] [--probes <triple,...>]
                       Build these hubs and probes into a folder, under their
                       release names, with SHA256SUMS. Reads no key: the public
                       keys come from $BLENTINEL_RELEASE_KEYS, else the key folder
  blentinelmake release assemble --from <folder> --out <folder> [--commit <sha>]
                       An unsigned release from those binaries (each checked
                       against their SHA256SUMS): the hubs, probes/ with its
                       manifest.json, release.json (version, commit: HEAD unless
                       given) and hub.sh. Signing it is the last step

CI gate (what every pull request must pass, D-102; from the workspace root):
  blentinelmake ci [step ...]
                       Run fmt, lint, test, deny and browser (the browser tests),
                       or only the steps named, as GitHub Actions does; stops at
                       the first failure

Documentation:
  blentinelmake docs          Build the PDF manual from docs/ (needs pandoc + xelatex)
  blentinelmake docs --tex     Write the generated LaTeX instead of a PDF
";

fn main() {
    let args: Vec<String> = env::args().collect();

    // ────────────────────────────────────────────────────────────
    // INTERACTIVE MODE: Triggered when run with NO arguments
    // ────────────────────────────────────────────────────────────
    if args.len() == 1 {
        if let Err(e) = run_interactive() {
            eprintln!("\nError: {}", e);
            exit(1);
        }
        return;
    }

    // ────────────────────────────────────────────────────────────
    // CLI MODE: Original argument parsing (preserved exactly)
    // ────────────────────────────────────────────────────────────

    // `release` signs probe releases (D-55). Before the flags below: its
    // `--version <v>` is the release's, not this program's.
    if args[1] == "release" {
        if let Err(e) = run_release(&args[2..]) {
            eprintln!("\nError: {}", e);
            exit(1);
        }
        return;
    }

    // Handle --version and --help
    for arg in &args[1..] {
        match arg.as_str() {
            "--version" => {
                println!("{}", VERSION);
                exit(0);
            }
            "-h" | "--help" => {
                print!("{}", HELP);
                exit(0);
            }
            _ => {}
        }
    }

    // `ci` runs the CI gate locally, as GitHub Actions runs it (D-102).
    if args[1] == "ci" {
        if let Err(e) = ci::run(&args[2..]) {
            eprintln!("\nError: {}", e);
            exit(1);
        }
        return;
    }

    // `docs` is a standalone command (not a component): build the manual.
    if args[1] == "docs" {
        let tex_only = args[2..].iter().any(|a| a == "--tex");
        if let Err(e) = build_manual(tex_only) {
            eprintln!(
                "
Error: {}",
                e
            );
            exit(1);
        }
        return;
    }

    // Parse component and action
    if args.len() < 3 {
        eprintln!("Error: Missing required arguments");
        eprintln!("Run with --help for usage information.");
        exit(1);
    }

    let component = match args[1].as_str() {
        "hub" => Component::Hub,
        "probe" => Component::Probe,
        other => {
            eprintln!("Error: Unknown component '{}'", other);
            eprintln!("Valid components: hub, probe");
            exit(1);
        }
    };

    let action = match args[2].as_str() {
        "build" => Action::Build,
        "publish" => Action::Publish,
        "clean" => Action::Clean,
        other => {
            eprintln!("Error: Unknown action '{}'", other);
            eprintln!("Valid actions: build, publish, clean");
            exit(1);
        }
    };

    // Parse options
    let mut release = false;
    let mut target: Option<String> = None;
    let mut with_probes: Vec<String> = Vec::new();
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--release" => {
                release = true;
                i += 1;
            }
            "--target" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --target requires a value");
                    exit(1);
                }
                target = Some(args[i + 1].clone());
                i += 2;
            }
            "--with-probes" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --with-probes requires a list of target triples");
                    exit(1);
                }
                with_probes = args[i + 1]
                    .split(',')
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(String::from)
                    .collect();
                i += 2;
            }
            other => {
                eprintln!("Error: Unknown option '{}'", other);
                exit(1);
            }
        }
    }

    // Validate target usage
    if target.is_some() && component == Component::Hub {
        eprintln!("Error: Hub does not support cross-compilation");
        eprintln!("The --target flag is only valid for probe");
        exit(1);
    }

    if !with_probes.is_empty() && (component, action) != (Component::Hub, Action::Publish) {
        eprintln!("Error: --with-probes is only valid for `hub publish`");
        exit(1);
    }

    // Publish always uses release mode
    if action == Action::Publish {
        release = true;
    }

    // Execute
    if let Err(e) = run(component, action, release, target, &with_probes) {
        eprintln!("\nError: {}", e);
        exit(1);
    }
}

/// `blentinelmake release <keygen | build | sign | rollback | hub>`.
fn run_release(args: &[String]) -> Result<(), String> {
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let list = |flag: &str| -> Vec<String> {
        value(flag)
            .unwrap_or_default()
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    };
    match args.first().map(String::as_str) {
        Some("keygen") => release::keygen(),
        Some("binaries") => hub_release::binaries(
            Path::new(&value("--out").ok_or("release binaries needs --out <folder>")?),
            &list("--hub"),
            &list("--probes"),
            build_release_hub,
            build_release_probe,
        ),
        Some("assemble") => {
            let commit = match value("--commit") {
                Some(commit) => commit,
                None => hub_release::head_commit()?,
            };
            hub_release::assemble(
                Path::new(&value("--from").ok_or("release assemble needs --from <folder>")?),
                Path::new(&value("--out").ok_or("release assemble needs --out <folder>")?),
                &release::workspace_version()?,
                &commit,
                &release::public_keys()?,
            )
        }
        Some("hub") => {
            hub_release::build(
                &list("--targets"),
                &list("--probes"),
                build_release_hub,
                build_release_probe,
            )
        }
        Some("build") => {
            let targets: Vec<String> = value("--targets")
                .ok_or("release build needs --targets <triple,...>")?
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            release::build(&targets, build_release_probe)
        }
        Some("sign") => {
            let folder = args
                .get(1)
                .filter(|a| !a.starts_with('-'))
                .ok_or("release sign needs the folder")?;
            let version = match value("--version") {
                Some(v) => v,
                None => release::workspace_version()?,
            };
            release::sign(Path::new(folder), &version).map(|_| ())
        }
        Some("rollback") => {
            let to = value("--to").ok_or("release rollback needs --to <version>")?;
            let from: Vec<String> = value("--from")
                .ok_or("release rollback needs --from <version,...>")?
                .split(',')
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .collect();
            release::rollback(&to, &from)
        }
        _ => Err(
            "Usage: blentinelmake release <keygen | build --targets <triple,...> | sign <folder> | rollback --to <version> --from <version,...> | hub --targets <triple,...> --probes <triple,...> | binaries --out <folder> [--hub <triple,...>] [--probes <triple,...>] | assemble --from <folder> --out <folder> [--commit <sha>]>"
                .into(),
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Component {
    Hub,
    Probe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Build,
    Publish,
    Clean,
}

// ════════════════════════════════════════════════════════════════════════════
// INTERACTIVE MODE
// ════════════════════════════════════════════════════════════════════════════
// When invoked with no arguments, this function presents a terminal UI
// to gather component, action, and target selections, then dispatches
// to the same execution path as the CLI mode.
// ════════════════════════════════════════════════════════════════════════════

fn run_interactive() -> Result<(), String> {
    let theme = ColorfulTheme::default();

    println!("\n╔═══════════════════════════════════════════════════════════╗");
    println!("║            Blentinel Make - Interactive Mode              ║");
    println!("╚═══════════════════════════════════════════════════════════╝\n");

    // ────────────────────────────────────────────────────────────
    // Step 1: Select component
    // ────────────────────────────────────────────────────────────
    let components = &["hub", "probe", "cancel"];
    let component_idx = Select::with_theme(&theme)
        .with_prompt("Select component")
        .items(components)
        .default(0)
        .interact()
        .map_err(|e| format!("Selection cancelled: {}", e))?;

    let component = match components[component_idx] {
        "hub" => Component::Hub,
        "probe" => Component::Probe,
        "cancel" => {
            println!("\nCancelled by user.");
            exit(0);
        }
        _ => unreachable!(),
    };

    // ────────────────────────────────────────────────────────────
    // Step 2: Select action
    // ────────────────────────────────────────────────────────────
    let actions = &["build", "publish", "clean", "cancel"];
    let action_idx = Select::with_theme(&theme)
        .with_prompt("Select action")
        .items(actions)
        .default(0)
        .interact()
        .map_err(|e| format!("Selection cancelled: {}", e))?;

    let action = match actions[action_idx] {
        "build" => Action::Build,
        "publish" => Action::Publish,
        "clean" => Action::Clean,
        "cancel" => {
            println!("\nCancelled by user.");
            exit(0);
        }
        _ => unreachable!(),
    };

    // ────────────────────────────────────────────────────────────
    // Step 3: Select build mode (build action only)
    // ────────────────────────────────────────────────────────────
    let release = if action == Action::Build {
        let build_modes = &[
            "Debug (faster compile, slower runtime)",
            "Release (optimized)",
            "cancel",
        ];
        let mode_idx = Select::with_theme(&theme)
            .with_prompt("Select build mode")
            .items(build_modes)
            .default(0)
            .interact()
            .map_err(|e| format!("Selection cancelled: {}", e))?;

        match mode_idx {
            0 => false, // Debug
            1 => true,  // Release
            2 => {
                println!("\nCancelled by user.");
                exit(0);
            }
            _ => unreachable!(),
        }
    } else {
        // Publish and clean always use release mode
        true
    };

    // ────────────────────────────────────────────────────────────
    // Step 4: Select target (probe only)
    // ────────────────────────────────────────────────────────────
    let target = if component == Component::Probe {
        let targets = &[
            "native (auto-detect current machine)",
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-musl",
            "aarch64-unknown-linux-gnu",
            "x86_64-pc-windows-msvc",
            "cancel",
        ];

        let target_idx = Select::with_theme(&theme)
            .with_prompt("Select target")
            .items(targets)
            .default(0)
            .interact()
            .map_err(|e| format!("Selection cancelled: {}", e))?;

        // Map display strings to actual target triples
        // "native" returns None (auto-detect), others return the triple
        match target_idx {
            0 => None, // native
            1 => Some("x86_64-unknown-linux-gnu".to_string()),
            2 => Some("x86_64-unknown-linux-musl".to_string()),
            3 => Some("aarch64-unknown-linux-gnu".to_string()),
            4 => Some("x86_64-pc-windows-msvc".to_string()),
            5 => {
                println!("\nCancelled by user.");
                exit(0);
            }
            _ => unreachable!(),
        }
    } else {
        None
    };

    // ────────────────────────────────────────────────────────────
    // Step 5: Show summary and confirm
    // ────────────────────────────────────────────────────────────
    println!("\n┌───────────────────────────────────────────────────────────┐");
    println!("│ Summary                                                   │");
    println!("├───────────────────────────────────────────────────────────┤");
    println!(
        "│ Component: {:<46} │",
        match component {
            Component::Hub => "hub",
            Component::Probe => "probe",
        }
    );
    println!(
        "│ Action:    {:<46} │",
        match action {
            Action::Build => "build",
            Action::Publish => "publish",
            Action::Clean => "clean",
        }
    );
    if action == Action::Build {
        println!(
            "│ Mode:      {:<46} │",
            if release { "Release" } else { "Debug" }
        );
    }
    if let Some(ref t) = target {
        println!("│ Target:    {:<46} │", t);
    }
    println!("└───────────────────────────────────────────────────────────┘\n");

    let confirmed = Confirm::with_theme(&theme)
        .with_prompt("Proceed?")
        .default(true)
        .interact()
        .map_err(|e| format!("Confirmation failed: {}", e))?;

    if !confirmed {
        println!("\nCancelled by user.");
        exit(0);
    }

    // ────────────────────────────────────────────────────────────
    // Execute: Dispatch to the same logic as CLI mode
    // ────────────────────────────────────────────────────────────
    println!();

    run(component, action, release, target, &[])
}

fn run(
    component: Component,
    action: Action,
    release: bool,
    target: Option<String>,
    with_probes: &[String],
) -> Result<(), String> {
    match component {
        Component::Hub => match action {
            Action::Build => hub_build(release, None),
            Action::Publish => hub_publish(with_probes),
            Action::Clean => hub_clean(),
        },
        Component::Probe => match action {
            Action::Build => probe_build(release, target.as_deref()),
            Action::Publish => probe_publish(target),
            Action::Clean => probe_clean(target.as_deref()),
        },
    }
}

// ============================================================================
// HUB
// ============================================================================

/// Build the hub; `target` (a Linux musl triple) cross-builds the server
/// for a hub release (D-57), from a Linux machine with `musl-tools`.
fn hub_build(release: bool, target: Option<&str>) -> Result<(), String> {
    println!("Building hub{}...", if release { " (release)" } else { "" });

    // Pre-flight: check if target/site/pkg is locked (hub probably running)
    let site_pkg = Path::new("target/site/pkg");
    if site_pkg.exists() {
        // Try to remove a probe file to test for locks
        let test_path = site_pkg.join(".build_lock_test");
        match fs::write(&test_path, b"test") {
            Ok(_) => {
                let _ = fs::remove_file(&test_path);
            }
            Err(_) => {
                // Can't write — try to detect if it's locked by checking removability
            }
        }
        // The real test: can cargo-leptos clean the directory?
        // Try renaming the pkg dir as a lock check
        let temp_name = site_pkg.with_file_name("pkg_build_check");
        match fs::rename(site_pkg, &temp_name) {
            Ok(_) => {
                // Not locked, rename back
                let _ = fs::rename(&temp_name, site_pkg);
            }
            Err(e) if e.raw_os_error() == Some(32) => {
                return Err(
                    "The hub appears to be running. Stop the hub process before building.\n\
                     (Windows locks files in target/site/pkg/ while the hub serves them)"
                        .to_string(),
                );
            }
            Err(_) => {
                // Some other error, let cargo-leptos handle it
            }
        }
    }

    // Check if cargo-leptos is installed
    let check = Command::new("cargo").args(["leptos", "--help"]).output();

    if check.is_err() || !check.unwrap().status.success() {
        println!("cargo-leptos not found, installing...");
        let status = Command::new("cargo")
            .args(["install", "cargo-leptos"])
            .status()
            .map_err(|e| format!("Failed to run cargo install: {}", e))?;

        if !status.success() {
            return Err("Failed to install cargo-leptos".to_string());
        }
    }

    // Two steps, so the server embeds the finished web UI (hub/build.rs,
    // D-51): cargo-leptos builds the front end and the server in parallel
    // otherwise.
    let site = fs::canonicalize(".")
        .map_err(|e| format!("Failed to resolve the workspace folder: {}", e))?
        .join(SITE_ROOT);
    for (step, embed) in [("--frontend-only", false), ("--server-only", true)] {
        let mut command = Command::new("cargo");
        command.current_dir("hub").args(["leptos", "build", step]);
        if release {
            command.arg("--release");
        }
        if embed {
            command.env(EMBED_SITE_ENV, &site);
            if let Some(target) = target {
                command.env("LEPTOS_BIN_TARGET_TRIPLE", target);
            }
        }
        release::apply_keys(&mut command)?;
        let status = command
            .status()
            .map_err(|e| format!("Failed to run cargo leptos build {}: {}", step, e))?;
        if !status.success() {
            return Err(format!("Hub build failed (cargo leptos build {})", step));
        }
    }

    println!("Hub build completed successfully (web UI embedded in the binary)");
    Ok(())
}

/// Where cargo-leptos writes the web UI (`site-root` in hub/Cargo.toml).
const SITE_ROOT: &str = "target/site";
/// Tells hub/build.rs to embed the web UI from that folder.
const EMBED_SITE_ENV: &str = "BLENTINEL_EMBED_SITE";

fn hub_publish(with_probes: &[String]) -> Result<(), String> {
    use chrono::Local;
    let timestamp = Local::now().format("hub-%Y%m%d_%H%M%S").to_string();
    let publish_root = Path::new("publish").join(&timestamp);
    let app_dir = publish_root.join("app");

    println!("Publishing hub...");

    // Build in release mode
    hub_build(true, None)?;

    // Create output directory
    fs::create_dir_all(&app_dir)
        .map_err(|e| format!("Failed to create output directory: {}", e))?;

    // One file: the web UI is embedded in the binary (D-51).
    // (cargo-leptos in workspace mode outputs to the workspace's target/.)
    let exe_name = if cfg!(windows) { "hub.exe" } else { "hub" };
    let src = Path::new("target/release").join(exe_name);
    let dst = app_dir.join(exe_name);

    fs::copy(&src, &dst)
        .map_err(|e| format!("Failed to copy hub binary from {}: {}", src.display(), e))?;

    // Generate SHA256 checksum
    generate_sha256sum(&dst, &app_dir)?;
    // No config or service files: `hub install` writes them (D-52), and the
    // web UI is inside the binary (D-51). A signed probe release for `hub
    // install` to add, which Add probe offers (D-53, D-55):
    if !with_probes.is_empty() {
        release::build_into(&app_dir.join("probes"), with_probes, build_release_probe)?;
    }

    // Create zip archive of the publish directory
    let zip_name = format!("{}.zip", timestamp);
    let zip_path = Path::new("publish").join(zip_name);

    create_zip(&publish_root, &zip_path)?;

    println!("\nPublish output:");
    println!("  {}", publish_root.display());
    println!("  {}", zip_path.display());
    println!("\nHub publish completed successfully");

    Ok(())
}

fn hub_clean() -> Result<(), String> {
    println!("Cleaning hub...");

    // Remove Leptos-specific directories (workspace target)
    remove_dir_if_exists("target/front")?;
    remove_dir_if_exists("target/site")?;

    // Clean cargo artifacts
    let status = Command::new("cargo")
        .current_dir("hub")
        .args(["clean"])
        .status()
        .map_err(|e| format!("Failed to run cargo clean: {}", e))?;

    if !status.success() {
        return Err("Cargo clean failed".to_string());
    }

    println!("Hub clean completed");
    Ok(())
}

// ============================================================================
// PROBE
// ============================================================================

fn probe_build(release: bool, target: Option<&str>) -> Result<(), String> {
    let target_display = target.unwrap_or("native");
    println!(
        "Building probe for {}{}...",
        target_display,
        if release { " (release)" } else { "" }
    );

    let mut args = vec!["build", "-p", "probe"];
    if release {
        args.push("--release");
    }
    if let Some(t) = target {
        args.push("--target");
        args.push(t);
    }

    // Use cargo-zigbuild for Linux targets if available
    let use_zigbuild = target.is_some_and(|t| t.contains("linux"));
    let cargo_cmd = if use_zigbuild && command_exists("cargo-zigbuild") {
        println!("Using cargo-zigbuild for Linux target");
        "zigbuild"
    } else {
        "build"
    };

    let mut final_args = vec![cargo_cmd, "-p", "probe"];
    if release {
        final_args.push("--release");
    }
    if let Some(t) = target {
        final_args.push("--target");
        final_args.push(t);
    }

    let mut command = Command::new("cargo");
    command.args(&final_args);
    release::apply_keys(&mut command)?;
    let status = command
        .status()
        .map_err(|e| format!("Failed to run cargo build: {}", e))?;

    if !status.success() {
        return Err("Probe build failed".to_string());
    }

    println!("Probe build completed successfully");
    Ok(())
}

fn probe_publish(target: Option<String>) -> Result<(), String> {
    // Auto-detect target if not specified
    let target = target.unwrap_or_else(detect_native_target);

    use chrono::Local;
    let timestamp = Local::now().format("probe-%Y%m%d_%H%M-%S").to_string();
    let publish_root = Path::new("publish").join(&timestamp);
    let app_dir = publish_root.join("app");

    println!("Publishing probe for {}...", target);

    let src = build_release_probe(&target)?;

    // Create output directory
    fs::create_dir_all(&app_dir)
        .map_err(|e| format!("Failed to create output directory: {}", e))?;
    let dst = app_dir.join(src.file_name().ok_or("the probe has a file name")?);
    fs::copy(&src, &dst).map_err(|e| format!("Failed to copy probe binary: {}", e))?;

    // Generate SHA256 checksum
    generate_sha256sum(&dst, &app_dir)?;

    // Nothing else: no config (its checks come from the hub, D-49), and
    // `probe install` sets up the service (D-53).

    // Create zip
    let zip_name = format!("{}-{}.zip", target, timestamp);
    let zip_path = Path::new("publish").join(zip_name);

    create_zip(&publish_root, &zip_path)?;

    println!("\nPublish output:");
    println!("  {}", publish_root.display());
    println!("  {}", zip_path.display());
    println!("\nProbe publish completed successfully");

    Ok(())
}

/// Build the hub server for `target` in release mode (stripped by the
/// release profile); where the binary is.
fn build_release_hub(target: &str) -> Result<PathBuf, String> {
    if !cfg!(target_os = "linux") {
        return Err(format!(
            "build the hub for {target} on Linux (with musl-tools and `rustup target add {target}`)"
        ));
    }
    hub_build(true, Some(target))?;
    Ok(PathBuf::from("target")
        .join(target)
        .join("release")
        .join("hub"))
}

/// Build the probe for `target` in release mode (stripped by the release
/// profile); where the binary is.
fn build_release_probe(target: &str) -> Result<PathBuf, String> {
    probe_build(true, Some(target))?;
    let exe_name = if target.contains("windows") {
        "probe.exe"
    } else {
        "probe"
    };
    Ok(PathBuf::from("target")
        .join(target)
        .join("release")
        .join(exe_name))
}

fn probe_clean(target: Option<&str>) -> Result<(), String> {
    println!("Cleaning probe...");

    let mut args = vec!["clean", "-p", "probe"];
    if let Some(t) = target {
        args.push("--target");
        args.push(t);
    }

    let status = Command::new("cargo")
        .args(&args)
        .status()
        .map_err(|e| format!("Failed to run cargo clean: {}", e))?;

    if !status.success() {
        return Err("Cargo clean failed".to_string());
    }

    println!("Probe clean completed");
    Ok(())
}

// ============================================================================
// CONFIG GENERATION
// ============================================================================

// ============================================================================
// UTILITIES
// ============================================================================

fn detect_native_target() -> String {
    // Simple detection based on current platform
    if cfg!(target_os = "windows") {
        if cfg!(target_arch = "x86_64") {
            "x86_64-pc-windows-msvc".to_string()
        } else if cfg!(target_arch = "aarch64") {
            "aarch64-pc-windows-msvc".to_string()
        } else {
            eprintln!("Warning: Unsupported Windows architecture, defaulting to x86_64");
            "x86_64-pc-windows-msvc".to_string()
        }
    } else if cfg!(target_os = "linux") {
        if cfg!(target_arch = "x86_64") {
            "x86_64-unknown-linux-gnu".to_string()
        } else if cfg!(target_arch = "aarch64") {
            "aarch64-unknown-linux-gnu".to_string()
        } else {
            eprintln!("Warning: Unsupported Linux architecture, defaulting to x86_64");
            "x86_64-unknown-linux-gnu".to_string()
        }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "x86_64") {
            "x86_64-apple-darwin".to_string()
        } else if cfg!(target_arch = "aarch64") {
            "aarch64-apple-darwin".to_string()
        } else {
            eprintln!("Warning: Unsupported macOS architecture, defaulting to aarch64");
            "aarch64-apple-darwin".to_string()
        }
    } else {
        eprintln!("Error: Unsupported operating system");
        exit(1);
    }
}

fn generate_sha256sum(binary_path: &Path, output_dir: &Path) -> Result<(), String> {
    // Calculate SHA256 hash of the binary
    let mut file = fs::File::open(binary_path)
        .map_err(|e| format!("Failed to open binary for hashing: {}", e))?;

    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 8192];

    loop {
        let bytes_read = file
            .read(&mut buffer)
            .map_err(|e| format!("Failed to read binary: {}", e))?;

        if bytes_read == 0 {
            break;
        }

        hasher.update(&buffer[..bytes_read]);
    }

    let hash = hasher.finalize();
    let hash_hex = format!("{:x}", hash);

    // Get just the filename (not the full path)
    let filename = binary_path
        .file_name()
        .ok_or("Invalid binary path")?
        .to_str()
        .ok_or("Invalid filename encoding")?;

    // Write SHA256SUM file in standard format: <hash>  <filename>
    let sha256sum_path = output_dir.join("SHA256SUM");
    let content = format!("{}  {}\n", hash_hex, filename);

    fs::write(&sha256sum_path, content).map_err(|e| format!("Failed to write SHA256SUM: {}", e))?;

    println!("Generated SHA256SUM: {}", sha256sum_path.display());
    Ok(())
}

fn command_exists(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--help")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn remove_dir_if_exists(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if p.exists() {
        fs::remove_dir_all(p).map_err(|e| format!("Failed to remove directory {}: {}", path, e))?;
        println!("Removed {}", path);
    }
    Ok(())
}

fn create_zip(source_dir: &Path, dest_path: &Path) -> Result<(), String> {
    // Remove existing zip if present
    if dest_path.exists() {
        fs::remove_file(dest_path).map_err(|e| format!("Failed to remove existing zip: {}", e))?;
    }

    // Create parent directory for zip if needed
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create zip parent directory: {}", e))?;
    }

    // Use PowerShell Compress-Archive on Windows, zip on Unix
    if cfg!(windows) {
        let source_pattern = source_dir.join("*");
        let status = Command::new("powershell")
            .args([
                "-Command",
                &format!(
                    "Compress-Archive -Path '{}' -DestinationPath '{}'",
                    source_pattern.display(),
                    dest_path.display()
                ),
            ])
            .status()
            .map_err(|e| format!("Failed to run Compress-Archive: {}", e))?;

        if !status.success() {
            return Err("Zip creation failed".to_string());
        }
    } else {
        // On Unix, use zip command
        let abs_dest = fs::canonicalize(dest_path.parent().unwrap())
            .unwrap()
            .join(dest_path.file_name().unwrap());

        let status = Command::new("zip")
            .args(["-r", abs_dest.to_str().unwrap(), "."])
            .current_dir(source_dir)
            .status() // ← THIS LINE
            .map_err(|e| format!("Failed to run zip: {}", e))?;

        if !status.success() {
            return Err("Zip creation failed".to_string());
        }
    }

    Ok(())
}

// ════════════════════════════════════════════════════════════════════════════
// DOCUMENTATION: the manual is generated from docs/ (DECISIONS.md D-09)
// ════════════════════════════════════════════════════════════════════════════

const MANUAL_DIR: &str = "docs/manual";
const MANUAL_OUT_DIR: &str = "target/manual";

/// Build the manual from the Markdown chapters listed in docs/manual/chapters.txt.
/// Output: target/manual/blentinel-manual.pdf (or .tex with `tex_only`).
fn build_manual(tex_only: bool) -> Result<(), String> {
    let chapters = read_manual_chapters(&Path::new(MANUAL_DIR).join("chapters.txt"))?;
    let pandoc = find_tool("pandoc", &[r"C:\Program Files\Pandoc\pandoc.exe"])
        .ok_or("pandoc not found. Install it from https://pandoc.org (and a LaTeX engine such as MiKTeX or TeX Live for PDF output).")?;

    fs::create_dir_all(MANUAL_OUT_DIR)
        .map_err(|e| format!("Failed to create {MANUAL_OUT_DIR}: {e}"))?;
    let output = Path::new(MANUAL_OUT_DIR).join(if tex_only {
        "blentinel-manual.tex"
    } else {
        "blentinel-manual.pdf"
    });
    let date = format!(
        "Version {} (generated {})",
        VERSION,
        chrono::Local::now().format("%Y-%m-%d")
    );

    let mut cmd = Command::new(&pandoc);
    cmd.arg("--metadata-file")
        .arg(Path::new(MANUAL_DIR).join("manual.yaml"))
        .arg("--metadata")
        .arg(format!("date={date}"))
        .arg("--include-in-header")
        .arg(Path::new(MANUAL_DIR).join("header.tex"))
        .args([
            "--number-sections",
            "--top-level-division=chapter",
            "--standalone",
        ])
        .arg("--output")
        .arg(&output);
    if !tex_only {
        let engine = find_tool(
            "xelatex",
            &[r"C:\Program Files\MiKTeX\miktex\bin\x64\xelatex.exe"],
        )
        .ok_or(
            "xelatex not found. Install MiKTeX or TeX Live, or use `blentinelmake docs --tex`.",
        )?;
        cmd.arg(format!("--pdf-engine={}", engine.display()));
    }
    cmd.args(&chapters);

    println!(
        "Building manual from {} chapters with {} ...",
        chapters.len(),
        pandoc.display()
    );
    let status = cmd
        .status()
        .map_err(|e| format!("Failed to run pandoc: {e}"))?;
    if !status.success() {
        return Err(format!("pandoc failed ({status})"));
    }
    println!("Manual written to {}", output.display());
    Ok(())
}

/// Chapter paths from `chapters.txt`: one per line, `#` comments and blank lines ignored.
/// Every listed file must exist, so a moved or renamed doc fails the build.
fn read_manual_chapters(list: &Path) -> Result<Vec<PathBuf>, String> {
    let text =
        fs::read_to_string(list).map_err(|e| format!("Failed to read {}: {e}", list.display()))?;
    let chapters: Vec<PathBuf> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(PathBuf::from)
        .collect();
    if let Some(missing) = chapters.iter().find(|p| !p.exists()) {
        return Err(format!(
            "{} lists a file that doesn't exist: {}",
            list.display(),
            missing.display()
        ));
    }
    if chapters.is_empty() {
        return Err(format!("{} lists no chapters", list.display()));
    }
    Ok(chapters)
}

/// A tool on PATH, or at one of the given well-known locations.
fn find_tool(name: &str, fallbacks: &[&str]) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(&exe))
        .chain(fallbacks.iter().map(PathBuf::from))
        .find(|p| p.is_file())
}
