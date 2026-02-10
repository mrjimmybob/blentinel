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
    Communication: Optional TLS/HTTPS + X25519/ChaCha20-Poly1305 Payload Encryption + Ed25519 Signatures

Security Features

    ✅ Multi-layer security (defense in depth)
        - Transport Layer: Optional TLS/HTTPS with certificate pinning
        - Application Layer: X25519 key exchange + ChaCha20-Poly1305 encryption
        - Authentication: Ed25519 digital signatures
    ✅ Zero-trust certificate pinning (probes only trust specific hub certificate)
    ✅ Auto-generated self-signed certificates (no manual PKI required)
    ✅ Probe whitelist (hub rejects unknown probes)
    ✅ Push-based architecture (no inbound firewall rules needed)

HTTPS Configuration (Optional)

    Blentinel supports optional HTTPS with certificate pinning for enhanced transport security.
    By default, the system uses HTTP with application-layer encryption (ChaCha20-Poly1305).

    Quick Start: Enable HTTPS

        1. Enable TLS on Hub

           Edit blentinel_hub.toml:

           [server.tls]
           enabled = true
           cert_path = "hub_tls_cert.pem"
           key_path = "hub_tls_key.pem"
           https_port = 3443  # Optional: run HTTP and HTTPS simultaneously

           Restart the hub. Certificates will be auto-generated on first run:
           - hub_tls_cert.pem (public certificate - share with probes)
           - hub_tls_key.pem (private key - keep secure!)

        2. Update Probes to Use HTTPS

           a. Copy the hub certificate to your build environment:

              cp hub_tls_cert.pem probe/hub_cert.pem

           b. Rebuild the probe (certificate is embedded at compile time):

              .\build_probe.ps1 -Release
              # Or for Linux:
              .\build_probe.ps1 -Release -Target x86_64-unknown-linux-gnu

           c. Update probe configuration (blentinel_probe.toml):

              [agent]
              hub_url = "https://HUB_IP:3443"  # Changed from http:// to https://

           d. Deploy and restart the probe

    Operating Modes

        HTTP-only (default):
            [server.tls] section disabled or commented out
            Probes use hub_url = "http://..."

        HTTPS-only:
            [server.tls]
            enabled = true
            # No https_port specified - HTTPS replaces HTTP on main port

            Probes use hub_url = "https://..."

        Dual-mode (recommended for migration):
            [server.tls]
            enabled = true
            https_port = 3443  # HTTPS on 3443, HTTP still on main port

            Allows gradual probe migration from HTTP to HTTPS

    Certificate Pinning Security

        Probes embed the hub certificate at compile time and ONLY trust that certificate.
        This provides stronger security than traditional HTTPS with CA validation:

        ✅ Prevents MITM attacks even if attacker has valid CA certificates
        ✅ No dependency on system certificate stores
        ✅ Perfect for private networks and air-gapped environments

        ⚠️  Certificate changes require probe rebuild (by design for security)

    Documentation

        HTTPS_SETUP_GUIDE.md    - Complete setup guide with troubleshooting
        HTTPS_IMPLEMENTATION.md - Technical details and architecture

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

    2.a Use the bulid tool

        cargo build -p blentinelmake --release

        Then run the binary in the target directory:
            ./target/release/blentinelmake

            If run without arguments it asks you what you want to do, interactive.

        Otherwise, you can run it with arguments, see help for usage:

            ./target/debugrelease/blentinelmake.exe --help # for more ways to run the script

    2.b Build the hub and the probe manually (choose 2.a or 2.b, not both)
    
        2.b.1 Build the Hub

        You can use the following command to build the hub, or use the available helper scripts:

        cargo leptos build --release

        # Binary will be at: target/release/blentinel_hub (Linux)
        # Binary will be at: target/release/blentinel_hub.exe (Windows)

        2.b.2. Build the Probe

        You can use the following command to build the probe, or use the available helper scripts:

        cargo build -p probe --release

        # Binary will be at: target/release/blentinel_probe (Linux)
        # Binary will be at: target/release/blentinel_probe.exe (Windows)


Cross-Compilation

    You cannot cross compile the Hub due to it using leptos.

    To cross compile the probe you will to setup Cross-Compilation Targets and install a cross compilation toolchain.

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

Running PROBE as a service

    Linux service on systemd

        Create file on target machine:

            /etc/systemd/system/blentinel-probe.service

                [Unit]
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

        Install:

            sudo useradd -r blentinel
            sudo mkdir -p /opt/blentinel/probe
            sudo cp probe /opt/blentinel/probe/
            sudo chmod +x /opt/blentinel/probe/probe
            sudo chown -R blentinel:blentinel /opt/blentinel

            sudo cp blentinel-probe.service /etc/systemd/system/

            sudo systemctl daemon-reload
            sudo systemctl enable blentinel-probe
            sudo systemctl start blentinel-probe


        Logs:

            journalctl -u blentinel-probe -f

    Windows Service (PowerShell)

        Create install_probe_service.ps1:

            $serviceName = "BlentinelProbe"
            $exePath = "C:\Blentinel\probe\probe.exe"

            New-Item -ItemType Directory -Force -Path "C:\Blentinel\probe" | Out-Null
            Copy-Item ".\probe.exe" $exePath -Force

            sc.exe create $serviceName binPath= "`"$exePath`"" start= auto
            sc.exe description $serviceName "Blentinel network monitoring probe"

            Start-Service $serviceName

        Uninstall:

            Stop-Service BlentinelProbe
            sc.exe delete BlentinelProbe

        Logs:

            Windows Event Viewer

## Hot Reload Configuration

Both the probe and hub support **hot reloading** of configuration files. When you modify a config file, the application automatically detects the change and reloads the configuration without requiring a restart.

### How It Works

- **File watching**: Both applications monitor their config files (`blentinel_probe.toml` and `blentinel_hub.toml`) for changes
- **Debouncing**: Changes are debounced with a 500ms delay to handle text editors that save files in multiple chunks
- **Validation**: New configs are fully validated before being applied. If validation fails, the old config is kept and an error is logged
- **Thread-safe**: Configurations are accessed through `Arc<RwLock>` allowing concurrent reads during updates

### Probe - Fully Hot-Reloadable

All probe configuration settings can be hot-reloaded:

- `agent.hub_url` - Hub endpoint URL
- `agent.company_id` - Company identifier
- `agent.interval` - Monitoring interval in seconds
- `agent.hub_public_key` - Hub's public key (optional)
- `[[resources]]` - Entire resource list (add/remove/modify resources)

**Example workflow:**
1. Edit `blentinel_probe.toml` and change the interval from 30 to 60 seconds
2. Save the file
3. Console output: `✓ Configuration reloaded successfully`
4. Next monitoring cycle uses the new 60-second interval

### Hub - Partially Hot-Reloadable

**Hot-reloadable settings** (take effect immediately):
- `[[probes]]` - Probe whitelist (add/remove probes)
- `server.probe_timeout_secs` - Probe expiry timeout

**Restart-required settings** (logged as warnings):
- `server.host` - Bind IP address
- `server.port` - Bind port
- `server.db_path` - Database file location
- `server.identity_key_path` - Hub identity key file
- `server.auth_token_path` - Admin auth token file

When you modify a restart-required setting, the hub will log a warning like:
```
⚠ Restart-required changes detected:
  ⚠ server.port changed from 3000 to 3001 - requires restart
  Please restart the hub for these changes to take effect.
```

The hot-reloadable fields in the same edit will still take effect immediately.

**Example: Adding a new probe to the whitelist**
1. Edit `blentinel_hub.toml` and add a new `[[probes]]` entry
2. Save the file
3. Console output: `✓ Configuration reloaded successfully` with `+ 1 probe(s) added to whitelist`
4. New probe can immediately send reports (no hub restart needed)

### Error Handling

If a config file has errors (syntax errors, validation failures), the application:
- Logs the error clearly
- Keeps the previous working configuration
- Continues operating normally
- Will attempt to reload again on the next file change

**Example error output:**
```
[Hot Reload] Config file changed, attempting reload...
✗ Failed to reload config: Validation error: agent.interval must be greater than 0
  Keeping previous configuration
```

### Testing Hot Reload

See [HOT_RELOAD_TESTING.md](HOT_RELOAD_TESTING.md) for comprehensive test cases and verification steps.

### Performance Notes

- Config reads use `RwLock` allowing unlimited concurrent readers
- Write locks are held only during the actual config swap (typically < 1ms)
- File watching runs in a background task and doesn't block normal operations
- The file watcher automatically recovers if it encounters errors

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

    New files for HTTPS support:
        probe/build.rs                  # Build-time certificate verification
        probe/hub_cert.pem              # Hub TLS certificate (embedded at build)
        probe/src/hot_reload.rs         # Configuration hot reloading
        probe/src/tls.rs                # Certificate embedding and validation
        hub/src/hot_reload.rs           # Configuration hot reloading
        hub/src/tls.rs                  # Certificate generation and TLS config

    Generated files (auto-created at runtime):
        hub_tls_cert.pem                # Hub TLS certificate (public)
        hub_tls_key.pem                 # Hub TLS private key (keep secure!)
        hub_identity.key                # Hub X25519 key
        hub_auth.token                  # Admin authentication token

