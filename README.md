Project Overview

Blentinel is a high-performance, multi-tenant monitoring platform designed for MSPs (Managed Service Providers). It allows a central IT company to monitor diverse client networks—even those behind NAT or private firewalls—using a secure "Push" architecture.

System Architecture

The project is split into three core components:

    Sentinel Probe (Rust): A zero-dependency, low-footprint binary running on client network devices as a service. It polls local resources (NAS, Printers, Servers, DB Servers, ...) and pushes encrypted health data to the Blind Sentinel Hub.

    Blind Sentinel Hub (Rust): The central nervous system. A high-concurrency server that validates agent identities, stores time-series data in SQLite, and exposes a type-safe API.

    Blind Sentinel mobile app (tba): A mobile client for technicians to view and interact with the data.

Motivation

    Why "Blind" Sentinel? Traditional monitoring solutions mainly operate on a private network and often require open inbound ports or VPNs, which can be security risks. Blentinel's push architecture is designed to work with remote private networks and ensures that no inbound connections are needed, enhancing security.

Tech Stack

    Agent: Rust (Tokio, Rustls, Serde)

    Server: Rust 

    Database: SQLite (via SQLx)

    Frontend: Leptos

    Communication: mTLS (Mutual TLS) + AES-256-GCM Payload Encryption

A Note on Permissions (Linux/Windows)

Because we are using surge-ping for native ICMP:

    Linux: You may need to run sudo setcap cap_net_raw+ep ./target/debug/probe so the binary can open a raw socket without being root.

    Windows: Usually requires running the terminal as Administrator.
    
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
