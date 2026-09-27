# HTTPS with certificate pinning

Reports are always encrypted and signed at the application level ([security.md](../security.md)). HTTPS adds a transport layer on top. It also protects the **web UI**, whose login token and session cookie otherwise travel in plain HTTP.

How it works today:

- The hub generates a **self-signed certificate** (ECDSA P-256, valid 10 years) the first time TLS is enabled.
- Probes are **built with that certificate embedded** (`probe/hub_cert.pem`) and trust only it, not the operating system's certificate authorities.

So a new hub certificate means **rebuilding the probes**. Runtime pinning through enrollment codes, which removes the rebuild, is planned (DECISIONS D-01, ROADMAP M2).

## Modes

| Mode | Config | Result |
|---|---|---|
| HTTP only (default) | no `[server.tls]` or `enabled = false` | HTTP on `port` |
| HTTPS only | `enabled = true` | HTTPS on `port` |
| Both (for migrating) | `enabled = true` + `https_port = 3443` | HTTP on `port`, HTTPS on `https_port` |

## Setup

### 1. Enable TLS on the hub

```toml
[server.tls]
enabled = true
cert_path = "hub_tls_cert.pem"
key_path = "hub_tls_key.pem"
https_port = 3443          # keep HTTP on `port` while probes migrate
```

Restart the hub. On the first TLS start it prints:

```
INFO TLS: Generating new self-signed certificate...
INFO TLS: Certificate saved to hub_tls_cert.pem
INFO TLS: Private key saved to hub_tls_key.pem
INFO TLS: Certificate fingerprint (SHA-256): A1:B2:C3:…
```

Note the fingerprint for your records. **Keep `hub_tls_key.pem` private.**

### 2. Build probes with the certificate

```sh
cp hub_tls_cert.pem probe/hub_cert.pem          # in the repository
./target/release/blentinelmake probe publish --target x86_64-unknown-linux-musl
```

The build warns `hub_cert.pem not found! HTTPS will not work.` if the file is missing.

### 3. Point probes at HTTPS

In each probe's `blentinel_probe.toml`:

```toml
hub_url = "https://192.0.2.10:3443"
```

Deploy the new binary and restart the probe.

### 4. Switch to HTTPS only

When every probe reports over HTTPS, remove `https_port` so HTTPS replaces HTTP on the main port, update the probes' `hub_url` port, and restart the hub.

## Checking it works

- **Hub:** the log shows `TLS: Loading existing certificate …` with the fingerprint, then `Blentinel Hub listening on https://…`.
- **Hub from a shell:** `curl -k https://<hub>:3443/api/handshake` returns a 64-character hex key (`-k` because the certificate is self-signed).
- **Pinning:** a probe built with a *different* certificate must fail to connect.

## Troubleshooting

| Problem | Check |
|---|---|
| Hub won't start after enabling TLS | the port isn't already in use; the hub can write `cert_path`/`key_path`; `host` is valid |
| Probe can't connect over HTTPS | `hub_url` starts with `https://` and uses the HTTPS port; the probe was built with the *current* `hub_cert.pem`; the firewall allows the port |
| Certificate expired (after 10 years) or replaced | stop the hub, delete both `.pem` files, start it (new certificate), then rebuild and redeploy every probe |

## Your own certificate

Place your certificate and key at `cert_path` / `key_path` before starting the hub, then build probes with that certificate as `probe/hub_cert.pem`. For example, a self-signed one with OpenSSL:

```sh
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
  -days 3650 -nodes -keyout hub_tls_key.pem -out hub_tls_cert.pem -subj "/CN=Blentinel Hub"
```

## Going back to HTTP

Set `enabled = false` (or remove `[server.tls]`) and restart the hub. On probes, set `hub_url` back to `http://…` and restart them; a probe built with a certificate still works over HTTP.
