# Ops: the lab's machines as code

The machines behind the release pipeline, `lab-control` and staging, are built and kept by the Ansible playbooks in [`ops/`](../ops/). This is phase 3 of [proposal 0013](proposals/0013-ci-cd.md) (D-105). With playbooks, a machine can be rebuilt from nothing, and nothing changes on it that isn't in a reviewed pull request.

> **Status:** done (2026-10-21). Both machines were rebuilt from fresh VMs with this page ([below](#rebuild-from-nothing-the-phases-test)): `verify.yml` passed, and a second apply reported `changed=0`. The roles: `base` and `firewall` for both machines, `lab_control` (with `bootstrap.sh`), and `staging`. Phase 4 D adds [the deployment agent](#the-deployment-agent-phase-4-d): `deploy_agent` on `lab-control`, `deploy_target` on staging. It installs the staging hub and keeps it on the newest internal development build.

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
| `lab-control` | Debian 13, minimal | `base`; a firewall that lets in SSH from your PC only, and lets out only DNS, NTP, HTTP(S) to public addresses (GitHub, the mirrors), SSH to staging and staging's hub port; `lab_control`; `deploy_agent` |
| staging | Ubuntu 26.04 | `base`; `staging`; `deploy_target`; a firewall that lets in SSH from your PC and `lab-control`, and the hub's ports from `staging_hub_sources` |

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
| `firewall_egress_tcp` | `lab-control` only: more ports it may reach, each on its hosts: staging's 3443, for the agent's smoke tests |
| `blentinel_staging_keys` | under `all: vars`: the staging public key(s), in hex, which builds must be signed with. Both the agent and staging check with this list |
| `deploy_target_agent_source` | staging only: `lab-control`'s address. The agent's key works from there only. It must also be in `staging_hub_sources` |
| `deploy_agent_github_token` | `lab-control` only, **in Ansible Vault** ([below](#the-deployment-agent-phase-4-d)), never in `hosts.yml` |
| `firewall_verify_blocked` | `lab-control` only: a LAN address and port it must never reach (a production machine is a good choice); `verify.yml` checks it can't |

**Keep an encrypted copy of two things somewhere other than `lab-control`** (a password manager's file attachments will do). They are the only things a rebuild of `lab-control` needs that aren't in the repository:
- **`/etc/blentinel-ops/`**, the inventory.
- **`~/.ssh/blentinel_staging` and its `.pub`**, `lab-control`'s key to staging. **This one is a private key:** whoever has it can log into staging as `ops`. Keep it encrypted, and never in Git. Without it, a rebuilt `lab-control` gets a new key from `bootstrap.sh`, and staging, which takes keys only, doesn't know that key. You'd then have to change the inventory and add the new key on staging from its console. With the backup, `lab-control` can be rebuilt alone, and staging needs no change.

**One secret lives in the inventory:** the deployment agent's GitHub token. It is encrypted with Ansible Vault, in `/etc/blentinel-ops/inventory/host_vars/lab-control/vault.yml` ([below](#the-deployment-agent-phase-4-d)). Back up that file with the rest; without the Vault password it is useless, so keep the password in your password manager, not beside it.

## Apply a change

On `lab-control`:

```sh
cd ~/blentinel
git fetch
git diff HEAD origin/main -- ops/       # read what would change
git merge --ff-only origin/main
cd ops
ansible-playbook site.yml --check --diff --ask-become-pass --ask-vault-pass   # what it would do
ansible-playbook site.yml --ask-become-pass --ask-vault-pass                  # do it
ansible-playbook verify.yml --ask-become-pass --ask-vault-pass                # check the result
```

`--ask-vault-pass` opens the agent's token (below). Ansible asks for it on every run, because it reads the whole inventory. `--limit lab_control` applies to one machine. **Staging is applied together with `lab-control`** (`site.yml`, or `--limit lab_control:staging`): staging takes the agent's public key from `lab-control`'s play in the same run, and refuses to go on without it. A run on machines that are already right reports `changed=0`.

## Check for drift

`ansible-playbook site.yml --check --diff --ask-become-pass --ask-vault-pass` changes nothing and shows what differs from the playbooks. `changed=0` means the machines are as the playbooks say. CI proves that check mode is accurate (below), so anything else is real drift: someone changed the machine by hand, or a package changed a file. Fix it in `ops/` and apply, or apply to put the machine back.

## The deployment agent (phase 4 D)

Every merge to `main` that passes the Build workflow becomes an internal development build, `staging-<version>`, signed with the staging key ([ci.md](ci.md#internal-development-builds-phase-4-c)). The agent on `lab-control` brings staging to the newest one by itself, and GitHub shows the result on the environment `staging` as a Deployment (proposal 0013 §10.2, §10.4 D, D-107). It deploys; it never configures. The playbooks stay yours to apply.

**What a run does** (`blentinel-deploy-agent.timer`, every 5 minutes, as the user `deployer`):
1. **Find the newest build:** asks GitHub for the newest successful push to `main` of the Build workflow that has a `staging-<version>` artifact. It stops if staging has had that version, or a newer one.
2. **Download it** into an empty folder, and unpack it flat: plain files only, size-limited.
3. **Verify it** with the staging key(s) pinned in its settings, never with a key from the download. The check is `verify-release.sh`, the one CI's signing job runs, installed from your checkout. The signed `release.json` must name the run's commit and the artifact's version.
4. **Record it, and create the GitHub Deployment** for that exact commit. The record (0013 §6.4) holds the version, commit, run, the SHA-256 of `SHA256SUMS`, and every file's and probe's SHA-256.
5. **Send it to staging** over SSH, to a key that can only run the forced command. Staging verifies it again with its own pinned key, before anything changes. Then:
   - **no hub yet:** it installs one with the release's `hub.sh` (`BLENTINEL_STAGING=1`);
   - **a hub on another version:** `hub upgrade --from`, which checks the signature a third time and rolls the hub and its database back if the new hub doesn't keep running;
   - then `hub release approve <version>`, once per version.
6. **Smoke tests,** for up to 5 minutes:
   - the hub service is active;
   - its installed program says the version;
   - the **running** process is the release's own hub binary: staging hashes `/proc/<pid>/exe`;
   - the hub answers `/api/setup` on HTTPS, with the TLS key staging says it serves (`curl --pinnedpubkey`).
7. **Report:** the Deployment's status, success or failure with the reason, and one JSON line in `/var/lib/blentinel-deploy-agent/deployments.log`. **Success is reported only after the smoke tests pass.**

**Each step is saved before the next starts,** in `/var/lib/blentinel-deploy-agent/builds/<version>/state`, so a run cut short carries on from there next time:

| State | Meaning |
|---|---|
| `downloaded` | unpacked, not yet verified |
| `verified` | verified; the record is written |
| `deploying` | the Deployment exists; staging was (or is being) sent the release |
| `installed` | staging said it is on the version; smoke tests to come |
| `succeeded` / `failed` | the outcome, still to report to GitHub (`reason` says why it failed) |
| `reported` | GitHub has it: done |

Runs never overlap: systemd starts one at a time, and the agent holds a lock. Each side has one: staging's forced command takes a lock too, and answers "busy" to a second deployment. A Deployment is created once per version: a run cut short after creating it finds it again by commit and version. A deployment that gets no answer from staging is tried again on the next runs, up to 6 times. Staging finishes an upgrade even if the connection drops, and on the next try it finds the hub already on the version.

**When something fails:**
- **Verification:** staging is never contacted, and nothing changes there.
- **`hub upgrade` rolled back:** the old hub and its database are back. GitHub gets a failure saying so, and staging keeps running the previous build.
- **Smoke tests:** the failure says what was wrong. Whatever runs on staging stays.
- **Any of these:** that version is never tried again. The next merge is the fix: roll forward (0013 §13).

### What each side may do

| | Holds | May do |
|---|---|---|
| `deployer` on `lab-control` | the GitHub token (`/etc/blentinel-deploy-agent/github-token`, root-owned, mode 0440, its group only); its SSH key; its state | read GitHub's runs and artifacts, write Deployments; run the forced command on staging; nothing as root |
| `blentinel-deploy` on staging | nothing: its `authorized_keys` is root's, mode 0644 | log in with that one key, from `lab-control`'s address only (`from=`), with no shell, pty or forwarding (`restrict`), to run `ssh-entry`. That runs `deploy` or `status` as root through sudo, with no argument (`""` in sudoers). |

- **The token:**
  - It is fine-grained, for this repository only.
  - It carries only *Actions: read* and *Deployments: read and write*.
  - It goes into a header file in the agent's private runtime folder: never on a command line or in a log.
  - It is never sent to the artifact storage that GitHub redirects downloads to.
- **What the agent runs and trusts is root's:** its script, its settings (with the pinned keys), staging's host key (`known_hosts`, taken from staging by Ansible).
- **Its systemd unit:**
  - It may write only its own state (`ProtectSystem=strict`).
  - It has no privileges and no capabilities.
- **GitHub's own branch rule doesn't guard Deployments:**
  - The environment `staging` accepts only `main`, but that rule applies to workflow jobs (the signing job), not to the Deployments API.
  - Tested on 2026-10-25: a Deployment for a commit only on another branch was accepted.
  - So the agent takes a build only from a successful **push** run on `main`, and only if the signed `release.json` names that run's commit.
  - Each Deployment it creates is for that exact commit, and it checks the commit GitHub recorded.

### Set it up (once)

The steps that only you can do:

1. **Make the GitHub token.** GitHub → Settings → Developer settings → Personal access tokens → **Fine-grained tokens** → Generate new token:
   - **Name:** `blentinel lab-control deployment agent`. **Expiration:** a year, with a reminder to renew it.
   - **Repository access:** *Only select repositories* → `mrjimmybob/blentinel`.
   - **Permissions → Repository permissions:** *Actions:* **Read-only**; *Deployments:* **Read and write**. Nothing else; *Metadata: Read-only* is added by GitHub and stays.
   - Copy the token (`github_pat_…`): GitHub shows it once.
2. **Put it in Ansible Vault,** on `lab-control`, as `ops`:
   ```sh
   install -d -m 0700 /etc/blentinel-ops/inventory/host_vars/lab-control
   ansible-vault create /etc/blentinel-ops/inventory/host_vars/lab-control/vault.yml
   ```
   Choose a Vault password, store it in your password manager, and write this one line in the editor that opens:
   ```yaml
   deploy_agent_github_token: github_pat_…
   ```
   `ansible-vault edit` changes it later, `ansible-vault rekey` changes the password.
3. **Add to the inventory** (`/etc/blentinel-ops/inventory/hosts.yml`; [the example](../ops/inventory.example/hosts.yml) shows where):
   - under `all: vars`: `blentinel_staging_keys: ["<hex of release.pub of the staging key>"]`. This is the value of the repository variable `BLENTINEL_STAGING_KEYS`, quoted.
   - on `lab-control`: `firewall_egress_tcp: [{port: 3443, hosts: [<staging>]}]`;
   - on staging: `deploy_target_agent_source: <lab-control>`, and `<lab-control>` added to `staging_hub_sources`.
4. **Update the checkout and apply,** as in [Apply a change](#apply-a-change): `site.yml --ask-become-pass --ask-vault-pass`, then `verify.yml`. `verify.yml` checks:
   - the token's and the key's permissions;
   - that the timer runs;
   - that staging answers `status` to the agent's key, and refuses anything else.
5. **The first build arrives by itself** within 5 minutes of the next merge to `main`. A staging without a hub gets one from that build, as a fresh install: open `https://<staging>:3443`, and create the first Owner with the setup code in `/var/lib/blentinel/setup.code` on staging. To deploy the newest build at once, without waiting for a merge: `sudo systemctl start blentinel-deploy-agent.service` on `lab-control`.

**Renewing the token:** `ansible-vault edit …/vault.yml`, then apply. **Changing the staging key:** set `blentinel_staging_keys` to the new public key (both, during a change-over), and apply.

### Watch it

On `lab-control`:

```sh
systemctl list-timers blentinel-deploy-agent.timer          # when it runs next
journalctl -u blentinel-deploy-agent -n 50                  # what it did
sudo tail -n 3 /var/lib/blentinel-deploy-agent/deployments.log | jq .   # the records
sudo cat /var/lib/blentinel-deploy-agent/builds/<version>/state       # where a build stands
```

On GitHub: the repository's **Environments → staging** lists each Deployment, with its status and the record as its payload.

- A run that stops with status 75 ran into something passing: GitHub or staging didn't answer, or staging was busy. The next run carries on.
- A run that ends with status 1 reported a failed deployment.
- To try a failed version again anyway, after fixing what was wrong outside the build (staging's disk, say): `sudo rm -r /var/lib/blentinel-deploy-agent/builds/<version>`. It is then a new build to the agent.
- **Staging changed without the agent** (rebuilt from nothing, or a hub installed by hand): each run asks staging for its version first, so the next run notices. If the newest build was deployed well before, the agent puts it back, once, as a new Deployment. A fresh staging gets it as a fresh install. A build that failed waits for the next merge.

## Rebuild from nothing (the phase's test)

This was phase 3's test (0013 §15), passed on 2026-10-21. Repeat it after a change that could break a fresh install, such as a new role, a new OS release, or changes to `bootstrap.sh`. With an XCP-ng snapshot of each machine's fresh install:

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
   ansible-playbook site.yml --check --diff --ask-become-pass --ask-vault-pass   # the preview works on fresh machines
   ansible-playbook site.yml --ask-become-pass --ask-vault-pass                  # builds both
   ansible-playbook verify.yml --ask-become-pass --ask-vault-pass                # every check passes
   ansible-playbook site.yml --ask-become-pass --ask-vault-pass                  # changed=0 on both
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
