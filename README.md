Project Overview

    Blentinel is a high-performance, multi-tenant monitoring platform designed for Managed Service Providers (MSPs). It enables a central IT organization to monitor diverse client networks — including those behind NAT or private firewalls — using a secure push-based architecture.

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

Motivation

    Why “Blind” Sentinel?

    Traditional monitoring solutions typically operate within private networks and often require open inbound ports or VPN access, which increases attack surface and operational complexity.

    Blentinel uses a push-based model designed for remote and private networks.
    Probes initiate all connections outbound, eliminating the need for inbound firewall rules and significantly improving security posture.

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
    
The Blentinel Directory Structure

    blentinel/
    ├── .gitignore
    ├── Cargo.toml                # Workspace configuration
    ├── README.md                 # Project vision and spec
    ├── probe/                    # THE AGENT (Rust)
    │   ├── Cargo.toml
    │   ├── src/
    │   │   ├── main.rs           # Entry point
    │   │   ├── collector/        # Polling logic (ICMP, TCP, HTTP)
    │   │   ├── crypto/           # Ed25519 & ChaCha20 logic
    │   │   └── transport/        # Secure push logic
    │   └── config.toml           # Local-only probe config
    ├── hub/                      # THE SERVER (Rust + Axum)
    │   ├── Cargo.toml
    │   ├── src/
    │   │   ├── main.rs           # Entry point
    │   │   ├── handlers/         # API & Web routes
    │   │   ├── db/               # SQLite/Postgres logic
    │   │   └── templates/        # HTML/Dashboard logic
    │   └── hub_config.toml
    └── common/                   # SHARED LOGIC (The "Glue")
        ├── Cargo.toml
        └── src/
            ├── lib.rs
            ├── models.rs         # Shared Structs (StatusReport, etc.)
            └── protocol.rs       # Shared constants and versioning
