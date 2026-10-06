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

A self-signed certificate isn't signed by an authority the browser knows, so it shows a warning the first time. Check that the fingerprint in the browser (certificate details, SHA-256) matches the one in the hub's log, then accept it. To avoid the warning for everyone, get a certificate from [Let's Encrypt](#a-certificate-from-lets-encrypt) (a hub people reach by a public name), install `hub_tls_cert.pem` as a trusted certificate on the admins' machines, or use a certificate from your own CA ([below](#your-own-certificate)).

## A certificate from Let's Encrypt

A hub reached by a public DNS name can get its certificate from Let's Encrypt (or another ACME authority) and renew it by itself (DECISIONS D-62). Browsers then trust it: no warning, nothing to install on the admins' machines.

**Probes notice nothing.** The hub asks for the certificate with its own TLS key, so the new certificate has the same public key, which is what probes pin: no rotation, no enrolling again, and that stays true at every renewal and if you turn it off again. Probes keep trusting the pin, not Let's Encrypt.

### What it needs

- **A DNS name** that points at the hub, such as `hub.example.com` (up to 10 names). Not an IP address, not a wildcard.
- **Let's Encrypt must reach the hub on one of these ports**, from the internet:

  | Validation | Let's Encrypt connects to | So |
  |---|---|---|
  | **TLS-ALPN** (the default) | port **443** of the name | the hub's HTTPS port is 443 (`hub.sh … --https-port 443`), or the router forwards 443 to it (3443) |
  | **HTTP** | port **80** of the name | the hub's plain-HTTP `port` is 80, or the router forwards 80 to it (3000). Not on an HTTPS-only hub |

  Only for a moment, while a certificate is issued (every two months or so), but the port must stay open for renewals.
- The hub serves HTTPS itself (`[server.tls] enabled = true`, the default). Behind a reverse proxy, the proxy gets the certificate instead.

### Turning it on

On the admin page, **HTTPS certificate**:

1. Tick **Get the certificate from an ACME authority**.
2. **Names the hub is reached by:** `hub.example.com` (several: separated by commas).
3. **Contact e-mail** (optional): Let's Encrypt no longer sends expiry warnings, so this is only for their account notices.
4. **Authority:** Let's Encrypt. Choose **Let's Encrypt staging** first if you're not sure the name and port are right: it issues test certificates browsers don't trust, but has generous limits; switch to Let's Encrypt once it works. **Another ACME authority** takes a directory URL (and the authority's root certificate if it isn't a public one), for a company's own CA such as step-ca.
5. **Validation:** TLS-ALPN (port 443) or HTTP (port 80); the page says which hub port that must reach.
6. Accept the terms of service, and **Save**.

The hub asks at once and answers when done (a few seconds). The section then shows the certificate (**Issued by** the authority's intermediate, for your names, valid until …) and when the hub next asks. The browser shows the new certificate on its next connection: the hub swaps it in place, without a restart.

### Renewing

The hub looks every hour and renews a certificate when a third of its life is left (30 days of 90; shorter-lived ones follow the same rule). **Renew now** asks at once. If a request fails, the section shows the authority's reason, the log has it, and the hub tries again after an hour, then 2, 4, … up to once a day (failed validations count against Let's Encrypt's limits). Saving the settings again with new names or another authority asks at once, and so does saving after a failure (once you fixed the DNS or the port); otherwise a new contact or validation method is used at the next renewal.

While a [key rotation](hub.md#rotating-the-certificate-and-key) that replaces the certificate is waiting for probes, the hub gets the next certificate from the authority too, for the next key, so the switch serves a trusted one.

### Turning it off

Untick it and **Save**: the hub goes back to a self-signed certificate **with the same key** (so the same pin). Browsers warn again. Turned off, the hub never touches the certificate file, so it can be [your own](#your-own-certificate).

### What it keeps

The certificate is `hub_tls_cert.pem` (the whole chain), its key `hub_tls_key.pem` as before, and the ACME account's key `hub_acme_account.json` in the data folder. Backups include all three.

## The other modes

In `blentinel_hub.toml` (all reloaded without a restart, D-38):

| Setting | Result |
|---|---|
| `[server.tls]` `enabled = true`, `https_port = 3443` (**default**, also when the section is missing) | split mode, as above |
| `https_port` = the same as `port` | HTTPS only, on `port`. Every probe must be enrolled with an `https://` address. |
| `enabled = false` | plain HTTP for everything. Only behind a TLS-terminating proxy (below), or for local testing. **Passwords then cross the network in clear text** unless a proxy encrypts them. |

## Probes over HTTPS

A probe's **enrollment code** carries a pin of the hub's certificate: the SHA-256 of its **public key**, taken when the code is created. The probe then trusts **only** a certificate with that key, not the operating system's certificate authorities, from its very first request. That protects against interception even by someone holding a certificate from a trusted CA. Nothing is built per hub (DECISIONS D-01, D-47). The hub replaces its certificate, and probes follow, by [rotation](hub.md#rotating-the-certificate-and-key) (D-61); a certificate renewed with the same key keeps the same pin.

Which transport a probe uses follows the **hub address** in its code (the Add probe dialog, [probe.md](probe.md#1-add-probe-on-the-hub)):

| Address in the code | Transport |
|---|---|
| `https://<hub>:3443` (the default: the address you're using in the browser) | HTTPS, pinned to the hub's certificate |
| `https://…` of a proxy with a public certificate, while the hub itself has TLS off | HTTPS, trusting the system's certificate authorities (the code carries no pin) |
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
| Browser warning | expected with the self-signed certificate; compare the fingerprint with the hub log (above), or get one from [Let's Encrypt](#a-certificate-from-lets-encrypt) |
| Let's Encrypt: "Connection refused", "Timeout during connect" or "Failed to connect" | the name's DNS points at the hub's public address; the port the validation uses (443 for TLS-ALPN, 80 for HTTP) reaches the hub through the firewall and router; try the staging authority while fixing it |
| Let's Encrypt: "rateLimited" | too many certificates or failed validations for these names: wait (the message says until when), use the staging authority meanwhile |
| Let's Encrypt: "the hub doesn't serve HTTPS itself" | `[server.tls] enabled = false`: the proxy in front of the hub gets the certificate |
| Probe can't connect over HTTPS | the address in its code is one the probe can reach, with the HTTPS port; the firewall allows the port; the hub's certificate hasn't changed since the code was made (the probe log says `the hub's certificate doesn't match the one in the enrollment code`) |
| Certificate expiring (after 10 years; or Let's Encrypt's isn't renewing: the admin page says why) | **Renew now** if it comes from Let's Encrypt, else rotate it on the admin page (**Hub keys**): probes take the new one before the hub switches ([hub.md](hub.md#rotating-the-certificate-and-key)). Replacing the `.pem` files by hand cuts off every probe that uses HTTPS until it is enrolled again |

## Your own certificate

Place your certificate and key at `cert_path` / `key_path` before starting the hub (leave Let's Encrypt off on the admin page: on, it replaces the certificate). Probes pin the certificate's **public key**: a certificate your CA issues for the hub's existing key (a request signed with `hub_tls_key.pem`) keeps every probe working; a new key means codes created from then on pin it, and probes enrolled before must be enrolled again (or [rotate](hub.md#rotating-the-certificate-and-key) instead). For example, a self-signed one with OpenSSL:

```sh
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
  -days 3650 -nodes -keyout hub_tls_key.pem -out hub_tls_cert.pem -subj "/CN=Blentinel Hub"
```
