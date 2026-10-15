# Command-line reference

The text below is each program's `--help` output. If you change a flag, update this page in the same change.

## hub

```
Usage: hub [OPTIONS]
       hub [OPTIONS] <install | upgrade | uninstall | status | doctor | backup | restore>
       hub [OPTIONS] release <add <folder | file.tar.gz> | list>
       hub [OPTIONS] user <COMMAND>

Blentinel Hub — central collection and storage node.
Receives encrypted reports from probes, verifies signatures,
and persists data to the local database.

Options:
  -h, --help               Print this help message and exit
      --version            Print version and exit
      --init               Create a default blentinel_hub.toml and exit
      --config <path>      Path to the hub configuration TOML file (default:
                           blentinel_hub.toml in the working directory, else
                           the installed hub's)
      --print-public-key   Print the hub public key to stdout and write
                           hub_identity.pub, then exit (no server start)
  -v, --verbose            Debug logging: also one line per report received
  -d, --debug              Trace logging: also every accepted report in clear text
                           (implies --verbose; never enable in production)

Running the hub on this server:
  install [--host <ip>] [--port <n>] [--https-port <n>] [--no-service]
                     Install (or upgrade) as a service: copies itself into place,
                     creates the config (first time only; listens on 0.0.0.0,
                     3000 and 3443 unless given), restricts the data folder,
                     starts the service and prints the address and setup code.
                     Needs root / Administrator.
  upgrade [--from <url | folder>]
                     Upgrade the installed hub to a newer release: checks its
                     signature, runs the new hub's self-test on a copy of the
                     database, backs up, switches, and puts the old hub and
                     database back if the new one doesn't keep running. From
                     [upgrades] release_url (GitHub) unless --from. Adds the
                     release's probes. Needs root / Administrator.
  self-test          Check that this hub can run here: reads the config and
                     key, upgrades a copy of the database (`upgrade` runs it)
  uninstall [--delete-data]
                     Remove the service and program. Keeps the data and config
                     unless --delete-data. Needs root / Administrator.
  status             Service, ports, database, probes, certificate expiry
  doctor             Check config, permissions, keys, database, certificate,
                     ports and disk space, with what to do about problems
  backup [--out <file>]
                     Database (while the hub runs), keys, certificate and
                     config in one .tar.gz (default: the backups folder,
                     [backups] path, <data>/backups/ unless set)
  restore <file>     Put a backup back (the hub must be stopped; the files it
                     replaces are moved aside)
  release add <folder | file.tar.gz>
                     Add a signed probe release (`blentinelmake release build`);
                     the admin page approves it for probes to update to
  release list       The probe releases on this hub, newest first

User accounts (break-glass, on the server; works while the hub runs):
  user list                              List users
  user create <username> --role <role> [--display-name <name>]
                                         Create a user with a temporary password
                                         (printed once; changed at first login).
                                         Roles: owner, admin, technician, viewer
                                         (all companies; Clients: in the web UI)
  user reset-password <username>         New temporary password; logs the user out

The log goes to stdout (under systemd: journalctl -u blentinel-hub).
RUST_LOG overrides the flags, e.g. RUST_LOG=hub_lib=debug,sqlx=warn
```

`--debug` prints every accepted report in clear text. Don't use it in production.

`hub release add` and `release list` manage the signed probe releases the hub offers ([install/probe.md](../install/probe.md#updates)); `release add` also takes a signed rollback's folder. Rolling one out (canaries, company policies) is done on the admin page. `hub install`, `upgrade`, `uninstall`, `status`, `doctor`, `backup` and `restore` manage the hub on its server: see [install/hub.md](../install/hub.md) (`upgrade`: [Upgrading](../install/hub.md#upgrading)). `self-test` is what `upgrade` runs on the new hub before switching; it changes nothing (the database it upgrades is a copy). `install`, `upgrade` and `uninstall` need root/Administrator; the others read the data folder, which is private to the service, so run them as root/Administrator too. `doctor` exits with `1` if a check fails.

`hub user …` is for account recovery on the server; the web UI's **Users** page is the normal way. It opens the database from the config (use the same `--config` as the running hub; the hub can keep running). Details: [guide/users.md](../guide/users.md#locked-out-hub-user).

## probe

```
Usage: probe [OPTIONS]
       probe <install | uninstall | status | doctor>
       probe enroll <CODE>
       probe updates <on | off>

Blentinel Probe — silent network sentinel.
Runs the checks its hub gives it and ships encrypted reports to the hub.
Checks and the report interval are managed on the hub; the probe keeps
the last ones it received (config.json) for when the hub is unreachable.

Commands:
  install [--code <CODE>]
                  Install (or upgrade) as a service: copies itself into its
                  folder, enrolls with the code from the hub's "Add probe"
                  (a new code enrolls it again in place), starts the service.
                  Needs root / Administrator.
  uninstall [--delete-data]
                  Remove the service and the program. Keeps the identity and
                  enrollment unless --delete-data (then nothing is left).
  status          Service, hub, checks, reports waiting to be sent
  doctor          Check the folder, identity, enrollment, the hub's reachability
                  (through its pinned certificate) and the clock
  enroll <CODE>   Join a hub, then exit, without installing: writes
                  enrollment.json next to the probe.
  updates off     The local emergency brake: refuse every update the hub
                  offers (it shows the refusal) until `probe updates on`.
                  The probe updates itself only to releases signed with a
                  key built into it, newer than itself (see `status`).

Options:
  -h, --help      Print this help message and exit
      --version   Print version and exit
  -v, --verbose   Log operational events to the terminal
                  (monitoring results, Hub connectivity)
  -d, --debug     Log cleartext payloads before encryption
                  (implies --verbose; never enable in production)
      --self-test Check this binary can run the probe in its folder, change
                  nothing, and exit (the updater runs it on a new version)

On Windows the probe auto-detects whether it was launched by the Service
Control Manager and runs as a service in that case, or as a normal CLI
process otherwise.  No flag is required for either mode.
```

`install`, `uninstall`, `status`, `doctor` and `updates` manage the probe on its machine: see [install/probe.md](../install/probe.md). Run them as root/Administrator (the probe's folder is private). `doctor` exits with `1` if a check fails. On Windows the probe detects whether the Service Control Manager started it and runs as a service in that case; no flag is needed. The probe has no config file and no `--init` (D-49): what it checks is managed on the hub. `--self-test` is what the probe runs on a downloaded version before switching to it (D-55); it changes nothing.

## blentinelmake (development)

```
Usage: blentinelmake <component> <action> [OPTIONS]

Components:  hub | probe
Actions:     build | publish | clean
Options:
  --target <triple>    Target triple for cross-compilation (probe only)
  --release            Build in release mode (build action only)
  --with-probes <triple,...>
                       hub publish: also build these probes, signed, into probes/
                       in the package; `hub install` adds them as a release
                       (Add probe offers them; needs a release key)
  -h, --help / --version

       blentinelmake release keygen
       blentinelmake release build --targets <triple,...>
       blentinelmake release sign <folder> [--version <v>]
       blentinelmake release rollback --to <version> --from <version,...>
       blentinelmake release hub --targets <triple,...> --probes <triple,...>
       blentinelmake docs [--tex]
```

`release` makes signed probe releases (D-55): `keygen` creates the release key pair once, in `$BLENTINEL_RELEASE_KEY_DIR` or else `blentinel/release-keys` in the user's configuration folder (`%APPDATA%` on Windows, `~/.config` elsewhere). The private key stays there; every hub and probe built by `blentinelmake` afterwards trusts the public keys of every `.pub` file in that folder (passed to the build as `BLENTINEL_RELEASE_KEYS`). `build` builds the probes for the given targets and signs them as the workspace version: `publish/probe-release-<version>/` and its `.tar.gz`. `sign` signs `probe-<target>` binaries already in a folder. `rollback` signs permission for probes running a `--from` version to go back to the older release `--to` (probes never downgrade otherwise, D-74): `publish/probe-rollback-<to>/` and its `.tar.gz`, added to the hub like a release. `hub` makes the files of a GitHub release for the one-line install (D-57), on Linux: the hub for each target (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`; needs `musl-tools` and the Rust target), the probes for `--probes` as a signed release (`probes.tar.gz`), `hub.sh` with the release public keys in it, `release.json` (the version, which `hub upgrade` and the admin page's check read), and `SHA256SUMS` with its signature (`SHA256SUMS.sig`, raw Ed25519), in `publish/github-release-<version>/`.

Run it without arguments for an interactive menu. Examples are in [CONTRIBUTING.md](../../CONTRIBUTING.md#build).
