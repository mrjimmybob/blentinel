# Audit log

The hub records who did what, when, and from which address. **Audit** in the menu (Owner and Admin) shows the log, newest first, with filters and a CSV export. The same is available from the API ([reference/api.md](../reference/api.md#audit-log)).

## What is recorded

| Action | When | Target |
|---|---|---|
| `login.success`, `login.failed`, `login.logout` | a login (a failed one keeps the username that was tried), a logout | |
| `setup.complete`, `setup.failed` | the first Owner created with the setup code, a wrong setup code | user |
| `account.password_change`, `account.logout_others` | someone changed their own password or logged out their other sessions | user |
| `user.create`, `user.update`, `user.reset_password`, `user.force_logout` | user management, in the UI or with `hub user` on the server | user |
| `company.create`, `site.create` | a company or site was added | company, site |
| `code.create`, `code.revoke` | an enrollment code was created (Add probe) or revoked; the code itself is never recorded | code |
| `probe.enroll` | a probe enrolled with a code (done by the probe; no user) | probe |
| `probe.update`, `probe.revoke` | a probe was renamed or moved to another site, or revoked | probe |
| `check.create`, `check.update`, `check.delete`, `check.copy` | a probe's checks added, changed (the entry keeps before and after), deleted, or copied from another probe | check, probe |
| `command.queue`, `command.cancel` | a command queued for a probe (e.g. Run checks now; clicking again while it waits isn't recorded twice) or cancelled before it was sent (D-56) | probe |
| `company.update`, `company.archive`, `company.unarchive`, `site.update` | company and site edits | company, site |
| `silence.create`, `silence.delete` | alerts silenced (a check, a probe, a site or a company) or unmuted | silence (details: `scope`) |
| `maintenance.create`, `maintenance.delete` | a maintenance window added or removed (D-65) | maintenance (details: `scope`, `reason`, `timezone`, `schedule`) |
| `data.delete_company`, `data.delete_probe`, `probe.remove` | data deleted or a probe removed on the admin page | company, probe |
| `archive.create` | an archive created (by hand; the automatic archive isn't a person's action) | archive |
| `rotation.start` | a rotation of the hub's certificate and/or key started | rotation (details: `tls`, `key`, `window_days`) |
| `rotation.switch` | the hub switched to its new certificate/key: by a person (**Switch now**) or by the hub when every probe was ready | rotation (details: `probes_not_ready`, `codes_revoked`) |
| `rotation.cancel` | a rotation cancelled | rotation |
| `certificate.settings` | the HTTPS certificate settings saved (Let's Encrypt or another ACME authority, D-62) | (details: `enabled`, `domains`, `directory`, `challenge`) |
| `certificate.issue` | a certificate issued: when the settings were saved, by **Renew now**, or by the hub when one was due | (details: `certificates`, 2 while a rotation's next one is issued too) |
| `certificate.issue_failed` | the authority didn't issue one | (details: `error`, its reason) |
| `incident.acknowledge`, `incident.note` | an incident acknowledged, or a note added (D-67) | incident (details: `title`, `note`) |
| `alerts.settings` | the mail server or the hub's address saved (D-66) | (details: the settings, `password_changed`; never the password) |
| `channel.create`, `channel.update`, `channel.delete` | an alert channel added, changed or removed | channel (details: `name`, `kind`, `enabled`, `target`, `secret_changed`; never the secret) |
| `alerts.routes` | the routing rules saved | (details: `rules`) |
| `thresholds.update` | the thresholds of the hub, a company or a site changed (a check's are part of `check.update`) | thresholds (`hub`), company or site (details: what was set) |
| `backup.create` | **Back up now** (the daily backup isn't a person's action) | backup (its file name) |
| `backup.delete` | a backup deleted | backup |
| `backup.download` | a backup downloaded (the password was given again) | backup |
| `backup.download_failed` | a wrong password when downloading a backup | backup |
| `backup.schedule` | the backup schedule changed | (details: `enabled`, `hour_utc`, `keep`) |
| `release.add`, `release.approve`, `release.remove` | a probe release added on the admin page, approved for probes (`none`: updates stopped), or removed (D-55). `hub release add` on the server isn't audited | release (its version) |

Each entry also has the **company** it concerns, if any, and **details**: the values that were set (e.g. a user's role and companies, a silence's reason). Passwords, temporary passwords and session tokens are never recorded.

Entries made by the hub itself, or by `hub user` on the server console, show **hub** as the user.

## Filtering and export

- **Username**, **action group** (Logins, Users, Companies, …), **target** (a company, user or probe ID; also matches the company of an entry), and a **date range** (UTC days).
- **Load older** pages back through the log.
- **Export CSV** downloads what the filters select (up to 1000 entries per file), for a spreadsheet or your records.

## Keeping it

The log is kept in the hub's database and never changes. It isn't copied into archives, which hold monitoring history only. It grows by a few rows per action, so it stays small. Deleting old entries comes with data purge (ROADMAP M1-10).

If the log can't be written (e.g. the disk is full), the action still happens and the hub logs an error. An audit problem never locks anyone out.
