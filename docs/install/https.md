# HTTPS

The web UI and API use **HTTPS by default** (DECISIONS D-03, D-43). Passwords and session cookies never cross plain HTTP. Probe reports are signed and encrypted end to end anyway ([security.md](../security.md)); HTTPS adds a transport layer, and probes use it by default: their enrollment code pins the hub's certificate.

## How the hub listens (split mode, the default)

| Port | Protocol | Serves |
|---|---|---|
| `port` (3000) | plain HTTP | **only** the probe endpoints (`/api/enroll`, `/api/report`), for probes enrolled with an `http://` address. Everything else is redirected (308) to the same path on HTTPS. |
| `https_port` (3443) | HTTPS | everything: the web UI, the API, and the probe endpoints |

People and probes use `https://<server>:3443`. Port 3000 is only for probes you deliberately enrolled with `http://…:3000`.

On first start the hub creates a **self-signed certificate** (ECDSA P-256, valid 10 years) and logs its fingerprint:

```
INFO TLS: Generating new self-signed certificate...
INFO TLS: Certificate saved to hub_tls_cert.pem
INFO TLS: Private key saved to hub_tls_key.pem
INFO TLS: Certificate fingerprint (SHA-256): A1:B2:C3:…
```

**Keep `hub_tls_key.pem` private.** Open port 3443 in the hub's firewall; port 3000 only if some probes use plain HTTP.

### Your browser warns about the certificate

A self-signed certificate isn't signed by an authority the browser knows, so it shows a warning the first time. Check that the fingerprint in the browser (certificate details, SHA-256) matches the one in the hub's log, then accept it. To avoid the warning for everyone, either install `hub_tls_cert.pem` as a trusted certificate on the admins' machines, or use a certificate from your own CA or a public one (below). Let's Encrypt support is planned (ROADMAP M2-14).

## The other modes

In `blentinel_hub.toml` (all reloaded without a restart, D-38):

| Setting | Result |
|---|---|
| `[server.tls]` `enabled = true`, `https_port = 3443` (**default**, also when the section is missing) | split mode, as above |
| `https_port` = the same as `port` | HTTPS only, on `port`. Every probe must be enrolled with an `https://` address. |
| `enabled = false` | plain HTTP for everything. Only behind a TLS-terminating proxy (below), or for local testing. **Passwords then cross the network in clear text** unless a proxy encrypts them. |

## Probes over HTTPS

A probe's **enrollment code** carries the SHA-256 fingerprint of the hub's certificate (the one in the hub log), taken when the code is created. The probe then trusts **only** that certificate, not the operating system's certificate authorities, from its very first request. That protects against interception even by someone holding a certificate from a trusted CA. Nothing is built per hub (DECISIONS D-01, D-47).

Which transport a probe uses follows the **hub address** in its code (the Add probe dialog, [probe.md](probe.md#1-add-probe-on-the-hub)):

| Address in the code | Transport |
|---|---|
| `https://<hub>:3443` (the default: the address you're using in the browser) | HTTPS, pinned to the hub's certificate |
| `https://…` of a proxy with a public certificate, while the hub itself has TLS off | HTTPS, trusting the system's certificate authorities (the code carries no fingerprint) |
| `http://<hub>:3000` | plain HTTP. The reports are still signed and encrypted end to end. |

To move a probe from HTTP to HTTPS, enroll it again with a new code.

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
- **HTTPS:** `curl -k https://<hub>:3443/api/setup` answers (`-k` because the certificate is self-signed).
- **Probes:** the probe log shows `Report delivered`; its page in the hub shows it as active.
- **Pinning:** after replacing the hub's certificate, a probe enrolled before must refuse to connect.

## Troubleshooting

| Problem | Check |
|---|---|
| Hub won't start | ports 3000 and 3443 aren't already in use; the hub can write `cert_path`/`key_path`; `host` is valid |
| The UI isn't reachable | use `https://…:3443`, not port 3000; the firewall allows 3443 |
| Browser warning | expected with the self-signed certificate; compare the fingerprint with the hub log (above) |
| Probe can't connect over HTTPS | the address in its code is one the probe can reach, with the HTTPS port; the firewall allows the port; the hub's certificate hasn't changed since the code was made (the probe log says `the hub's certificate doesn't match the one in the enrollment code`) |
| Certificate expired (after 10 years) or replaced | stop the hub, delete both `.pem` files, start it (new certificate), then enroll probes that use HTTPS again with new codes (automatic rotation: ROADMAP M2-13) |

## Your own certificate

Place your certificate and key at `cert_path` / `key_path` before starting the hub. Codes created from then on pin it; probes enrolled before the change must be enrolled again. For example, a self-signed one with OpenSSL:

```sh
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
  -days 3650 -nodes -keyout hub_tls_key.pem -out hub_tls_cert.pem -subj "/CN=Blentinel Hub"
```
