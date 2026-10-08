# Alerts on your phone with ntfy

[ntfy](https://ntfy.sh) ("notify") sends push notifications to phones (Android, iOS) and browsers: whatever is published to a **topic** reaches everyone subscribed to it. The hub publishes its alerts to a topic you choose (an **ntfy** channel on the **Alerts** page, [guide/alerting.md](../guide/alerting.md#ntfy)); you subscribe to it in the ntfy app. It is free and open source, and it is **optional**: the hub needs neither ntfy nor any other outside service (DECISIONS D-70).

You choose the server:

- **ntfy.sh**, the public server run by ntfy's author: nothing to install. But a topic there is open to anyone who knows its name (it works as a password: the hub's **New random topic** makes one nobody can guess), and the alerts (customer, site and device names) pass through a server you don't run, which keeps them for 12 hours.
- **Your own ntfy server**, e.g. `https://ntfy.example.com` on a small VPS: the alerts stay with you, and only people with an account read them. The rest of this page sets one up.

## A self-hosted ntfy server

You need a server reachable from the internet (the phones connect to it), a DNS name pointing to it (here `ntfy.example.com`), and Docker. ntfy's own documentation has every option and other ways to install it: <https://docs.ntfy.sh/install/>.

### 1. Run ntfy

`/etc/ntfy/server.yml`:

```yaml
base-url: "https://ntfy.example.com"
listen-http: ":80"
behind-proxy: true
cache-file: "/var/cache/ntfy/cache.db"
auth-file: "/var/lib/ntfy/user.db"
# Nobody reads or publishes without an account.
auth-default-access: "deny-all"
# iPhones: ntfy.sh only wakes the app; the message comes from your server.
upstream-base-url: "https://ntfy.sh"
```

`docker-compose.yml`:

```yaml
services:
  ntfy:
    image: binwiederhier/ntfy
    container_name: ntfy
    command: serve
    restart: unless-stopped
    volumes:
      - /etc/ntfy:/etc/ntfy
      - /var/cache/ntfy:/var/cache/ntfy
      - /var/lib/ntfy:/var/lib/ntfy
    ports:
      - "127.0.0.1:2586:80"
```

`docker compose up -d`. It listens on `127.0.0.1:2586` only: HTTPS comes from the reverse proxy.

### 2. HTTPS in front of it

Any reverse proxy with a certificate works. With [Caddy](https://caddyserver.com), which gets the certificate from Let's Encrypt by itself, `/etc/caddy/Caddyfile`:

```
ntfy.example.com {
    reverse_proxy 127.0.0.1:2586
}
```

Open `https://ntfy.example.com`: ntfy's web app should load.

### 3. Accounts: one for the hub, one for the phones

```sh
# The hub publishes, and reads nothing.
docker exec -it ntfy ntfy user add blentinel
docker exec -it ntfy ntfy access blentinel 'blentinel*' write-only
docker exec -it ntfy ntfy token add blentinel      # prints tk_…: the hub's access token

# The phones read, and publish nothing.
docker exec -it ntfy ntfy user add oncall
docker exec -it ntfy ntfy access oncall 'blentinel*' read-only
```

Each command asks for a password where it needs one. Keep the token: it goes into the hub, which keeps it encrypted (`hub_data.key`).

### 4. The hub

On the hub's **Alerts** page, **Add a channel**:

- **Kind:** ntfy
- **Topic URL:** `https://ntfy.example.com/blentinel-alerts` (any topic the `blentinel` account may write: with the access above, one starting with `blentinel`)
- **Access token:** the `tk_…` from step 3

Save, then add a [routing rule](../guide/alerting.md#routing-rules) that sends to it.

### 5. The phones

Install the ntfy app (Android: Google Play or F-Droid; iOS: the App Store). In its settings, add your server (`https://ntfy.example.com`) with the `oncall` account, then subscribe to the topic (`blentinel-alerts`). Back on the hub, press **Send test**: the phone should ring within seconds.

Several teams, several topics: one channel per topic, and the routing rules choose which alerts go where.

## Troubleshooting

- **Send test says `the receiver answered 401 Unauthorized` (or `403 Forbidden`):** the token is missing or wrong, or its account may not write that topic (`ntfy access blentinel` lists what it may do).
- **`can't reach the receiver`:** the hub can't reach the server: its DNS name, the firewall, the reverse proxy. The hub only connects out; ntfy never connects to the hub.
- **The phone gets nothing, Send test works:** the app is subscribed to another topic or server, or (iOS) `upstream-base-url` is missing.
