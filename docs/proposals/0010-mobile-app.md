# 0010 · Blentinel Mobile — App Design Spec

| | |
|---|---|
| **Status** | **Separate project, pending** (D-23). Written 2026-09-26 against the *current* API for a Rork prototype. |
| **Depends on** | 0001 (per-user API tokens replace the shared-token cookie, D-14), 0005 (Warning/Unknown statuses, incidents/ack), 0007 (SSE events) |
| **Before resuming** | Update §3 (status model → 4 states), §4.1 (auth → Bearer API tokens), §4.3 (silences keyed by `check_id`), and add Acknowledge to the Alerts tab. |

> Build brief for an AI app builder (Rork / Expo + React Native + TypeScript).
> Source of truth: the Blentinel Hub web UI (`hub/src/app.rs`) and REST API (`hub/src/api.rs`, `hub/src/db.rs`).

---

## 1. What the app is

Blentinel is a monitoring platform for Managed Service Providers (MSPs). Small agents called **probes** run inside client networks, check local **devices** (ping, HTTP, TCP port, disk, CPU, memory, uptime), and push results to a central **Hub**. The Hub has a web dashboard.

**The mobile app is a read-mostly companion to the web dashboard** for technicians on the go. It lets them:

1. See at a glance which client companies are healthy, degraded, or critical.
2. Drill into a company → its probes → each probe's devices.
3. See an uptime chart per company (24h / 7d / 30d / all).
4. Silence / unsilence alerts for a noisy device.
5. See hub storage / archive status (read-only).

### Data hierarchy

```
Hub
 └── Company         (e.g. "acme-corp")          – identified by company_id string
      └── Probe       (one agent at a client site) – identified by probe_id (long hex string)
           └── Device  (a monitored resource)      – identified by name + target
```

---

## 2. Tech & platform requirements

- Expo (React Native) + TypeScript, iOS and Android. Should also run in Expo web preview.
- Navigation: stack navigator with a bottom tab bar (see §5).
- Data fetching: React Query (TanStack Query) or equivalent, with polling support.
- Charts: `react-native-svg` (or `victory-native` / `react-native-gifted-charts`).
- Storage: `expo-secure-store` for hub URL + token; AsyncStorage for theme preference.
- **Must include a Demo Mode** backed by the mock data in §9, selectable on the login screen. The Hub usually runs on a private network and may not be reachable from a cloud preview, so the app must be fully usable without a live Hub.

---

## 3. Status model (important — use consistently everywhere)

Three severity levels, same colors as the web hub:

| Level | Meaning | Color (light / dark) |
|---|---|---|
| **HEALTHY** | Everything reporting, nothing down | green `#22c55e` / `#4ade80` |
| **DEGRADED** | A probe has stopped reporting ("expired"); its data may be stale | amber `#f59e0b` / `#fbbf24` |
| **CRITICAL** | An active probe reports one or more devices down | red `#ef4444` / `#f87171` |

Accent / links: blue `#3b82f6` / `#60a5fa`.

### Rules

**Company status** (from `DashboardCompany`):
- `expired_probes > 0` → DEGRADED (stale data overrides device counts)
- else `devices_down > 0` → CRITICAL
- else → HEALTHY

**Company list sort order:** companies with `devices_down > 0` first, then those with `expired_probes > 0`, then healthy; ties sorted alphabetically by `company_id`.

**Probe status** (from `CompanyProbe`):
- `status != "active"` (i.e. `"expired"`) → DEGRADED
- else `devices_down > 0` → CRITICAL
- else → HEALTHY

**Device status:** `status` is the string `"Up"` or `"Down"`. Down devices are shown first, then alphabetical by name.

A probe becomes `"expired"` when the hub hasn't heard from it for `probe_timeout_secs` (default 300 s).

---

## 4. Backend API

Base URL is user-supplied, e.g. `http://192.168.1.50:3000` (default port 3000; HTTPS optional, often on 3443 with a self-signed cert).

### 4.1 Authentication

The Hub has a single shared **admin token** (32-char hex string). There are no usernames.

| Call | Details |
|---|---|
| `POST /api/login` | Body: the raw token as **plain text** (`Content-Type: text/plain`), not JSON. `200` → success, response sets cookie `blentinel_session=<id>` (HttpOnly, 24 h). `401` → bad token. |
| `POST /api/logout` | Empty body. Clears the session. |

- Every other endpoint needs the `blentinel_session` cookie. Use `credentials: 'include'` on all fetches; on iOS/Android the native cookie jar keeps it.
- Any endpoint may return **`401`** when the session has expired (24 h, or a hub restart, because sessions are held in memory). On 401: log in again silently with the stored token, retry once, and go to the Login screen only if that also fails.
- Error responses are plain text (`"Internal error"`), not JSON.

### 4.2 Read endpoints

All return JSON. Timestamps are ISO-8601 UTC strings ending in `Z` (e.g. `"2026-03-08T01:27:26.522816772Z"`, which may have nanosecond precision). Display them in the device's local time.

#### `GET /api/dashboard/companies` → `DashboardCompany[]`
```ts
interface DashboardCompany {
  company_id: string;
  total_probes: number;
  active_probes: number;
  expired_probes: number;
  devices_up: number;      // across the latest report of every probe
  devices_down: number;
  last_report: string | null; // ISO timestamp; null = never reported
}
```

#### `GET /api/company/{company_id}/probes` → `CompanyProbe[]`
Sorted by most recently seen.
```ts
interface CompanyProbe {
  probe_id: string;         // long hex; show first 8 chars + "…" if probe_name is empty
  probe_name: string;       // friendly name, may be ""
  hostname: string | null;  // show "—" if null
  site: string | null;      // show "—" if null
  status: "active" | "expired";
  last_seen_at: string | null;
  devices_up: number;
  devices_down: number;
}
```

#### `GET /api/probe/{probe_id}/devices` → `ProbeDevice[]`
Devices from the probe's latest report.
```ts
interface ProbeDevice {
  name: string;
  resource_type: "ping" | "http" | "tcp" | "local_disk" | "local_cpu"
               | "local_mem" | "local_uptime" | "local_data" | "error";
  target: string;             // IP, URL, host:port, or "localhost"
  status: "Up" | "Down";
  message: string | null;     // e.g. "HTTP 200", "Port Open", "CPU usage: 12.3%"
  latency_ms: number | null;  // show "42ms"; "—" if null
  metric_value: number | null;
  metric_unit: string | null; // "%" or "seconds"
}
```
Display rules:
- **Metric**: if both `metric_value` and `metric_unit` exist, show `value.toFixed(1) + unit` (e.g. `87.3%`). For `metric_unit === "seconds"` (uptime), show `message` instead, e.g. "Uptime: 3d 4h 12m".
- For `%` metrics (disk, CPU, mem), show a small horizontal progress bar: green below 80%, amber from 80% to 90%, red above 90%.
- Icon per `resource_type`: ping → radar/wifi, http → globe, tcp → plug, local_disk → hard drive, local_cpu → chip, local_mem → memory, local_uptime → clock, error → alert.

#### `GET /api/company/{company_id}/uptime?range=24h|7d|30d|all` → `UptimeBucket[]`
```ts
interface UptimeBucket {
  bucket: string;     // "YYYY-MM-DD HH:MM" — UTC, no "Z"
  up_count: number;   // number of "Up" check results in the bucket
  down_count: number;
}
```
| range | window | bucket size | ~points |
|---|---|---|---|
| `24h` (default) | last 24 h | 15 min | 96 |
| `7d` | last 7 days | 1 h | 168 |
| `30d` | last 30 days | 4 h | 180 |
| `all` | everything retained | 1 day | ≤ retention |

**Plot uptime % = `up_count / (up_count + down_count) * 100`** (100% if total is 0). Y-axis 0–100%. X-axis labels: `HH:MM` for 24h, `DD/MM` for other ranges. Empty array → "No uptime data available".

#### `GET /api/silences` → `AlertSilence[]` (active silences only)
```ts
interface AlertSilence {
  id: number;
  scope_type: "resource";
  scope_id: string;          // the resource_key (see below)
  reason: string;
  created_at: string;
  expires_at: string | null; // null = forever
}
```

#### `GET /api/admin/storage-info` → `StorageInfo`
```ts
interface StorageInfo {
  current_db_size_mb: number;
  warn_threshold_mb: number;
  retention_days: number;
  last_archive_date: string | null;
  last_archive_size_mb: number | null;
  warning: boolean;           // true when DB size ≥ threshold
}
```

#### `GET /api/admin/archives` → `ArchiveMetadata[]`
```ts
interface ArchiveMetadata { id: number; filename: string; created_at: string; cutoff_date: string; size_mb: number; }
```

#### Archive (historical, read-only) endpoints
Same shapes as the live ones:
- `GET /api/archive/{archive_id}/companies` → `DashboardCompany[]`
- `GET /api/archive/{archive_id}/company/{company_id}/probes` → `CompanyProbe[]`
- `GET /api/archive/{archive_id}/company/{company_id}/uptime?range=…` → `UptimeBucket[]`
- `GET /api/archive/{archive_id}/probe/{probe_id}/devices` → `ProbeDevice[]`

### 4.3 Write endpoints (silencing only)

**Resource key** identifies a device for silencing:
```
resource_key = `${company_id}:${probe_id}:${device.name}:${device.target}`
```
A device is "muted" if any silence has `scope_type === "resource" && scope_id === resource_key`.

| Call | Body (JSON) | Response |
|---|---|---|
| `POST /api/silence` | `{ "resource_key": string, "reason": string, "duration_hours": 1 \| 24 \| 168 \| null }` (null = forever) | `{ "success": true, "silence_id": number }` |
| `POST /api/silence/delete` | `{ "silence_id": number }` | `{ "success": true }` |

> The hub also has destructive admin endpoints (delete company data, remove probe, create archive). They are **out of scope for the mobile app** and must not be implemented.

---

## 5. Navigation

Bottom tab bar (3 tabs) once logged in:

1. **Dashboard** (home icon) — stack: Dashboard → Company Detail → Probe Detail → Device Detail (sheet)
2. **Alerts** (bell icon) — list of problem devices + active silences
3. **Settings** (gear icon) — hub connection, theme, storage/archive info, logout

Login is a separate full-screen flow, shown when there is no valid session.

---

## 6. Screens

### 6.1 Login / Connect
- App logo + "Blentinel" wordmark; subtitle "Connect to your Hub".
- Fields: **Hub URL** (placeholder `http://192.168.1.50:3000`, URL keyboard, no autocorrect), **Admin Token** (secure text entry with show/hide toggle, monospace).
- Primary button **Connect** → shows a spinner and "Connecting…" while the request is in flight.
- Errors inline under the button: "Invalid token. Please try again." (401), "Can't reach hub — check the URL and network." (network error).
- Checkbox **Remember this hub** (on by default) → stores URL + token in SecureStore so the app can log in again automatically.
- Secondary text button **Try Demo Mode** → uses the mock data in §9, with a small "DEMO" chip shown in the header everywhere.

### 6.2 Dashboard (home)
Header: title "Dashboard", subtitle "Updated 12s ago" (relative, ticking).

**Summary strip** (horizontally scrollable stat tiles, or a 2×3 grid):
| Tile | Value | Color |
|---|---|---|
| Companies | count | neutral |
| Probes | sum `total_probes` | neutral |
| Active | `sum(active)/sum(total)` | green |
| Devices Up | sum | green |
| Devices Down | sum | red if > 0 |

**Overall banner** above the list: if any company is CRITICAL → red banner "N companies need attention"; else if any DEGRADED → amber "N probes not reporting"; else green "All systems operational".

**Filter chips:** All · Critical · Degraded · Healthy (with counts). **Search bar** to filter by company_id.

**Company cards** (sorted per §3):
- Left color bar (4 px) in the status color.
- Title: `company_id` (bold).
- Status pill: HEALTHY / DEGRADED / CRITICAL.
- 2×2 mini stats: Active · Expired (red if > 0) · Devices Up (green) · Devices Down (red).
- Footer: status dot + "Last report: 3 min ago" (relative; "Never" if null). Long-press shows the absolute timestamp.
- Tap → Company Detail.

States: skeleton cards while loading; error card with **Retry**; empty state "No companies found. Deploy a probe to get started."

Pull-to-refresh. **Auto-poll every 15 s** while the screen is focused and the app is in the foreground (same as the web). Don't re-render or re-order cards when data hasn't changed. Avoid flicker, which has been a recurring problem on the web version.

### 6.3 Company Detail
Header: back button, title = `company_id`, status pill.

**Uptime chart card**
- Title changes with range: "Uptime — Last 24 Hours" / "Last 7 Days" / "Last 30 Days" / "All Data".
- Segmented control: `24h | 7d | 30d | All` (default 24h).
- Line + light area-fill chart of uptime %, 0–100% y-axis with gridlines at 0/25/50/75/100.
- Tap or drag on the chart → tooltip showing local time of the bucket, uptime %, and "X up / Y down".
- Show the average uptime % for the selected range as a big number above the chart (e.g. **99.2%**).

**Probes section** (header "Probes" + count)
Each probe is a card or list row:
- Status dot + display name (`probe_name`, or first 8 chars of `probe_id` + "…").
- Secondary line: `hostname · site` (use "—" for missing values).
- Right side: status pill, and "▲ up / ▼ down" counts.
- Footer: "Last seen 2 min ago".
- Tap → Probe Detail.
- If the company has exactly one probe, you may open Probe Detail directly or expand the probe inline.

Empty: "No probes found for this company." Poll every 15 s (probes and uptime).

### 6.4 Probe Detail
Header: back, probe display name, status pill.

**Info card:** Probe ID (full, monospace, tap-to-copy), Hostname, Site, Status, Last seen (relative + absolute), Devices up/down.

If expired → amber banner: "⚠ Probe expired — device states may be outdated."

**Devices list** (Down first, then A→Z):
Each row:
- Type icon (see §4.2) + device **name** (bold) + `target` (secondary, monospace, truncated).
- Right: UP/DOWN pill. Rows that are down get a faint red background tint.
- Third line: latency ("42ms") or metric/progress bar, plus `message`.
- Bell-slash icon if the device is muted.
- Tap → Device Detail sheet.
- Swipe left → **Silence** / **Unmute** action.

Pull-to-refresh; poll every 15 s.

### 6.5 Device Detail (bottom sheet)
- Name, type (human label: "Ping", "HTTP", "TCP Port", "Disk", "CPU", "Memory", "Uptime"), target, status, latency, metric (large, with gauge for %), full message.
- Company / Probe breadcrumb.
- **Alerts section**:
  - If not muted: button **Silence alerts** → opens the Silence form.
  - If muted: shows "🔕 Muted — reason: … — until <date> / Forever", plus button **Unmute** → `POST /api/silence/delete`.

**Silence form** (modal):
- Title "Silence Alerts", "Device: **{name}**".
- Reason (text input, placeholder "e.g. Scheduled maintenance").
- Duration segmented control: **1 hour · 24 hours (default) · 7 days · Forever**, mapping to `1 | 24 | 168 | null`.
- Buttons: Cancel / **Silence** (primary). On success, update the silences cache optimistically and show a toast "Alerts silenced for {name}".

### 6.6 Alerts tab
Two sections:
1. **Problems:** every device currently Down, plus every expired probe, across all companies. Build this client-side by fetching `/api/dashboard/companies`, then probes for companies with problems, then devices for those probes. Group by company. Each row shows device name, company › probe, message, and a mute indicator. Tap → Device Detail.
2. **Active silences:** from `GET /api/silences`. Each row: the parsed resource_key (company › probe › device name), reason, "Expires in 5h" / "Forever", and an **Unmute** button.

Badge on the tab icon = number of down devices that aren't muted.

### 6.7 Settings
- **Hub connection:** URL (read-only), "Connected" status dot, **Switch hub** (goes back to Login).
- **Appearance:** Theme: System / Light / Dark (default System).
- **Refresh interval:** 15 s (default) / 30 s / 60 s / Manual.
- **Storage & Retention** (from `/api/admin/storage-info`): Current DB size, Warning threshold, Retention window ("90 days"), Last archive date / size ("Never"). If `warning` → amber banner "Database exceeds retention threshold". Show DB size as a progress bar against the threshold.
- **Archives** (from `/api/admin/archives`): list of Date created · Data cutoff · Size MB → tap opens **Archive Viewer**.
- **About:** app version.
- **Log out** (destructive-styled) → `POST /api/logout`, clear the session, but keep the saved URL (ask "Forget saved token?").

### 6.8 Archive Viewer
Uses the same Dashboard → Company → Probe screens but fed from `/api/archive/{id}/…` endpoints, with:
- A persistent top banner: "📂 HISTORICAL VIEW — READ ONLY ARCHIVE" (neutral/blue).
- No polling, no silence actions.

---

## 7. Visual design

- Look: a clean, dense "ops console" in the style of Linear, Vercel, or Datadog mobile. Readability first; status colors are the only strong colors.
- **Light theme:** background `#f4f6f8`, cards `#ffffff`, header/nav `#1e293b` (dark slate), text slate-900.
- **Dark theme** (default for many technicians): background `#0f172a`, cards `#1e293b`, text slate-100.
- Cards: 12 px radius, subtle 1 px border (`slate-200` / `slate-700`), no heavy shadows.
- Typography: system font; numbers in stat tiles large (28–32 pt, semibold, tabular figures); IDs, targets, and hostnames in monospace.
- Status pills: uppercase, 11 pt, bold, tinted background (status color at 15% opacity) with status-colored text.
- Status dots 8 px; pulse animation on CRITICAL dots only.
- Haptic feedback on pull-to-refresh and on silence/unmute.
- Minimum touch target 44 pt. Support Dynamic Type and screen readers: status must also be conveyed by text or icon, not color alone.

---

## 8. Behaviour details

- **Polling:** 15 s default on Dashboard, Company, and Probe screens. Pause when the app is backgrounded or the screen isn't focused. Refetch immediately on app foreground.
- **Stale indicator:** if the last successful fetch is more than 60 s old (e.g. network lost), show a thin amber bar "Offline — showing data from 2 min ago" and keep the cached data visible.
- **Relative times:** "just now", "45s ago", "3 min ago", "2 h ago", "Mar 8, 14:02".
- **Probe display name:** `probe_name` if non-empty, else `probe_id.slice(0, 8) + "…"`.
- **URL-encode** `company_id` and `probe_id` in paths.
- **Self-signed HTTPS:** a production build can't trust the hub's self-signed cert out of the box. For this prototype, support `http://` URLs and show a hint on the Login screen if an `https://` connection fails ("Self-signed certificates aren't supported yet — try the HTTP port").
- **Push notifications:** out of scope for v1. Leave a placeholder toggle in Settings labelled "Coming soon".

---

## 9. Mock data (Demo Mode)

Use this data in Demo Mode. Make the timestamps relative to "now" when generating them so the UI looks live. Each poll should randomly jitter latencies and CPU/mem metrics slightly, and occasionally flip one device between Up and Down, so polling and animations are visible.

```json
{
  "companies": [
    { "company_id": "acme-corp",      "total_probes": 2, "active_probes": 2, "expired_probes": 0, "devices_up": 11, "devices_down": 2, "last_report": "<now-20s>" },
    { "company_id": "brightside-dental","total_probes": 1, "active_probes": 0, "expired_probes": 1, "devices_up": 6,  "devices_down": 0, "last_report": "<now-14m>" },
    { "company_id": "northwind-legal", "total_probes": 1, "active_probes": 1, "expired_probes": 0, "devices_up": 7,  "devices_down": 0, "last_report": "<now-35s>" },
    { "company_id": "summit-logistics","total_probes": 3, "active_probes": 3, "expired_probes": 0, "devices_up": 18, "devices_down": 0, "last_report": "<now-8s>" }
  ],
  "probes": {
    "acme-corp": [
      { "probe_id": "a3f9c2e17b4d4e8f9a0b1c2d3e4f5a6b", "probe_name": "acme-hq-probe", "hostname": "ACME-SRV01", "site": "Head Office", "status": "active", "last_seen_at": "<now-20s>", "devices_up": 6, "devices_down": 2 },
      { "probe_id": "b7e1d0c95a3f4b2e8c7d6e5f4a3b2c1d", "probe_name": "", "hostname": "acme-wh-pi", "site": "Warehouse", "status": "active", "last_seen_at": "<now-40s>", "devices_up": 5, "devices_down": 0 }
    ],
    "brightside-dental": [
      { "probe_id": "c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6", "probe_name": "front-desk", "hostname": "BSD-RECEPTION", "site": "Main Clinic", "status": "expired", "last_seen_at": "<now-14m>", "devices_up": 6, "devices_down": 0 }
    ]
  },
  "devices": {
    "a3f9c2e17b4d4e8f9a0b1c2d3e4f5a6b": [
      { "name": "Core Switch",    "resource_type": "ping", "target": "10.0.0.1",                 "status": "Up",   "message": "Responded in 1.2ms", "latency_ms": 1,    "metric_value": null, "metric_unit": null },
      { "name": "File Server NAS","resource_type": "ping", "target": "10.0.0.20",                "status": "Down", "message": "Request timed out",  "latency_ms": null, "metric_value": null, "metric_unit": null },
      { "name": "Intranet",       "resource_type": "http", "target": "http://intranet.acme.local","status": "Up",   "message": "HTTP 200",           "latency_ms": 48,   "metric_value": null, "metric_unit": null },
      { "name": "SQL Server",     "resource_type": "tcp",  "target": "10.0.0.30:1433",           "status": "Up",   "message": "Port Open",          "latency_ms": 3,    "metric_value": null, "metric_unit": null },
      { "name": "Printer - Floor 2","resource_type": "tcp","target": "10.0.0.45:9100",           "status": "Down", "message": "Connection refused", "latency_ms": null, "metric_value": null, "metric_unit": null },
      { "name": "CPU",            "resource_type": "local_cpu",    "target": "localhost", "status": "Up", "message": "CPU usage: 23.4%", "latency_ms": null, "metric_value": 23.4, "metric_unit": "%" },
      { "name": "Memory",         "resource_type": "local_mem",    "target": "localhost", "status": "Up", "message": "Memory: 6.1 GB / 8.0 GB (76.2%)", "latency_ms": null, "metric_value": 76.2, "metric_unit": "%" },
      { "name": "Disk C:",        "resource_type": "local_disk",   "target": "C:\\",      "status": "Up", "message": "C:\\: 212.4 GB / 237.9 GB (89.3%)", "latency_ms": null, "metric_value": 89.3, "metric_unit": "%" },
      { "name": "Uptime",         "resource_type": "local_uptime", "target": "localhost", "status": "Up", "message": "Uptime: 12d 4h 37m", "latency_ms": null, "metric_value": 1053420, "metric_unit": "seconds" }
    ]
  },
  "silences": [
    { "id": 7, "scope_type": "resource", "scope_id": "acme-corp:a3f9c2e17b4d4e8f9a0b1c2d3e4f5a6b:Printer - Floor 2:10.0.0.45:9100", "reason": "Printer being replaced", "created_at": "<now-3h>", "expires_at": "<now+21h>" }
  ],
  "storage": { "current_db_size_mb": 842, "warn_threshold_mb": 1000, "retention_days": 90, "last_archive_date": "2026-06-30T02:00:00Z", "last_archive_size_mb": 311, "warning": false },
  "archives": [
    { "id": 1, "filename": "blentinel_archive_2026-06-30.db", "created_at": "2026-06-30T02:00:00Z", "cutoff_date": "2026-04-01T00:00:00Z", "size_mb": 311 }
  ]
}
```

Generate probes and devices for companies not listed above using the same patterns. Generate uptime buckets per the range table in §4.2: mostly 100%, with a few dips to 70–90% for `acme-corp` and a flat gap (no buckets) for `brightside-dental` over the last 14 minutes.

Implement the data layer behind a single `HubClient` interface with two implementations, `LiveHubClient` and `MockHubClient`, so screens don't know which one they use.

---

## 10. Acceptance checklist

- [ ] Demo Mode works end-to-end with no network.
- [ ] Login with a real hub URL + token; a 401 triggers a silent re-login.
- [ ] Dashboard sort order and card colors follow §3 exactly.
- [ ] Company → Probe → Device drill-down works; expired probe shows the stale-data banner.
- [ ] Uptime chart switches ranges and shows correct %, with a tooltip.
- [ ] Silence / Unmute round-trips and the muted icon updates immediately.
- [ ] Polling every 15 s, paused in background, without flicker or list reordering when data is unchanged.
- [ ] Light/Dark/System themes.
- [ ] No destructive admin actions are exposed.
