"""Fails if anything under ops/ looks like private infrastructure (D-105).

The repository is public; the inventory, keys and secrets stay on lab-control.
This catches the usual slips: a real address, an Ansible Vault file, a private
key. Allowed IPv4 literals: the documentation ranges (RFC 5737), loopback, and
the private ranges written as whole networks (the firewall's own list).

Run from the repository's root: python3 ops/ci/guard.py
"""

import ipaddress
import pathlib
import re
import sys

DOCUMENTATION = [ipaddress.ip_network(n) for n in ("192.0.2.0/24", "198.51.100.0/24", "203.0.113.0/24")]
LOOPBACK = ipaddress.ip_network("127.0.0.0/8")
WHOLE_RANGES = {"10.0.0.0/8", "100.64.0.0/10", "127.0.0.0/8", "169.254.0.0/16", "172.16.0.0/12", "192.168.0.0/16"}
IPV4 = re.compile(r"(?<![\d.])(\d{1,3}(?:\.\d{1,3}){3})(/\d{1,2})?(?![\d.])")
# Split, so this file doesn't match itself.
SECRETS = {
    "$ANSIBLE_" + "VAULT": "an Ansible Vault file",
    "PRIVATE " + "KEY-----": "a private key",
}


def allowed(address: str, prefix: str | None) -> bool:
    if prefix and address + prefix in WHOLE_RANGES:
        return True
    try:
        ip = ipaddress.ip_address(address)
    except ValueError:
        return True  # not an address (a version number, say)
    return ip.is_unspecified or ip in LOOPBACK or any(ip in n for n in DOCUMENTATION)


def main() -> int:
    problems = []
    for path in sorted(pathlib.Path("ops").rglob("*")):
        if not path.is_file():
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for number, line in enumerate(text.splitlines(), 1):
            for match in IPV4.finditer(line):
                if not allowed(match.group(1), match.group(2)):
                    problems.append(f"{path}:{number}: address {match.group(0)}: use a documentation address (RFC 5737)")
            for marker, what in SECRETS.items():
                if marker in line:
                    problems.append(f"{path}:{number}: looks like {what}")
    for problem in problems:
        print(f"::error::{problem}")
    if problems:
        print("Private addresses, keys and secrets belong in the inventory on lab-control, not in Git (docs/ops.md).")
        return 1
    print("ops/: no private addresses, keys or secrets.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
