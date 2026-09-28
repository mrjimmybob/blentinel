# 0001 · Users, Roles & Access Control

| | |
|---|---|
| **Status** | Accepted. §4.1–4.4 and the Users / My account UI implemented (M1-01, M1-02; D-42). Company scoping done (M1-04, D-45); HTTPS/CSRF/rate limiting done (M1-05, D-43). Open: audit log (M1-06), API tokens (M1-07), TOTP (M1-08), active-session list |
| **Milestone** | M1 — Accounts & Access |
| **Priority** | P0 |
| **Depends on** | M0 (migrations, test harness, layered hub) |
| **Decisions** | D-03, D-06 |

## 1. Problem

The hub has a **single shared admin token** (`hub_auth.token`):

- Anyone with the token is an all-powerful admin. There's no way to give a client read-only access, or a junior technician limited access.
- There's no record of **who** did what (deleted data, silenced an alert).
- Sessions are held only in memory (`hub/src/auth.rs`), so every hub restart logs everyone out.
- Revoking one person's access means rotating the token for everyone.
- The mobile app or scripts would have to share the admin token.

## 2. Goals / Non-goals

**Goals:** username and password login; roles; per-company scoping; persistent sessions; per-user API tokens; audit log; first-run setup without the console token dance.

**Non-goals (for now):** SSO/OIDC/SAML (P3, future), LDAP/AD sync (P3), fine-grained custom roles.

## 3. Principle check

| | |
|---|---|
| FP-1 push-only | Not affected (hub ↔ browser only) |
| FP-2 zero-dep probe | Not affected |
| FP-3 zero-trust | Strengthened (per-user identity, audit) |
| PP-1/PP-2 | First-run wizard replaces copying the console token |
| PP-3 | SQLite only, no external IdP required |

## 4. Design

### 4.1 Roles

| Role | Scope | Can |
|---|---|---|
| **Owner** | Global | Everything, including managing Admins and hub settings. At least one must exist. |
| **Admin** | Global | Manage users (except Owners), companies, sites, probes, checks, alert channels, retention |
| **Technician** | All companies, or assigned ones | View; silence/ack/maintenance; edit checks on assigned companies |
| **Viewer** | All companies, or assigned ones | Read-only (internal staff) |
| **Client** | Assigned companies only | Read-only view of their own company; no internal notes, no other companies, no hub-wide stats |

Permissions are an enum (`Permission::ViewCompany`, `EditChecks`, `ManageUsers`, `Silence`, …). There is **one** authorization function:

```rust
fn authorize(user: &User, perm: Permission, scope: Scope) -> Result<(), AuthzError>
```

It is called by every handler through an Axum extractor (`RequireUser` + `.require(perm, scope)`). Every list query is filtered by the user's company scope in the **repository layer**, so a Client can never receive another company's data, even through an aggregate endpoint.

### 4.2 Data model (migration)

```sql
users (
  id TEXT PRIMARY KEY,                -- UUID
  username TEXT NOT NULL UNIQUE COLLATE NOCASE,
  display_name TEXT NOT NULL,
  email TEXT,
  password_hash TEXT NOT NULL,        -- argon2id PHC string
  role TEXT NOT NULL,                 -- owner|admin|technician|viewer|client
  all_companies INTEGER NOT NULL DEFAULT 0,
  is_active INTEGER NOT NULL DEFAULT 1,
  must_change_password INTEGER NOT NULL DEFAULT 0,
  totp_secret_enc BLOB,               -- phase 2
  created_at DATETIME NOT NULL,
  last_login_at DATETIME
);
user_companies (user_id, company_id, PRIMARY KEY(user_id, company_id));
sessions (
  id_hash TEXT PRIMARY KEY,           -- SHA-256 of the cookie value; raw value never stored
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at, expires_at, last_seen_at, ip TEXT, user_agent TEXT
);
api_tokens (
  id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL, prefix TEXT NOT NULL, secret_hash TEXT NOT NULL,
  created_at, last_used_at, expires_at
);
audit_log (
  id INTEGER PRIMARY KEY, at DATETIME NOT NULL, user_id TEXT, actor_ip TEXT,
  action TEXT NOT NULL,               -- e.g. 'silence.create', 'probe.revoke', 'user.login_failed'
  target_type TEXT, target_id TEXT, details TEXT  -- JSON
);
```

### 4.3 Authentication

- **Passwords:** argon2id (default parameters from the `argon2` crate) and a minimum of 12 characters. Common-password checking is optional.
- **Login:** `POST /api/auth/login {username, password}` → rotates the session ID and sets a cookie `blentinel_session` with `HttpOnly; Secure; SameSite=Strict; Path=/`. Sliding expiry: 12 h idle, 7 days absolute (configurable).
- **Rate limiting:** exponential backoff per `(ip, username)` after 5 failures. Failures are written to `audit_log`.
- **CSRF:** `SameSite=Strict` plus mutating requests must carry a custom header (`X-Blentinel: 1`), which the UI always sends.
- **API tokens:** `Authorization: Bearer blt_<prefix>_<secret>`. Only the hash is stored. The token is shown once at creation. They are used by the mobile app, scripts, and PSA integrations. A token inherits its user's role and scope. Optional read-only flag.
- **TOTP 2FA:** phase 2 (P2), per user, can be enforced per role.
- **Sessions survive restarts** (stored in the DB). Expired sessions are cleaned up by a periodic task.

### 4.4 First run and recovery

- On start with **no users**, the hub logs a one-time **setup code** (random, 8 chars) and the UI redirects to `/setup`. Entering the code lets you create the Owner. The code prevents someone else claiming a freshly installed, internet-exposed hub.
- CLI recovery on the server (break-glass, needs filesystem access):
  - `hub user create <username> --role owner`
  - `hub user reset-password <username>`
  - `hub user list`
- **Migration from the token:** the existing `hub_auth.token` file is ignored after upgrade. The setup wizard runs if no users exist. `--reset-admin-token` is removed (release note).

### 4.5 UI

- **Login page:** username/password, "remember me" (absolute expiry).
- **Users** (Admin+): list, invite/create, role, company assignment, deactivate, reset password, force logout.
- **My account:** change password, active sessions (revoke), API tokens (create/revoke), 2FA (phase 2).
- **Audit log** (Admin+): filter by user, action, target, date; CSV export.
- The header shows the current user and role. Menu items are hidden when the user lacks the permission, and the API enforces it regardless.

### 4.6 API changes

- All existing `/api/*` endpoints move behind `RequireUser` with explicit permissions.
- Destructive admin endpoints require `Admin` and write to the audit log.
- New: `/api/auth/*`, `/api/users/*`, `/api/me/*`, `/api/audit`.

## 5. Tests

- Unit: `authorize()` matrix for every role × permission × scope.
- Unit: password hashing and verification; token generation/hash/lookup; setup-code single use.
- Integration: a 401/403 matrix over **every** route (generated from the router so new routes can't be missed); Client-scope leakage tests on the dashboard, uptime, and search endpoints; rate-limit behaviour; session survives a restart; session revoked on deactivate.
- E2E: first-run wizard → login → logout.

## 6. Docs to update

README quick start (setup wizard instead of token), `docs/install/hub.md`, `docs/guide/users.md` (new), `docs/security.md`, `docs/reference/api.md` (auth), CHANGELOG (breaking: token login removed).

## 7. Open questions

1. Should Clients see uptime **percentages only**, or also device-level detail? **Decided (D-45):** device-level detail of their own companies, without internal notes and contacts.
2. E-mail invitations need SMTP. Fall back to "admin sets a temporary password" when SMTP isn't configured? (Proposed: yes.)
