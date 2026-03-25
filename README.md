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

    - Multi-layer security (defense in depth)
        - Transport Layer: Optional TLS/HTTPS with certificate pinning
        - Application Layer: X25519 key exchange + ChaCha20-Poly1305 encryption
        - Authentication: Ed25519 digital signatures
    - Zero-trust certificate pinning (probes only trust specific hub certificate)
    - Auto-generated self-signed certificates (no manual PKI required)
    - Probe whitelist (hub rejects unknown probes)
    - Push-based architecture (no inbound firewall rules needed)

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

        - Prevents MITM attacks even if attacker has valid CA certificates
        - No dependency on system certificate stores
        - Perfect for private networks and air-gapped environments

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

    Web Assembly:
    rustup target add wasm32-unknown-unknown

    Leptos CLI:
    cargo install leptos-cli

Quick Start (Secure Bootstrap)

    This section walks through the first-time setup of a Hub and a Probe, including identity generation and cryptographic registration.

    Blentinel uses:

        - Ed25519 → Probe identity & signature verification
        - X25519 + ChaCha20-Poly1305 → Encrypted payload transport
        - Optional TLS with certificate pinning → Transport hardening

    The Hub will reject any probe whose public key is not explicitly registered.

    Step 1 — Build and Start the Hub

        From the project root:

        cargo build -p blentinelmake --release
        ./target/release/blentinelmake hub publish


        Deploy the generated app/ (from the publish/hub) directory to your server.

        Initialize configuration, to create a template config file:

        ./hub --init

        If you already have a blentinel_hub.toml file you can skip that step

        Start the hub:

        ./hub

        On first run, the hub will automatically generate:

        hub_identity.key
        hub_auth.token

        What these files mean
        File	                Purpose
        hub_identity.key	    Hub’s X25519 private key used for encrypted sessions
        hub_auth.token	        Internal authentication token
        hub_tls_cert.pem	    (Optional) TLS certificate
        hub_tls_key.pem	        (Optional) TLS private key

        These files must be kept secure.

        The hub is now running — but no probes are authorized yet.

    Probe Registration & Key Exchange

        Blentinel uses a strict whitelist model.

        Each probe has a permanent cryptographic identity.
        The hub must explicitly trust that identity.


    Step 2 — Build and Configure the Probe

        On your build machine:

        ./target/release/blentinelmake probe publish --target x86_64-unknown-linux-gnu

        Deploy the generated app/ directory to the probe machine.

        Initialize configuration:

        ./probe --init

        Edit blentinel_probe.toml:

        [agent]
        company_id = "COMPANY_NAME"
        hub_url = "http://HUB_IP:3000"
        interval = 60

        Add monitoring resources under [[resources]].


    Step 3 — First Probe Run (Identity Generation)

        Start the probe:

        ./probe

        On first run, it generates a permanent Ed25519 keypair and prints:

        Probe public key:
        a773d201237ea75c354a4c2e05325110ea6d4fee9f69c40f1c14b882b2a7dfcd

        Copy this public key.

    Step 4 — Register Probe in Hub

        Edit blentinel_hub.toml on the hub server:

        [[probes]]
        name = "HOMELAB"
        public_key = "a773d201237ea75c354a4c2e05325110ea6d4fee9f69c40f1c14b882b2a7dfcd"

        Save the file.

        Because probe whitelist is hot-reloadable, the hub will apply this change immediately.

        No restart required.

    Step 5 — Successful Communication

        Restart the probe.

        The flow is now:

            1 - Probe signs payload with Ed25519 private key

            2 - Hub verifies signature using registered public key

            3 - X25519 key exchange derives shared session key

            4 - Payload decrypted and stored

        If the public key does not match, you will see:

            Security Alert: Invalid signature from <hex>

        This means:

            - The probe key in the hub config is wrong

            - Or the probe was reinstalled and generated a new identity

            Update the hub config with the new key.

    Correct Bootstrap Order

        1 - Start Hub (generates hub_identity.key)

        2 - Start Probe (generates probe public key)

        3 - Add probe public key to hub config

        4 - Restart or hot-reload hub config

        5 - Restart probe

        After this, communication is persistent and secure.

    Security Model Summary

        Blentinel is designed for MSP environments and zero-trust networks.
            - Probes initiate all connections outbound.
            - No inbound firewall rules required.
            - Unknown probes are rejected.
            - Payloads are signed and encrypted.
            - Optional TLS adds transport-layer hardening.

        If a probe binary is replaced, its identity changes.
        The hub will reject it until re-registered.

        This is intentional.

    Generated Runtime Files

        At runtime, the following files may be created:

            hub_identity.key
            hub_auth.token
            hub_tls_cert.pem
            hub_tls_key.pem
            blentinel.db


        These should be backed up and protected appropriately.

Building from Source

    1. Clone the Repository

    git clone https://codeberg.org/mr_jimmybob/blentinel.git
    cd blentinel

    Now choose either 2.a or 2.b, not both.

    2.a Use the bulid tool

        cargo build -p blentinelmake --release

        Then run the binary in the target directory:
            ./target/release/blentinelmake

            If run without arguments it asks you what you want to do, interactive.

        Otherwise, you can run it with arguments, see help for usage:

            ./target/debugrelease/blentinelmake.exe --help # for more ways to run the script

    2.b Build the hub and the probe manually

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

            sc.exe create $serviceName binPath= "`"$exePath`"" start= auto | Out-Null
            sc.exe description $serviceName "Blentinel network monitoring probe" | Out-Null
            sc.exe failure $serviceName reset= 90000 actions= restart/300000/restart/300000/restart/300000 | Out-Null

            Start-Service $serviceName

        Uninstall:

            Stop-Service BlentinelProbe
            sc.exe delete BlentinelProbe

        Logs:

            Windows Event Viewer

## Hot Reload Configuration

Both the probe and hub support hot reloading of configuration files. When you modify a config file, the application automatically detects the change and reloads the configuration without requiring a restart.

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
    ├── blentinel.db
    ├── blentinel.db-shm
    ├── blentinel.db-wal
    ├── blentinel_hub.toml
    ├── blentinelmake
    │   ├── Cargo.toml
    │   └── src
    │       └── main.rs
    ├── build_blentinelmake.ps1
    ├── build_blentinelmake.sh
    ├── Cargo.toml
    ├── common
    │   ├── Cargo.toml
    │   └── src
    │       ├── lib.rs
    │       └── models.rs
    ├── hub
    │   ├── blentinel.db
    │   ├── blentinel.db-shm
    │   ├── blentinel.db-wal
    │   ├── blentinel_hub.toml
    │   ├── Cargo.toml
    │   ├── end2end
    │   │   ├── package-lock.json
    │   │   ├── package.json
    │   │   ├── playwright.config.ts
    │   │   ├── tests
    │   │   │   └── example.spec.ts
    │   │   └── tsconfig.json
    │   ├── hub_auth.token
    │   ├── hub_identity.key
    │   ├── LICENSE
    │   ├── public
    │   │   └── favicon.ico
    │   ├── README.md
    │   ├── src
    │   │   ├── alerts
    │   │   │   ├── email.rs
    │   │   │   ├── engine.rs
    │   │   │   ├── silence.rs
    │   │   │   └── state.rs
    │   │   ├── alerts.rs
    │   │   ├── api.rs
    │   │   ├── app.rs
    │   │   ├── archive
    │   │   │   ├── engine.rs
    │   │   │   └── monitor.rs
    │   │   ├── archive.rs
    │   │   ├── archive_viewer.rs
    │   │   ├── args.rs
    │   │   ├── auth.rs
    │   │   ├── config.rs
    │   │   ├── crypto.rs
    │   │   ├── db
    │   │   │   └── types.rs
    │   │   ├── db.rs
    │   │   ├── hot_reload.rs
    │   │   ├── identity.rs
    │   │   ├── lib.rs
    │   │   ├── main.rs
    │   │   └── tls.rs
    │   └── style
    │       └── main.scss
    ├── probe
    │   ├── blentinel_probe.toml
    │   ├── build.rs
    │   ├── Cargo.toml
    │   ├── hub_cert.pem
    │   └── src
    │       ├── args.rs
    │       ├── checks.rs
    │       ├── config.rs
    │       ├── crypto.rs
    │       ├── hot_reload.rs
    │       ├── identity.rs
    │       ├── main.rs
    │       ├── monitor.rs
    │       ├── storage.rs
    │       ├── tls.rs
    │       └── transport.rs
    ├── publish
    ├── README.md
    ├── rust.instructions.md
    └── target

    Generated files (auto-created at runtime):
        hub_tls_cert.pem                # Hub TLS certificate (public)
        hub_tls_key.pem                 # Hub TLS private key (keep secure!)
        hub_identity.key                # Hub X25519 key
        hub_auth.token                  # Admin authentication token
