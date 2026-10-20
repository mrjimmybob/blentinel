# Ops: the lab's machines as code

The machines behind the release pipeline, `lab-control` and staging, are built and kept by the Ansible playbooks in [`ops/`](../ops/). This is phase 3 of [proposal 0013](proposals/0013-ci-cd.md) (D-105). With playbooks, a machine can be rebuilt from nothing, and nothing changes on it that isn't in a reviewed pull request.

> **Status:** built, and applied to both machines (`verify.yml` passes; a second run and check mode report `changed=0`). **Not done:** the machines were prepared by hand and then converged; the phase's test, rebuilding both from fresh VMs with this page ([below](#rebuild-from-nothing-the-phases-test)), is outstanding. The roles: `base` and `firewall` for both machines, `lab_control` (with `bootstrap.sh`), and `staging`. Blentinel itself (the staging hub) is installed in phase 4.

## The rule: a person applies configuration

| | Configuration (this page) | Deployment (phase 4) |
|---|---|---|
| What | users, packages, firewall, SSH | a new Blentinel version on staging |
| Applied by | **you**, with `ansible-playbook`, typing sudo's password | a timer on `lab-control` |
| From | a checkout of `ops/` you updated on purpose | a release whose signature was verified |

Nothing runs the playbooks by itself: no timer, no CI job against a real machine. A change pushed to `ops/` does nothing until you read it and apply it (0013 §10.1). The sudo password you type (`--ask-become-pass`) is what enforces this: it is stored nowhere.

## The machines

| Machine | OS | What the playbooks give it |
|---|---|---|
| `lab-control` | Debian 13, minimal | `base`; a firewall that lets in SSH from your PC only, and lets out only DNS, NTP, HTTP(S) to public addresses (GitHub, the mirrors) and SSH to staging; `lab_control` |
| staging | Ubuntu 26.04 | `base`; `staging`; a firewall that lets in SSH from your PC and `lab-control`, and the hub's ports from `staging_hub_sources` |

On staging, Ansible becomes root with `sudo.ws`, the classic sudo, not the system's default `sudo-rs`, whose prompt Ansible doesn't recognise ([`ops/group_vars/staging.yml`](../ops/group_vars/staging.yml), D-105). Both read the same `/etc/sudoers`, and the password is the same.

The playbooks run **on `lab-control`** (Ansible connects to itself locally and to staging over SSH). Production (the VPS) and every other machine are in no inventory, and `lab-control` has no key for them; its egress filter couldn't reach them anyway.

**`base`:**
- **The admin account:** the one the OS installer created (`ops` by default) must exist. It's in the `sudo` group, still with its password, and its `authorized_keys` holds exactly the inventory's keys; any other key is removed.
- **sshd:** keys only, no root, only the listed users. The drop-in is `/etc/ssh/sshd_config.d/10-ops.conf`, read before the distribution's own files: sshd keeps the first value it reads, and Ubuntu's `50-cloud-init.conf` turns passwords on. sshd checks the configuration before it reloads.
- **The system:** the clock in UTC, the journal kept on disk (500 MB at most), security updates installed daily (`unattended-upgrades`).

**`firewall`:**
- **Its own table:** an nftables table, `inet ops`, loaded at boot by `ops-firewall.service`. Each change replaces that table in one transaction, after `nft -c` checked it. It never runs `flush ruleset`, so rules it doesn't own (Docker's, say) stay.
- **What it lets in:** connections already open, ICMP, DHCP, SSH from `firewall_ssh_sources`, and on staging the hub's ports.

**`staging`** (staging only) readies the hub's machine, without installing the hub: phase 4 does that with `hub.sh`, because Ansible calls Blentinel's installers rather than doing their work (0013 §10.3).
- **What `hub.sh` needs:** `curl`, `openssl`, `sha256sum`, `tar` and the CA certificates ([install/hub.md](install/hub.md)).
- **The hub's ports**, as a first `hub install` sets them: 3000 (plain HTTP, for probes) and 3443 (HTTPS: the web UI, and probes), open to `staging_hub_sources` only ([`ops/group_vars/staging.yml`](../ops/group_vars/staging.yml)). An empty list is refused: no browser or probe could reach the hub. Until phase 4 nothing listens there, so a connection from a source is refused at once, and from anywhere else it gets no answer.

**`lab_control`** (`lab-control` only) keeps what [`ops/bootstrap.sh`](../ops/bootstrap.sh) made:
- Ansible and Git, from Debian's packages (security updates come with the system's).
- `/etc/blentinel-ops/` and its `inventory/`, owned by the admin account, mode 0700.
- The key to staging, `~/.ssh/blentinel_staging`, mode 0600. The role checks it exists but never makes one: a new key would lock `lab-control` out of staging until the inventory listed it.

## Set up `lab-control` (first time)

1. **Install** Debian 13 from the netinst image, with only "SSH server" and "standard system utilities". Create the admin account `ops` with a password, add it to `sudo` (`su -c 'adduser ops sudo'`), and install `xe-guest-utilities` on XCP-ng. Give the machine a fixed address (a DHCP reservation).
2. **Get the playbooks:**
   ```sh
   sudo apt install git
   git clone https://github.com/mrjimmybob/blentinel.git ~/blentinel
   ```
3. **Bootstrap:** `sudo sh ~/blentinel/ops/bootstrap.sh`. It installs Ansible (Debian's `ansible` brings `ansible-core` and the collections the roles use; nothing comes from pip or Galaxy), makes the inventory's folder with a copy of the example in it, and makes the key `lab-control` reaches staging with. It prints that key's public half. Running it again changes nothing: it never replaces a key or an inventory.
4. **Write the inventory** (below), with that key in staging's `base_admin_keys`.
5. **Let the key into staging once:** `ssh-copy-id -i ~/.ssh/blentinel_staging ops@<staging>` (staging's fresh install still takes passwords). This also records staging's host key. From the first apply on, the inventory keeps the key there.
6. **Back up the inventory and the key**, encrypted, off `lab-control` ([below](#the-inventory-private)). Do it again whenever you change the inventory.

## The inventory (private)

The inventory says which machines exist, their addresses, who may log in and what the firewall allows. **It is never committed:** the repository is public. It lives on `lab-control`, in `/etc/blentinel-ops/inventory/hosts.yml`, which only `ops` can read (`bootstrap.sh` makes it from [the example](../ops/inventory.example/hosts.yml)).

`ops/ansible.cfg` points there, so the commands below need no `-i`. Fill in, for each machine:

| Variable | What |
|---|---|
| `ansible_host`, `ansible_user`, `ansible_ssh_private_key_file` | how to reach staging; `lab-control` uses `ansible_connection: local` |
| `ops_environment` | `lab_control` or `staging`: each play refuses a host that says otherwise, so a host in the wrong group fails before anything changes |
| `base_admin_keys` | **every** public key that may log in as `ops`; the others are removed. On staging, include `lab-control`'s key |
| `firewall_ssh_sources` | the addresses that may connect to SSH: your PC; on staging also `lab-control` |
| `staging_hub_sources` | staging only: the addresses that may reach the hub (ports 3000 and 3443): your PC; the lab's probe machines as they join (phase 5) |
| `firewall_egress_filter`, `firewall_egress_dns`, `firewall_egress_ssh` | `lab-control` only: turn the egress filter on, your DNS server(s), staging's address |
| `firewall_verify_blocked` | `lab-control` only: a LAN address and port it must never reach (a production machine is a good choice); `verify.yml` checks it can't |

**Keep an encrypted copy of two things somewhere other than `lab-control`** (a password manager's file attachments will do). They are the only things a rebuild of `lab-control` needs that aren't in the repository:
- **`/etc/blentinel-ops/`**, the inventory.
- **`~/.ssh/blentinel_staging` and its `.pub`**, `lab-control`'s key to staging. **This one is a private key:** whoever has it can log into staging as `ops`. Keep it encrypted, and never in Git. Without it, a rebuilt `lab-control` gets a new key from `bootstrap.sh`, and staging, which takes keys only, doesn't know that key. You'd then have to change the inventory and add the new key on staging from its console. With the backup, `lab-control` can be rebuilt alone, and staging needs no change.

**There are no secrets in the inventory yet:** no passwords, no tokens. Ansible Vault arrives in phase 4 with the first real secret, the GitHub token of the deployment agent.

## Apply a change

On `lab-control`:

```sh
cd ~/blentinel
git fetch
git diff HEAD origin/main -- ops/       # read what would change
git merge --ff-only origin/main
cd ops
ansible-playbook site.yml --check --diff --ask-become-pass   # what it would do
ansible-playbook site.yml --ask-become-pass                  # do it
ansible-playbook verify.yml --ask-become-pass                # check the result
```

`--limit staging` or `--limit lab_control` applies to one machine. A run on a machine that is already right reports `changed=0`.

## Check for drift

`ansible-playbook site.yml --check --diff --ask-become-pass` changes nothing and shows what differs from the playbooks. `changed=0` means the machines are as the playbooks say. CI proves that check mode is accurate (below), so anything else is real drift: someone changed the machine by hand, or a package changed a file. Fix it in `ops/` and apply, or apply to put the machine back.

## Rebuild from nothing (the phase's test)

Phase 3 is done when you rebuild both machines from nothing with the playbooks (0013 §15). With an XCP-ng snapshot of each machine's fresh install:

1. **Save the inventory and the key to staging first.** They aren't in the repository, and reverting `lab-control` deletes them. From your PC:
   ```sh
   mkdir lab-control-backup
   scp -r ops@<lab-control>:/etc/blentinel-ops lab-control-backup/
   scp "ops@<lab-control>:.ssh/blentinel_staging*" lab-control-backup/
   ```
   Keep the folder encrypted, as above: it holds a private key.
2. **Revert both machines** to their "fresh install" snapshots in Xen Orchestra.
3. **Set up `lab-control`** (the steps above). Put the key back *before* `bootstrap.sh`, so it keeps that key instead of making a new one. On `lab-control`:
   ```sh
   sudo apt install git
   git clone https://github.com/mrjimmybob/blentinel.git ~/blentinel
   install -d -m 0700 ~/.ssh
   ```
   From your PC: `scp lab-control-backup/blentinel_staging lab-control-backup/blentinel_staging.pub ops@<lab-control>:.ssh/`. Then on `lab-control`:
   ```sh
   chmod 0600 ~/.ssh/blentinel_staging
   sudo sh ~/blentinel/ops/bootstrap.sh
   ```
   `bootstrap.sh` should print the saved key, the one in staging's `base_admin_keys`. Then put the saved inventory back. From your PC: `scp lab-control-backup/blentinel-ops/inventory/hosts.yml ops@<lab-control>:/etc/blentinel-ops/inventory/`. If you have no saved key, `bootstrap.sh` makes a new one: replace lab-control's line in staging's `base_admin_keys` with the key it prints.
4. **Let `lab-control` into staging:** `ssh-copy-id -i ~/.ssh/blentinel_staging ops@<staging>`, accepting staging's host key.
5. **Apply and check,** in `~/blentinel/ops`:
   ```sh
   ansible-playbook site.yml --check --diff --ask-become-pass   # the preview works on fresh machines
   ansible-playbook site.yml --ask-become-pass                  # builds both
   ansible-playbook verify.yml --ask-become-pass                # every check passes
   ansible-playbook site.yml --ask-become-pass                  # changed=0 on both
   ```
6. **From your PC:** a new SSH session to each machine works, and a password login is refused (`ssh -o PubkeyAuthentication=no ops@<machine>` says `Permission denied (publickey)`).

## Locked out?

The firewall lets SSH in only from the listed addresses, and sshd takes keys only. If a mistake locks you out:

1. Open the machine's console in Xen Orchestra (XCP-ng) and log in as `ops` with its password.
2. `sudo systemctl stop ops-firewall` removes the table `inet ops`: everything is open again until the next boot or apply.
3. Fix the inventory (or the change in `ops/`), and apply again.

**A firewall change can cut an SSH session that is already open,** along with the playbook run it carries. A session usually survives, because the table lets established connections through, but only while connection tracking knows the connection.
- **When tracking doesn't know it,** the session's next packet counts as a new connection. This happens the first time a firewall is loaded on the machine, or after the tracking state is lost. The packet is then let through only if a rule admits new connections like it.
- D-105 found this on `lab-control`'s first load: its egress filter cut the admin's session. There is now a rule for sshd's replies to `firewall_ssh_sources`.
- A session from an address that isn't (or no longer is) in `firewall_ssh_sources` can still be cut this way.

So, when a change touches the firewall or SSH:
- Keep the console in Xen Orchestra at hand.
- Test the change from a **second** session before you close the first.
- If a run died with its session, log in again and apply again. A run that dies loses the steps it had queued for the end, such as sshd's reload, and only another run does them.

## How the playbooks are tested

[`.github/workflows/ops.yml`](../.github/workflows/ops.yml) runs on every pull request and push that changes `ops/`, on GitHub-hosted runners, without access to the LAN. First [`ops/ci/guard.py`](../ops/ci/guard.py) refuses a private address (anything but the RFC 5737 documentation ranges), an Ansible Vault file or a private key anywhere in `ops/`. Then [`ops/ci/converge.sh`](../ops/ci/converge.sh):

1. **Lint:** `yamllint`, `ansible-lint` (production profile), `shellcheck` and a syntax check, with the Ansible from Debian 13's packages, the version `lab-control` runs.
2. **Fresh machines, set up as above:** two containers with systemd and sshd, Debian 13 as `lab-control` and Ubuntu 26.04 as staging, on a Docker network with documentation addresses. A third container is your PC. `bootstrap.sh` sets up `lab-control` (run twice: the second time must keep the key), [`ops/ci/inventory.yml`](../ops/ci/inventory.yml) becomes its inventory, and its key is let into staging. Then the PC logs into `lab-control` over SSH and runs each playbook there, as you do: Ansible configures `lab-control` locally and staging over SSH, and sudo asks for a password (made for the run).
3. **Preview, then apply twice:** a check-mode run on the fresh machines must work (the preview you run before applying); the first real run builds the machines; the **second run must report `changed=0`** on both. A playbook that changes something every time it runs isn't describing a state.
4. **Check mode must report `changed=0` too**, which makes `--check` trustworthy for drift.
5. **`verify.yml`** checks the result, including that `lab-control`'s egress filter refuses a connection it must refuse.
6. **The hub's ports from the PC:** 3000 and 3443 on staging are refused at once (the firewall let the connection through, and nothing listens yet), and another port gets no answer (dropped).

The same script runs on your machine with Docker: `bash ops/ci/converge.sh` from the repository's root. It's not a required check: it runs only when `ops/` changes.

What a container can't show: the clock's synchronisation (skipped there, checked on the real machines by `verify.yml`), and a real reboot loading the firewall.
