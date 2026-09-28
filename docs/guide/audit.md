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
| `company.update`, `company.archive`, `company.unarchive`, `site.update` | company and site edits | company, site |
| `silence.create`, `silence.delete` | alerts silenced or unmuted | silence |
| `data.delete_company`, `data.delete_probe`, `probe.remove` | data deleted or a probe removed on the admin page | company, probe |
| `archive.create` | an archive created (by hand; the automatic archive isn't a person's action) | archive |

Each entry also has the **company** it concerns, if any, and **details**: the values that were set (e.g. a user's role and companies, a silence's reason). Passwords, temporary passwords and session tokens are never recorded.

Entries made by the hub itself, or by `hub user` on the server console, show **hub** as the user.

## Filtering and export

- **Username**, **action group** (Logins, Users, Companies, …), **target** (a company, user or probe ID; also matches the company of an entry), and a **date range** (UTC days).
- **Load older** pages back through the log.
- **Export CSV** downloads what the filters select (up to 1000 entries per file), for a spreadsheet or your records.

## Keeping it

The log is kept in the hub's database and never changes. It isn't copied into archives, which hold monitoring history only. It grows by a few rows per action, so it stays small. Deleting old entries comes with data purge (ROADMAP M1-10).

If the log can't be written (e.g. the disk is full), the action still happens and the hub logs an error. An audit problem never locks anyone out.
