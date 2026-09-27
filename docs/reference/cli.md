# Command-line reference

The text below is each program's `--help` output. If you change a flag, update this page in the same change.

## hub

```
Usage: hub [OPTIONS]

Options:
  -h, --help               Print this help message and exit
      --version            Print version and exit
      --init               Create a default blentinel_hub.toml and exit
      --config <path>      Path to the hub configuration TOML file
                           (default: blentinel_hub.toml in the working directory)
      --print-public-key   Print the hub public key to stdout and write
                           hub_identity.pub, then exit (no server start)
      --reset-admin-token  Generate a new admin token, write hub_auth.token,
                           and print it to stdout, then exit (no server start)
  -v, --verbose            Log operational events to the terminal
                           (probe reports received, data saved to DB)
  -d, --debug              Verbose mode + detailed diagnostic output
                           (implies --verbose; never enable in production)
```

`--debug` prints every accepted report in clear text. Don't use it in production.

## probe

```
Usage: probe [OPTIONS]

Options:
  -h, --help      Print this help message and exit
      --version   Print version and exit
      --init      Create a default blentinel_probe.toml and exit
  -v, --verbose   Log operational events to the terminal
                  (handshake, monitoring results, Hub connectivity)
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
