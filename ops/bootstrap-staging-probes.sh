#!/bin/bash
# Helper to document setting up staging probes (proposal 0013 §10.4 E).
# Currently, enrollment codes must be created manually via the web UI, as API
# tokens (M1-07) are not yet implemented. This script documents the process
# and provides a template for future automation.
#
# Manual setup steps:
# 1. Open https://<staging>:3443 in your browser
# 2. Login with the admin account
# 3. Companies → staging → Add probe
# 4. Create an enrollment code (or use an existing one)
# 5. Copy the code (BLT1-…) and add it to the inventory
#
# Once API tokens are implemented (M1-07), this script can automate code
# creation like this (for reference):
#
#   curl -X POST "$HUB_URL/api/enrollment-codes" \
#     -H "Authorization: Bearer $API_TOKEN" \
#     -H "Content-Type: application/json" \
#     -d '{
#       "company_id": "staging",
#       "site_id": "staging",
#       "probe_name": "staging-probe",
#       "max_uses": null
#     }'

set -eu

echo "Phase 4 E: Staging probes setup" >&2
echo "" >&2
echo "Manual setup required (API tokens not yet implemented in M1-07):" >&2
echo "1. Open https://<staging>:3443 in your browser" >&2
echo "2. Login as admin" >&2
echo "3. Companies → staging → Add probe" >&2
echo "4. Create an enrollment code and copy it (BLT1-…)" >&2
echo "5. Add to /etc/blentinel-ops/inventory/hosts.yml:" >&2
echo "   probe_staging_code: BLT1-…" >&2
echo "6. Run: ansible-playbook site.yml --ask-become-pass --ask-vault-pass" >&2
echo "" >&2
