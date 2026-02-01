Project Overview

    Blentinel is a high-performance, multi-tenant monitoring platform designed for Managed Service Providers (MSPs). It enables a central IT organization to monitor diverse client networks — including those behind NAT or private firewalls — using a secure push-based architecture.

Motivation

    Why “Blind” Sentinel?

    Traditional monitoring solutions typically operate within private networks and often require open inbound ports or VPN access, which increases attack surface and operational complexity.

    Blentinel uses a push-based model designed for remote and private networks.
    Probes initiate all connections outbound, eliminating the need for inbound firewall rules and significantly improving security posture.

System Architecture

The project is split into three core components:

    1. Sentinel Probe (Rust)

    A zero-dependency, low-footprint service that runs on client network devices.
    It polls local resources (NAS, printers, servers, database servers, etc.) and securely pushes encrypted health data to the Blentinel Hub.

    2. Blind Sentinel Hub (Rust)

    The central coordination service.
    A high-concurrency server responsible for:
        Authenticating probe identities
        Storing time-series health data in SQLite
        Exposing a type-safe API for clients

    3. Blind Sentinel Mobile App (TBA)

    A mobile client for technicians to view and interact with monitoring data.

Tech Stack

    Agent: Rust (Tokio, Rustls, Serde)
    Server: Rust 
    Database: SQLite (via SQLx)
    Frontend: Leptos
    Communication: mTLS (Mutual TLS) + AES-256-GCM Payload Encryption

A Note on Permissions (Linux/Windows)

Because we are using surge-ping for native ICMP:

    Linux: You may need to grant raw socket capability:
        sudo setcap cap_net_raw+ep belntinel_probe
        so the binary can open a raw socket without being root.

    Windows: Usually requires running the service as Administrator.
    
Installation Instructions

    Prerequisites

    Rust Toolchain:

    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    source "$HOME/.cargo/env"

    Leptos CLI:
    cargo install leptos-cli


Building from Source

    1. Clone the Repository

    git clone https://codeberg.org/mr_jimmybob/blentinel.git
    cd blentinel

    2.a Build the hub and the probe manually (choose 2.a or 2.b, not both)
    
        2.a.1 Build the Hub

        You can use the following command to build the hub, or use the available helper scripts:

        cargo leptos build --release

        # Binary will be at: target/release/blentinel_hub (Linux)
        # Binary will be at: target/release/blentinel_hub.exe (Windows)

        2.a.2. Build the Probe

        You can use the following command to build the probe, or use the available helper scripts:

        cargo build -p probe --release

        # Binary will be at: target/release/blentinel_probe (Linux)
        # Binary will be at: target/release/blentinel_probe.exe (Windows)

    2.b Use the publish script

        If you have successfully built the hub and probe, you can use the publish script to build the stripped release binaries.

        Run in powweshell:

        publish.ps1 -Target <triple>


Cross-Compilation

    Setup Cross-Compilation Targets
    If you want to cross compile, you will need a cross compilation toolchain.

    # Install cross-compilation toolchain for your platform, for example:
    rustup target add x86_64-unknown-linux-gnu
    rustup target add aarch64-unknown-linux-gnu
    rustup target add x86_64-pc-windows-msvc

    Common Targets:
        Platform	            Target
        Linux VPS	            x86_64-unknown-linux-gnu
        Raspberry Pi 4	        aarch64-unknown-linux-gnu
        Windows server	        x86_64-pc-windows-msvc


    Cross-Compile for Different Platforms (examples for common targets)

        For Linux x86_64:

            in the blentinel directory:
            cargo build -p probe --release --target x86_64-unknown-linux-gnu

        For Raspberry Pi (ARM64):

            in the blentinel directory:
            cargo build -p probe --release --target aarch64-unknown-linux-gnu
 
        For Windows:

            in the blentinel directory:
            cargo build -p probe --release --target x86_64-pc-windows-msvc

The Blentinel Directory Structure

    blentinel/
    ├── .gitignore
    ├── Cargo.toml                      # Workspace configuration
    ├── README.md                           # Project vision and spec
    │
    ├── probe/                          # PROBE (client service)
    │   ├──  blentinel_probe.toml      # Probe run-time configuration file
    │   ├──  Cargo.toml                # Probe Rust configuration file
    │   └──  src
    │       ├──  args.rs               # Command line arguments processing
    │       ├──  config.rs             # Configuration parsing
    │       ├──  crypto.rs             # Encryption and signature verification (Ed25519 & ChaCha20) logic
    │       ├──  error.rs              # Error handling
    │       ├──  identity.rs           # Identity management
    │       ├──  main.rs               # Entry point
    │       ├──  monitor.rs            # Monitor and polling logic (ICMP, TCP, HTTP)
    │       ├──  storage.rs            # Data storage (SQLite)
    │       └──  transport.rs          # Secure push logic to the hub
    │
    ├── hub/                            # THE SERVER (Rust + Axum)
    │    ├──  blentinel_hub.toml
    │    ├──  Cargo.toml               # Hub Rust configuration file
    │    ├──  end2end                  # Leptos 
    │    │   ├──  package-lock.json    # Hub run-time configuration file
    │    │   ├──  package.json         # End-to-end tests configuration file   
    │    │   ├──  playwright.config.ts # Playwright configuration file
    │    │   ├──  tests
    │    │   │   └──  example.spec.ts
    │    │   └──  tsconfig.json        # Typescript configuration file
    │    ├──  LICENSE
    │    ├──  public
    │    │   └──  favicon.ico
    │    ├──  README.md
    │    ├──  src
    │    │   ├──  api.rs               # API endpoint callbacks
    │    │   ├──  app.rs               
    │    │   ├──  args.rs              # Command line arguments processing for the app
    │    │   ├──  auth.rs              # Authentication and authorization
    │    │   ├──  config.rs            # Configuration parsing
    │    │   ├──  crypto.rs            # Decryption and signature verification (Ed25519 & ChaCha20) logic
    │    │   ├──  db.rs                # Data storage (SQLite)
    │    │   ├──  identity.rs          # Identity management
    │    │   ├──  lib.rs               # WebAssembly hydration to initialize the Leptos frontend application.
    │    │   └──  main.rs              # Entry point
    │    └──  style
    │        └──  main.scss
    │
    └── common/                         # SHARED LOGIC (The "Glue")
        ├── Cargo.toml                  # Common Rust configuration file
        └── src/
            ├── lib.rs
            └── models.rs               # Shared Structs

