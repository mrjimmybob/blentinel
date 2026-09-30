# Command-line reference

The text below is each program's `--help` output. If you change a flag, update this page in the same change.

## hub

```
Usage: hub [OPTIONS]
       hub [OPTIONS] <install | uninstall | status | doctor | backup | restore>
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
  uninstall [--delete-data]
                     Remove the service and program. Keeps the data and config
                     unless --delete-data. Needs root / Administrator.
  status             Service, ports, database, probes, certificate expiry
  doctor             Check config, permissions, keys, database, certificate,
                     ports and disk space, with what to do about problems
  backup [--out <file>]
                     Database (while the hub runs), keys, certificate and
                     config in one .tar.gz (default: <data>/backups/)
  restore <file>     Put a backup back (the hub must be stopped; the files it
                     replaces are moved aside)

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

`hub install`, `uninstall`, `status`, `doctor`, `backup` and `restore` manage the hub on its server: see [install/hub.md](../install/hub.md). `install` and `uninstall` need root/Administrator; the others read the data folder, which is private to the service, so run them as root/Administrator too. `doctor` exits with `1` if a check fails.

`hub user …` is for account recovery on the server; the web UI's **Users** page is the normal way. It opens the database from the config (use the same `--config` as the running hub; the hub can keep running). Details: [guide/users.md](../guide/users.md#locked-out-hub-user).

## probe

```
Usage: probe [OPTIONS]
       probe <install | uninstall | status | doctor>
       probe enroll <CODE>

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

Options:
  -h, --help      Print this help message and exit
      --version   Print version and exit
  -v, --verbose   Log operational events to the terminal
                  (monitoring results, Hub connectivity)
  -d, --debug     Log cleartext payloads before encryption
                  (implies --verbose; never enable in production)

On Windows the probe auto-detects whether it was launched by the Service
Control Manager and runs as a service in that case, or as a normal CLI
process otherwise.  No flag is required for either mode.
```

`install`, `uninstall`, `status` and `doctor` manage the probe on its machine: see [install/probe.md](../install/probe.md). Run them as root/Administrator (the probe's folder is private). `doctor` exits with `1` if a check fails. On Windows the probe detects whether the Service Control Manager started it and runs as a service in that case; no flag is needed. The probe has no config file and no `--init` (D-49): what it checks is managed on the hub.

## blentinelmake (development)

```
Usage: blentinelmake <component> <action> [OPTIONS]

Components:  hub | probe
Actions:     build | publish | clean
Options:
  --target <triple>    Target triple for cross-compilation (probe only)
  --release            Build in release mode (build action only)
  --with-probes <triple,...>
                       hub publish: also build these probes into probes/ in the
                       package; `hub install` offers them for download (Add probe)
  -h, --help / --version
```

Run it without arguments for an interactive menu. Examples are in [CONTRIBUTING.md](../../CONTRIBUTING.md#build).
