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
  -h, --help           Print this help message
  --version            Print version

Examples:
  blentinelmake probe build
  blentinelmake probe build --release
  blentinelmake probe publish --target x86_64-unknown-linux-musl
  blentinelmake hub publish
  blentinelmake probe clean --target x86_64-unknown-linux-musl

Documentation:
  blentinelmake docs           Build the PDF manual from docs/ (needs pandoc + xelatex)
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

    // Publish always uses release mode
    if action == Action::Publish {
        release = true;
    }

    // Execute
    if let Err(e) = run(component, action, release, target) {
        eprintln!("\nError: {}", e);
        exit(1);
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

    run(component, action, release, target)
}

fn run(
    component: Component,
    action: Action,
    release: bool,
    target: Option<String>,
) -> Result<(), String> {
    match component {
        Component::Hub => match action {
            Action::Build => hub_build(release),
            Action::Publish => hub_publish(),
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

fn hub_build(release: bool) -> Result<(), String> {
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

    let mut args = vec!["leptos", "build"];
    if release {
        args.push("--release");
    }

    let status = Command::new("cargo")
        .current_dir("hub")
        .args(&args)
        .status()
        .map_err(|e| format!("Failed to run cargo leptos build: {}", e))?;

    if !status.success() {
        return Err("Hub build failed".to_string());
    }

    println!("Hub build completed successfully");
    Ok(())
}

fn hub_publish() -> Result<(), String> {
    use chrono::Local;
    let timestamp = Local::now().format("hub-%Y%m%d_%H%M%S").to_string();
    let publish_root = Path::new("publish").join(&timestamp);
    let app_dir = publish_root.join("app");

    println!("Publishing hub...");

    // Build in release mode
    hub_build(true)?;

    // Create output directory
    fs::create_dir_all(&app_dir)
        .map_err(|e| format!("Failed to create output directory: {}", e))?;

    // Copy binary (note: cargo-leptos in workspace mode outputs to workspace target/)
    let exe_name = if cfg!(windows) { "hub.exe" } else { "hub" };
    let src = Path::new("target/release").join(exe_name);
    let dst = app_dir.join(exe_name);

    fs::copy(&src, &dst)
        .map_err(|e| format!("Failed to copy hub binary from {}: {}", src.display(), e))?;

    // Copy the pkg directory (WASM, CSS, JS) - this must be in the same dir as the executable
    let pkg_src = Path::new("target/site/pkg");
    let pkg_dst = app_dir.join("pkg");

    if pkg_src.exists() {
        copy_dir_recursive(pkg_src, &pkg_dst)
            .map_err(|e| format!("Failed to copy pkg directory: {}", e))?;
    } else {
        return Err("pkg directory not found. Did cargo leptos build succeed?".to_string());
    }

    // Copy favicon if it exists
    let favicon_src = Path::new("target/site/favicon.ico");
    if favicon_src.exists() {
        fs::copy(favicon_src, app_dir.join("favicon.ico"))
            .map_err(|e| format!("Failed to copy favicon: {}", e))?;
    }

    // Generate SHA256 checksum
    generate_sha256sum(&dst, &app_dir)?;

    // Generate config file
    generate_hub_config(&app_dir)?;

    // Generate service files
    generate_hub_service_files(&app_dir)?;

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

    let status = Command::new("cargo")
        .args(&final_args)
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

    // Build in release mode
    probe_build(true, Some(&target))?;

    // Create output directory
    fs::create_dir_all(&app_dir)
        .map_err(|e| format!("Failed to create output directory: {}", e))?;

    // Determine binary name and path
    let exe_name = if target.contains("windows") {
        "probe.exe"
    } else {
        "probe"
    };

    let src = PathBuf::from("target")
        .join(&target)
        .join("release")
        .join(exe_name);
    let dst = app_dir.join(exe_name);

    fs::copy(&src, &dst).map_err(|e| format!("Failed to copy probe binary: {}", e))?;

    // Strip binary on non-Windows targets
    if !target.contains("windows") {
        strip_binary(&dst);
    }

    // Generate SHA256 checksum
    generate_sha256sum(&dst, &app_dir)?;

    // Generate config file
    generate_probe_config(&app_dir)?;

    // Generate service files
    generate_probe_service_files(&app_dir, &target)?;

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

fn generate_hub_config(app_dir: &Path) -> Result<(), String> {
    let config_file = app_dir.join("blentinel_hub.toml");
    let content = r#"# ---------------------------------------------------------------------------
# Blentinel Hub Configuration
# ---------------------------------------------------------------------------

[server]
# Address and port the hub listens on
host = "127.0.0.1"
port = 3000

# Directory for all runtime state (database, key, setup code).
# Relative paths below are resolved relative to state_dir.
# Use an absolute path in production (e.g. /var/lib/blentinel).
state_dir = "."

# SQLite database file (relative to state_dir)
db_path = "blentinel.db"

# Path to the persistent X25519 private key used for ECDH with probes.
# Generated automatically on first run if it does not exist.
identity_key_path = "hub_identity.key"

# Seconds of silence before a probe is marked expired.
# Should be at least 2-3× the probe's reporting interval.
probe_timeout_secs = 120

# ---------------------------------------------------------------------------
# Retention & Archival
# ---------------------------------------------------------------------------
# Configure automatic database archiving to manage storage and maintain
# historical data for compliance or forensic analysis.

[retention]
# Enable retention monitoring and archiving features
enabled = true
# Automatically archive old data when database size exceeds warn_db_size_mb
# Set to false to require manual archiving via admin panel
auto = false
# Data older than this many days will be moved to archive databases
archive_older_than_days = 90
# Warn when database exceeds this size in megabytes
warn_db_size_mb = 1000
# Directory where archive databases are stored (relative to working directory)
archive_path = "archives"

# ---------------------------------------------------------------------------
# Alerting System
# ---------------------------------------------------------------------------
# Configure email alerts for resource failures, recoveries, thresholds,
# and probe expiry events.

[alerts]
# Enable or disable the alerting system
enabled = false
# Default email recipients for all alerts (unless overridden per company)
default_recipients = []

# Technician contact list (for reference and future features)
[[alerts.technicians]]
name = "John Doe"
email = "john@example.com"
phone = "+1-555-0100"

[[alerts.technicians]]
name = "Jane Smith"
email = "jane@example.com"
phone = "+1-555-0101"

# Global threshold settings for local system metrics
[alerts.thresholds]
disk_percent = 90  # Alert when disk usage exceeds this percentage
cpu_percent = 95   # Alert when CPU usage exceeds this percentage
mem_percent = 90   # Alert when memory usage exceeds this percentage

# SMTP configuration for sending email alerts
[alerts.smtp]
server = "smtp.example.com"
port = 587
username = "alerts@example.com"
password = "your-smtp-password"
from = "blentinel@example.com"

# Per-company alert overrides (optional)
# Override alert recipients and thresholds for specific companies
#
# [alerts.company_overrides."CompanyA"]
# alert_emails = ["companya-it@example.com"]
#
# [alerts.company_overrides."CompanyA".thresholds]
# disk_percent = 85
# cpu_percent = 90
# mem_percent = 85

# ---------------------------------------------------------------------------
# HTTPS (on by default, D-43)
# ---------------------------------------------------------------------------
# The web UI and API are served over HTTPS on https_port, with a self-signed
# certificate created on first start. Probes enrolled with a code pin that
# certificate (its fingerprint is in the code). The plain-HTTP `port` serves
# only the probe endpoints (reports are signed and encrypted anyway) and
# redirects everything else to HTTPS.
#
#   https_port = port      HTTPS only, on `port`
#   enabled = false        plain HTTP for everything (behind a TLS proxy:
#                          also set behind_proxy = true under [server])

[server.tls]
enabled = true
cert_path = "hub_tls_cert.pem"
key_path = "hub_tls_key.pem"
https_port = 3443
"#;

    fs::write(&config_file, content).map_err(|e| format!("Failed to write hub config: {}", e))?;

    Ok(())
}

fn generate_probe_config(app_dir: &Path) -> Result<(), String> {
    let config_file = app_dir.join("blentinel_probe.toml");
    let content = r#"# ==================================
# Blentinel Probe Configuration File
# ==================================
# The hub, company and site come from enrollment: get a code from the hub's
# "Add probe" and run `probe enroll <code>` (writes enrollment.json).
# Here: the reporting interval and the [[resources]] to check.

[agent]
interval = 60


# ---- Ping (ICMP) ----
[[resources]]
name = "Router"
type = "ping"
target = "192.168.1.1"


# ---- HTTP ----
[[resources]]
name = "Company Website"
type = "http"
target = "https://example.com"


# ---- TCP Port ----
[[resources]]
name = "Database Server"
type = "tcp"
target = "192.168.1.50:5432"
"#;

    fs::write(&config_file, content).map_err(|e| format!("Failed to write probe config: {}", e))?;

    Ok(())
}

// ============================================================================
// SERVICE FILE GENERATION
// ============================================================================

// Installer templates live in blentinelmake/templates/ as real scripts, so they
// can be read, syntax-checked and reviewed as scripts (D-34).
const HUB_SYSTEMD_UNIT: &str = include_str!("../templates/hub/blentinel-hub.service");
const HUB_LINUX_INSTALLER: &str = include_str!("../templates/hub/install_hub_service.sh");
const HUB_WINDOWS_INSTALLER: &str = include_str!("../templates/hub/install_hub_service.ps1");
const PROBE_SYSTEMD_UNIT: &str = include_str!("../templates/probe/blentinel-probe.service");
const PROBE_LINUX_INSTALLER: &str = include_str!("../templates/probe/install_probe_service.sh");
const PROBE_WINDOWS_INSTALLER: &str = include_str!("../templates/probe/install_probe_service.ps1");

/// Write a Unix text file (shell script or systemd unit) with LF line endings,
/// whatever the checkout's line endings are: bash can't run a CRLF script.
fn write_unix_file(path: &Path, content: &str, executable: bool) -> Result<(), String> {
    fs::write(path, content.replace("\r\n", "\n"))
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to make {} executable: {}", path.display(), e))?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    Ok(())
}

fn write_windows_file(path: &Path, content: &str) -> Result<(), String> {
    fs::write(path, content).map_err(|e| format!("Failed to write {}: {}", path.display(), e))
}

fn generate_hub_service_files(app_dir: &Path) -> Result<(), String> {
    // The hub is built natively, so the build platform is the deployment platform.
    if cfg!(windows) {
        write_windows_file(
            &app_dir.join("install_hub_service.ps1"),
            HUB_WINDOWS_INSTALLER,
        )
    } else {
        write_unix_file(
            &app_dir.join("blentinel-hub.service"),
            HUB_SYSTEMD_UNIT,
            false,
        )?;
        write_unix_file(
            &app_dir.join("install_hub_service.sh"),
            HUB_LINUX_INSTALLER,
            true,
        )
    }
}

fn generate_probe_service_files(app_dir: &Path, target: &str) -> Result<(), String> {
    // Installers for the probe's deployment target (it can be cross-compiled).
    if target.contains("windows") {
        write_windows_file(
            &app_dir.join("install_probe_service.ps1"),
            PROBE_WINDOWS_INSTALLER,
        )
    } else {
        write_unix_file(
            &app_dir.join("blentinel-probe.service"),
            PROBE_SYSTEMD_UNIT,
            false,
        )?;
        write_unix_file(
            &app_dir.join("install_probe_service.sh"),
            PROBE_LINUX_INSTALLER,
            true,
        )
    }
}

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

fn strip_binary(path: &Path) {
    // Try to strip the binary; log warning if it fails but don't error
    let status = Command::new("strip").arg(path).status();

    match status {
        Ok(s) if s.success() => {
            println!("Stripped binary: {}", path.display());
        }
        Ok(_) => {
            println!("Warning: strip command failed for {}", path.display());
        }
        Err(_) => {
            println!("Warning: strip command not available, skipping binary stripping");
        }
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

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst)
        .map_err(|e| format!("Failed to create directory {}: {}", dst.display(), e))?;

    for entry in fs::read_dir(src)
        .map_err(|e| format!("Failed to read directory {}: {}", src.display(), e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
        let path = entry.path();
        let file_name = entry.file_name();
        let dest_path = dst.join(&file_name);

        if path.is_dir() {
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            fs::copy(&path, &dest_path).map_err(|e| {
                format!(
                    "Failed to copy {} to {}: {}",
                    path.display(),
                    dest_path.display(),
                    e
                )
            })?;
        }
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

#[cfg(test)]
mod installer_tests {
    //! Regression tests for the service installers (DECISIONS.md D-34).
    use super::*;

    /// `Key=Value` lines of a systemd unit (sections and comments skipped).
    fn unit_settings(unit: &str) -> Vec<&str> {
        unit.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('['))
            .collect()
    }

    #[test]
    fn unit_files_have_no_inline_comments() {
        // systemd has no inline comments: `User=blentinel # x` made the user "blentinel # x".
        for unit in [HUB_SYSTEMD_UNIT, PROBE_SYSTEMD_UNIT] {
            for line in unit_settings(unit) {
                assert!(!line.contains('#'), "inline comment in unit line: {line}");
            }
        }
    }

    #[test]
    fn hub_unit_passes_a_config_file_the_argument_parser_accepts() {
        // `--config=/var/lib/blentinel` was rejected by the parser (and was a directory).
        let exec = unit_settings(HUB_SYSTEMD_UNIT)
            .into_iter()
            .find(|l| l.starts_with("ExecStart="))
            .unwrap();
        assert!(
            exec.contains("--config /etc/blentinel/blentinel_hub.toml"),
            "{exec}"
        );
        assert!(!exec.contains("--config="), "{exec}");
    }

    #[test]
    fn installers_create_the_service_user() {
        for script in [HUB_LINUX_INSTALLER, PROBE_LINUX_INSTALLER] {
            assert!(
                script.contains("useradd --system"),
                "service user is not created"
            );
        }
    }

    #[test]
    fn installers_copy_only_known_files() {
        // `cp *` / `rsync .` shipped runtime files (keys, database) left in the
        // package folder; `rsync --delete` wiped the probe's identity on upgrade.
        for script in [HUB_LINUX_INSTALLER, PROBE_LINUX_INSTALLER] {
            assert!(
                !script.contains("rsync"),
                "installer must not sync whole folders"
            );
            assert!(
                !script.contains("cp *"),
                "installer must not copy everything"
            );
        }
    }

    #[test]
    fn probe_upgrade_never_touches_its_identity_or_config() {
        let acts_on_identity = PROBE_LINUX_INSTALLER
            .lines()
            .any(|l| l.contains("identity.key") && !l.trim_start().starts_with('#'));
        assert!(
            !acts_on_identity,
            "the installer must never touch identity.key"
        );
        assert!(
            PROBE_LINUX_INSTALLER.contains("if [ ! -f \"$INSTALL_DIR/blentinel_probe.toml\" ]"),
            "the config must only be written on first install"
        );
    }

    #[test]
    fn probe_can_ping_without_root_across_upgrades() {
        // setcap is lost when the binary is replaced; the unit grant survives upgrades.
        assert!(unit_settings(PROBE_SYSTEMD_UNIT).contains(&"AmbientCapabilities=CAP_NET_RAW"));
    }

    #[test]
    fn unix_files_are_written_with_lf_line_endings() {
        let dir = std::env::temp_dir().join(format!("blentinelmake-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("script.sh");

        write_unix_file(&path, "#!/bin/bash\r\necho hi\r\n", true).unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"#!/bin/bash\necho hi\n");
        let _ = fs::remove_dir_all(&dir);
    }
}
