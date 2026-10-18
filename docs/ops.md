# Ops: the lab's machines as code

The machines behind the release pipeline, `lab-control` and staging, are built and kept by the Ansible playbooks in [`ops/`](../ops/). This is phase 3 of [proposal 0013](proposals/0013-ci-cd.md) (D-105). With playbooks, a machine can be rebuilt from nothing, and nothing changes on it that isn't in a reviewed pull request.

> **Status:** being built. This page describes what is in `ops/` now: the `base` and `firewall` roles, for both machines. The `lab_control` and `staging` roles and the bootstrap script come next; Blentinel itself (the staging hub) is installed in phase 4.

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
| `lab-control` | Debian 13, minimal | `base`; a firewall that lets in SSH from your PC only, and lets out only DNS, NTP, HTTP(S) to public addresses (GitHub, the mirrors) and SSH to staging |
| staging | Ubuntu 26.04 | `base`; a firewall that lets in SSH from your PC and `lab-control` |

The playbooks run **on `lab-control`** (Ansible connects to itself locally and to staging over SSH). Production (the VPS) and every other machine are in no inventory, and `lab-control` has no key for them; its egress filter couldn't reach them anyway.

**`base`:**
- **The admin account:** the one the OS installer created (`ops` by default) must exist. It's in the `sudo` group, still with its password, and its `authorized_keys` holds exactly the inventory's keys; any other key is removed.
- **sshd:** keys only, no root, only the listed users. The drop-in is `/etc/ssh/sshd_config.d/10-ops.conf`, read before the distribution's own files: sshd keeps the first value it reads, and Ubuntu's `50-cloud-init.conf` turns passwords on. sshd checks the configuration before it reloads.
- **The system:** the clock in UTC, the journal kept on disk (500 MB at most), security updates installed daily (`unattended-upgrades`).

**`firewall`:**
- **Its own table:** an nftables table, `inet ops`, loaded at boot by `ops-firewall.service`. Each change replaces that table in one transaction, after `nft -c` checked it. It never runs `flush ruleset`, so rules it doesn't own (Docker's, say) stay.
- **What it lets in:** connections already open, ICMP, DHCP, and SSH from `firewall_ssh_sources`.

## Set up `lab-control` (first time)

1. **Install** Debian 13 from the netinst image, with only "SSH server" and "standard system utilities". Create the admin account `ops` with a password, add it to `sudo` (`su -c 'adduser ops sudo'`), and install `xe-guest-utilities` on XCP-ng. Give the machine a fixed address (a DHCP reservation).
2. **Install the tools:** `sudo apt install ansible git`. Debian's `ansible` brings `ansible-core` and the collections the roles use, with security updates from Debian; nothing comes from pip or Galaxy.
3. **Make the key it reaches staging with:** `ssh-keygen -t ed25519 -f ~/.ssh/blentinel_staging`. Add its `.pub` to `ops`'s `authorized_keys` on staging (the playbooks keep it there from then on, through the inventory).
4. **Get the playbooks:** `git clone https://github.com/mrjimmybob/blentinel.git ~/blentinel`.
5. **Write the inventory** (below).

## The inventory (private)

The inventory says which machines exist, their addresses, who may log in and what the firewall allows. **It is never committed:** the repository is public. It lives on `lab-control`:

```sh
sudo install -d -o ops -g ops -m 0700 /etc/blentinel-ops /etc/blentinel-ops/inventory
cp ~/blentinel/ops/inventory.example/hosts.yml /etc/blentinel-ops/inventory/hosts.yml
nano /etc/blentinel-ops/inventory/hosts.yml   # your addresses and public keys
```

`ops/ansible.cfg` points there, so the commands below need no `-i`. Fill in, for each machine:

| Variable | What |
|---|---|
| `ansible_host`, `ansible_user`, `ansible_ssh_private_key_file` | how to reach staging; `lab-control` uses `ansible_connection: local` |
| `ops_environment` | `lab_control` or `staging`: each play refuses a host that says otherwise, so a host in the wrong group fails before anything changes |
| `base_admin_keys` | **every** public key that may log in as `ops`; the others are removed. On staging, include `lab-control`'s key |
| `firewall_ssh_sources` | the addresses that may connect to SSH: your PC; on staging also `lab-control` |
| `firewall_egress_filter`, `firewall_egress_dns`, `firewall_egress_ssh` | `lab-control` only: turn the egress filter on, your DNS server(s), staging's address |
| `firewall_verify_blocked` | `lab-control` only: a LAN address and port it must never reach (a production machine is a good choice); `verify.yml` checks it can't |

**Keep an encrypted copy of `/etc/blentinel-ops/` somewhere other than `lab-control`** (a password manager's file attachment will do). A rebuild of `lab-control` needs it, and it is the only thing that isn't in the repository.

**There are no secrets in it yet:** no passwords, no tokens. Ansible Vault arrives in phase 4 with the first real secret, the GitHub token of the deployment agent.

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

1. Revert the machine to its "fresh install" snapshot (for `lab-control`, keep your copy of `/etc/blentinel-ops/` at hand, and follow the set-up steps above).
2. `ansible-playbook site.yml --ask-become-pass`.
3. `ansible-playbook verify.yml --ask-become-pass`: every check passes.
4. `ansible-playbook site.yml --ask-become-pass` again: `changed=0` on every machine.

## Locked out?

The firewall lets SSH in only from the listed addresses, and sshd takes keys only. If a mistake locks you out:

1. Open the machine's console in Xen Orchestra (XCP-ng) and log in as `ops` with its password.
2. `sudo systemctl stop ops-firewall` removes the table `inet ops`: everything is open again until the next boot or apply.
3. Fix the inventory (or the change in `ops/`), and apply again.

An SSH session that is already open survives a firewall change (the table lets established connections through), so test a new rule by opening a **second** session before you close the first.

## How the playbooks are tested

[`.github/workflows/ops.yml`](../.github/workflows/ops.yml) runs on every pull request and push that changes `ops/`, on GitHub-hosted runners, without access to the LAN. First [`ops/ci/guard.py`](../ops/ci/guard.py) refuses a private address (anything but the RFC 5737 documentation ranges), an Ansible Vault file or a private key anywhere in `ops/`. Then [`ops/ci/converge.sh`](../ops/ci/converge.sh):

1. **Lint:** `yamllint`, `ansible-lint` (production profile) and a syntax check, with the Ansible from Debian 13's packages, the version `lab-control` runs.
2. **Fresh machines:** two containers with systemd and sshd, Debian 13 as `lab-control` and Ubuntu 26.04 as staging, on a Docker network with documentation addresses (`ops/ci/inventory.yml`). A third container is the controller. As on the real machines, Ansible logs in over SSH as `ops`, and sudo asks for a password (made for the run).
3. **Apply twice:** the first run builds the machines; the **second run must report `changed=0`** on both. A playbook that changes something every time it runs isn't describing a state.
4. **Check mode must report `changed=0` too**, which makes `--check` trustworthy for drift.
5. **`verify.yml`** checks the result, including that `lab-control`'s egress filter refuses a connection it must refuse.

The same script runs on your machine with Docker: `bash ops/ci/converge.sh` from the repository's root. It's not a required check: it runs only when `ops/` changes.

What a container can't show: the clock's synchronisation (skipped there, checked on the real machines by `verify.yml`), and a real reboot loading the firewall.
