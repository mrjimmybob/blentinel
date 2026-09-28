# Users and roles

Everyone who uses the web UI has their own account with a role. The hub keeps accounts and login sessions in its database. There is no shared admin password.

## First run: the setup code

A new hub has no users. When it starts, it creates a one-time **setup code**. It prints the code to its log and writes it to `setup.code` in its `state_dir`, readable only by the hub's user:

```sh
sudo cat /var/lib/blentinel/setup.code              # Linux service
Get-Content C:\ProgramData\Blentinel\Hub\setup.code  # Windows
```

Open the web UI. It sends you to **Set up Blentinel**. Enter the code and choose the first **Owner**'s username and password. You are logged in, and the code and its file are gone.

The code stops someone else from claiming a freshly installed hub that is already reachable from the network. A restart before setup creates a new code.

## Roles

| Role | Can |
|---|---|
| **Owner** | Everything, including managing Owners. At least one active Owner always remains. |
| **Admin** | Everything except managing Owners: users, companies and sites, the admin page, deleting data, archives |
| **Technician** | View everything; silence and unmute alerts |
| **Viewer** | View everything (read-only) |
| **Client** | Not available yet: a read-only view of their own companies, once company scoping exists (ROADMAP M1-04) |

The menu shows only what your role can do. The hub checks every request anyway: a Viewer who calls the API to silence an alert gets `403 forbidden`. The permission of each API route is listed in [reference/api.md](../reference/api.md).

For now every user sees every company. Limiting a user to some companies comes with M1-04. Until then the hub refuses to create such users, so nobody gets access that only looks limited.

## Managing users

**Users** in the menu (Owner and Admin):

- **Add a user:** username, optional display name, and role. The hub creates a **temporary password** and shows it once. Pass it on; the user must choose their own password when they first log in.
- **Edit:** display name, role, or **Active**. A deactivated user can't log in, and their sessions end immediately. A role change applies to their next click.
- **Reset password:** a new temporary password, shown once. The user's sessions end.
- **Log out:** ends all of the user's sessions.

Only an Owner can create, edit or reset Owners. You can't deactivate yourself, and the last active Owner can't be demoted or deactivated.

## Your account

Click your name in the header (**My account**):

- **Change password.** Your other sessions end; this one stays.
- **Log out other sessions**, for example after using a shared computer.

Passwords need at least 12 characters. The hub stores only an argon2id hash.

**Too many wrong passwords** lock that username from that computer for a while: 5 attempts are free, then 30 seconds, doubling up to 15 minutes. The login page says how long to wait. Many wrong attempts from one address with different usernames lock that address for 15 minutes. A successful login clears the count; restarting the hub clears all counts.

## Sessions

Logging in creates a session. It ends after **12 hours without use** or **7 days after login**, whichever comes first, or when you log out. Sessions are stored in the database, so they survive a hub restart. Only a hash of the session cookie is stored.

## Locked out: `hub user`

On the server, with the same `--config` as the running hub (the hub can keep running):

```sh
hub --config /etc/blentinel/blentinel_hub.toml user list
hub --config /etc/blentinel/blentinel_hub.toml user reset-password alice
hub --config /etc/blentinel/blentinel_hub.toml user create rescue --role owner
```

`create` and `reset-password` print a temporary password once; it must be changed at the next login. `reset-password` also ends the user's sessions. These commands need access to the database file. That is the break-glass path: whoever can run them could read the database anyway. See [reference/cli.md](../reference/cli.md).
