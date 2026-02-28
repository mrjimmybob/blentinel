use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};

use dialoguer::{theme::ColorfulTheme, Confirm, Select};
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
    let check = Command::new("cargo").args(&["leptos", "--help"]).output();

    if check.is_err() || !check.unwrap().status.success() {
        println!("cargo-leptos not found, installing...");
        let status = Command::new("cargo")
            .args(&["install", "cargo-leptos"])
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
        .args(&["clean"])
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
    let use_zigbuild = target.map_or(false, |t| t.contains("linux"));
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

    // Copy TLS certificate if it exists
    let hub_cert = Path::new("probe/hub_cert.pem");
    if hub_cert.exists() {
        fs::copy(hub_cert, app_dir.join("hub_cert.pem"))
            .map_err(|e| format!("Failed to copy hub_cert.pem: {}", e))?;
        println!("Included hub TLS certificate for HTTPS support");
    } else {
        println!("Warning: No hub_cert.pem found. Probe will only support HTTP.");
    }

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

# Directory for all runtime state (database, keys, tokens).
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
# TLS/HTTPS Configuration (Optional)
# ---------------------------------------------------------------------------
# Uncomment to enable HTTPS. Certificate auto-generated on first run.
# Copy hub_tls_cert.pem to probe/hub_cert.pem for certificate pinning.
#
# [server.tls]
# enabled = false
# cert_path = "hub_tls_cert.pem"
# key_path = "hub_tls_key.pem"
# https_port = 3443  # Optional: run HTTP and HTTPS on different ports


# ---------------------------------------------------------------------------
# Authorized Probes
# ---------------------------------------------------------------------------
# Each [[probes]] entry registers a probe the hub will accept reports from.
# public_key is the hex-encoded Ed25519 public key printed by the probe on
# its very first run.  A probe whose ID is not listed here will be rejected.
#
# Example:
#   [[probes]]
#   name       = "Office-PC-1"
#   public_key = "23729607766e0ddce6d88f7221e37e651c45c040cff8889d022d41b863fbc4d5"
"#;

    fs::write(&config_file, content).map_err(|e| format!("Failed to write hub config: {}", e))?;

    Ok(())
}

fn generate_probe_config(app_dir: &Path) -> Result<(), String> {
    let config_file = app_dir.join("blentinel_probe.toml");
    let content = r#"# ==================================
# Blentinel Probe Configuration File
# ==================================
# 1. Set your company_id
# 2. Set your hub_url (http://HUB_IP:PORT)
# 3. Adjust interval if needed
# 4. Add or remove [[resources]] blocks

[agent]
company_id = "COMPANY_NAME"
hub_url = "http://HUB_ADDRESS:PORT"
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

fn generate_hub_service_files(app_dir: &Path) -> Result<(), String> {
    // systemd service
    let systemd_content = r#"[Unit]
Description=Blentinel Hub
After=network.target

[Service]
Type=simple
ExecStart=/opt/blentinel/hub/hub
Restart=always
RestartSec=5
User=blentinel
WorkingDirectory=/opt/blentinel/hub

[Install]
WantedBy=multi-user.target
"#;
    fs::write(app_dir.join("blentinel-hub.service"), systemd_content)
        .map_err(|e| format!("Failed to write systemd service: {}", e))?;

    // Windows installer
    let win_installer = r#"$serviceName = "BlentinelHub"
$installDir = "C:\Blentinel\hub"
$exeName = "hub.exe"
$exePath = Join-Path $installDir $exeName

Write-Host "Installing Blentinel Hub service..." -ForegroundColor Green

New-Item -ItemType Directory -Force -Path $installDir | Out-Null
Copy-Item ".\$exeName" $exePath -Force
Copy-Item ".\blentinel_hub.toml" $installDir -Force

sc.exe create $serviceName binPath= "`"$exePath`"" start= auto
sc.exe description $serviceName "Blentinel central monitoring hub"

Start-Service $serviceName

Write-Host "Service installed and started." -ForegroundColor Cyan
"#;
    fs::write(app_dir.join("install_hub_service.ps1"), win_installer)
        .map_err(|e| format!("Failed to write Windows installer: {}", e))?;

    // Linux installer
    let linux_installer = r#"#!/bin/bash
sudo mkdir -p /opt/blentinel/hub
sudo rsync -av . /opt/blentinel/hub
sudo cp blentinel-hub.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable blentinel-hub
sudo systemctl start blentinel-hub
"#;
    fs::write(app_dir.join("install_hub_service.sh"), linux_installer)
        .map_err(|e| format!("Failed to write Linux installer: {}", e))?;

    Ok(())
}

fn generate_probe_service_files(app_dir: &Path, target: &str) -> Result<(), String> {
    // systemd service
    let systemd_content = r#"[Unit]
Description=Blentinel Probe
After=network.target

[Service]
Type=simple
ExecStart=/opt/blentinel/probe/probe
Restart=always
RestartSec=5
User=blentinel
WorkingDirectory=/opt/blentinel/probe

[Install]
WantedBy=multi-user.target
"#;
    fs::write(app_dir.join("blentinel-probe.service"), systemd_content)
        .map_err(|e| format!("Failed to write systemd service: {}", e))?;

    // Windows installer (only for Windows targets)
    if target.contains("windows") {
        let win_installer = r#"$serviceName = "BlentinelProbe"
$installDir = "C:\Blentinel\probe"
$exeName = "probe.exe"
$exePath = Join-Path $installDir $exeName

Write-Host "Installing Blentinel Probe service..." -ForegroundColor Green

New-Item -ItemType Directory -Force -Path $installDir | Out-Null
Copy-Item ".\$exeName" $exePath -Force

sc.exe create $serviceName binPath= "`"$exePath`"" start= auto
sc.exe description $serviceName "Blentinel network monitoring probe"

Start-Service $serviceName

Write-Host "Service installed and started." -ForegroundColor Cyan
"#;
        fs::write(app_dir.join("install_probe_service.ps1"), win_installer)
            .map_err(|e| format!("Failed to write Windows installer: {}", e))?;
    }

    Ok(())
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
            .args(&[
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
            .args(&["-r", abs_dest.to_str().unwrap(), "."])
            .current_dir(source_dir)
            .status() // ← THIS LINE
            .map_err(|e| format!("Failed to run zip: {}", e))?;

        if !status.success() {
            return Err("Zip creation failed".to_string());
        }
    }

    Ok(())
}
