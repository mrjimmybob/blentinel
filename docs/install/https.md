# HTTPS

The web UI and API use **HTTPS by default** (DECISIONS D-03, D-43). Passwords and session cookies never cross plain HTTP. Probe reports are signed and encrypted end to end anyway ([security.md](../security.md)); HTTPS adds a transport layer for probes built to use it.

## How the hub listens (split mode, the default)

| Port | Protocol | Serves |
|---|---|---|
| `port` (3000) | plain HTTP | **only** the probe endpoints (`/api/handshake`, `/api/report`). Everything else is redirected (308) to the same path on HTTPS. |
| `https_port` (3443) | HTTPS | everything: the web UI, the API, and the probe endpoints |

So existing probes (`hub_url = "http://…:3000"`) keep working without being rebuilt, and people use `https://<server>:3443`.

On first start the hub creates a **self-signed certificate** (ECDSA P-256, valid 10 years) and logs its fingerprint:

```
INFO TLS: Generating new self-signed certificate...
INFO TLS: Certificate saved to hub_tls_cert.pem
INFO TLS: Private key saved to hub_tls_key.pem
INFO TLS: Certificate fingerprint (SHA-256): A1:B2:C3:…
```

**Keep `hub_tls_key.pem` private.** Open port 3443 in the hub's firewall; port 3000 only needs to be reachable by probes.

### Your browser warns about the certificate

A self-signed certificate isn't signed by an authority the browser knows, so it shows a warning the first time. Check that the fingerprint in the browser (certificate details, SHA-256) matches the one in the hub's log, then accept it. To avoid the warning for everyone, either install `hub_tls_cert.pem` as a trusted certificate on the admins' machines, or use a certificate from your own CA or a public one (below). Let's Encrypt support is planned (ROADMAP M2-14).

## The other modes

In `blentinel_hub.toml` (all reloaded without a restart, D-38):

| Setting | Result |
|---|---|
| `[server.tls]` `enabled = true`, `https_port = 3443` (**default**, also when the section is missing) | split mode, as above |
| `https_port` = the same as `port` | HTTPS only, on `port`. Probes must be built with the certificate and use `https://`. |
| `enabled = false` | plain HTTP for everything. Only behind a TLS-terminating proxy (below), or for local testing. **Passwords then cross the network in clear text** unless a proxy encrypts them. |

## Probes over HTTPS

Probes only trust the certificate they were **built with** (`probe/hub_cert.pem`, compiled in), not the operating system's certificate authorities. That protects against interception even by someone holding a certificate from a trusted CA. Runtime pinning via enrollment codes, which removes the rebuild, is planned (DECISIONS D-01, ROADMAP M2).

1. Build probes with the hub's certificate:

   ```sh
   cp hub_tls_cert.pem probe/hub_cert.pem          # in the repository
   ./target/release/blentinelmake probe publish --target x86_64-unknown-linux-musl
   ```

   The build warns `hub_cert.pem not found! HTTPS will not work.` if the file is missing.

2. Deploy the new binary and point it at the HTTPS port in `blentinel_probe.toml` (reloaded):

   ```toml
   hub_url = "https://192.0.2.10:3443"
   ```

3. Once every probe uses HTTPS, you may set `https_port` equal to `port` (HTTPS only) and update the probes' `hub_url` port.

## Behind a reverse proxy

If a proxy (nginx, Caddy, Traefik, …) terminates TLS in front of the hub:

```toml
[server]
behind_proxy = true   # the proxy speaks HTTPS to browsers

[server.tls]
enabled = false       # the hub speaks plain HTTP to the proxy
```

With `behind_proxy`, the hub marks session cookies `Secure` and takes the client address for login rate limiting from the last `X-Forwarded-For` entry. It trusts the proxy for both, so **only set it when clients can't reach the hub directly** (listen on `127.0.0.1`, or firewall the port). Probes can report through the proxy or directly to the hub.

## Checking it works

- **Hub log:** `TLS: Loading existing certificate …` with the fingerprint, then `listening on http://…:3000 for probes (everything else goes to HTTPS on port 3443)` and `listening on https://…:3443`.
- **Redirect:** `curl -i http://<hub>:3000/login` answers `308` with `Location: https://<hub>:3443/login`.
- **Probes on HTTP:** `curl http://<hub>:3000/api/handshake` returns a 64-character hex key.
- **HTTPS:** `curl -k https://<hub>:3443/api/handshake` returns the same key (`-k` because the certificate is self-signed).
- **Pinning:** a probe built with a *different* certificate must fail to connect over HTTPS.

## Troubleshooting

| Problem | Check |
|---|---|
| Hub won't start | ports 3000 and 3443 aren't already in use; the hub can write `cert_path`/`key_path`; `host` is valid |
| The UI isn't reachable | use `https://…:3443`, not port 3000; the firewall allows 3443 |
| Browser warning | expected with the self-signed certificate; compare the fingerprint with the hub log (above) |
| Probe can't connect over HTTPS | `hub_url` starts with `https://` and uses the HTTPS port; the probe was built with the *current* `hub_cert.pem`; the firewall allows the port. Or keep it on `http://…:3000`. |
| Certificate expired (after 10 years) or replaced | stop the hub, delete both `.pem` files, start it (new certificate), then rebuild probes that use HTTPS |

## Your own certificate

Place your certificate and key at `cert_path` / `key_path` before starting the hub, and build HTTPS probes with that certificate as `probe/hub_cert.pem`. For example, a self-signed one with OpenSSL:

```sh
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
  -days 3650 -nodes -keyout hub_tls_key.pem -out hub_tls_cert.pem -subj "/CN=Blentinel Hub"
```
