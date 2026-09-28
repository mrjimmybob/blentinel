# Command-line reference

The text below is each program's `--help` output. If you change a flag, update this page in the same change.

## hub

```
Usage: hub [OPTIONS]
       hub [OPTIONS] user <COMMAND>

Blentinel Hub — central collection and storage node.
Receives encrypted reports from probes, verifies signatures,
and persists data to the local database.

Options:
  -h, --help               Print this help message and exit
      --version            Print version and exit
      --init               Create a default blentinel_hub.toml and exit
      --config <path>      Path to the hub configuration TOML file
                           (default: blentinel_hub.toml in the working directory)
      --print-public-key   Print the hub public key to stdout and write
                           hub_identity.pub, then exit (no server start)
  -v, --verbose            Debug logging: also one line per report received
  -d, --debug              Trace logging: also every accepted report in clear text
                           (implies --verbose; never enable in production)

User accounts (break-glass, on the server; works while the hub runs):
  user list                              List users
  user create <username> --role <role> [--display-name <name>]
                                         Create a user with a temporary password
                                         (printed once; changed at first login).
                                         Roles: owner, admin, technician, viewer
  user reset-password <username>         New temporary password; logs the user out

The log goes to stdout (under systemd: journalctl -u blentinel-hub).
RUST_LOG overrides the flags, e.g. RUST_LOG=hub_lib=debug,sqlx=warn
```

`--debug` prints every accepted report in clear text. Don't use it in production.

`hub user …` is for account recovery on the server; the web UI's **Users** page is the normal way. It opens the database from the config (use the same `--config` as the running hub; the hub can keep running). Details: [guide/users.md](../guide/users.md#locked-out-hub-user).

## probe

```
Usage: probe [OPTIONS]
       probe enroll <CODE>

Commands:
  enroll <CODE>   Join a hub with the code from its "Add probe" (web UI),
                  then exit. Writes enrollment.json next to the probe; run
                  it again with a new code to move to another hub or company.

Options:
  -h, --help      Print this help message and exit
      --version   Print version and exit
      --init      Create a default blentinel_probe.toml and exit
  -v, --verbose   Log operational events to the terminal
                  (monitoring results, Hub connectivity)
  -d, --debug     Log cleartext payloads before encryption
                  (implies --verbose; never enable in production)
```

On Windows the probe detects whether the Service Control Manager started it and runs as a service in that case; no flag is needed.

## blentinelmake (development)

```
Usage: blentinelmake <component> <action> [OPTIONS]

Components:  hub | probe
Actions:     build | publish | clean
Options:
  --target <triple>    Target triple for cross-compilation (probe only)
  --release            Build in release mode (build action only)
  -h, --help / --version
```

Run it without arguments for an interactive menu. Examples are in [CONTRIBUTING.md](../../CONTRIBUTING.md#build).
