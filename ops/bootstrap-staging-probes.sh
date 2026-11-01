#!/bin/bash
# Bootstrap the staging probes: create an enrollment code for them to use
# (proposal 0013 §10.4 E). Run this once after staging's hub is installed
# to set up the infrastructure for probes to report to the staging hub.
#
# Usage: ./bootstrap-staging-probes.sh <hub-url> <admin-token> <company-id> <site-id>
#
# The company and site should already exist on the staging hub. Get the token
# from the hub's admin account setup.

set -eu

if [ $# -lt 4 ]; then
    echo "Usage: $0 <hub-url> <admin-token> <company-id> <site-id>" >&2
    echo "Example: $0 https://staging-hub.local:3443 token123 acme-corp hq" >&2
    exit 1
fi

HUB_URL=$1
ADMIN_TOKEN=$2
COMPANY_ID=$3
SITE_ID=$4
PROBE_NAME="${5:-staging-probe}"

echo "Creating enrollment code on $HUB_URL for probes..." >&2

# Create the enrollment code via the hub's API
RESPONSE=$(curl -s -X POST "$HUB_URL/api/enrollment-codes" \
    -H "Authorization: Bearer $ADMIN_TOKEN" \
    -H "Content-Type: application/json" \
    -d @- <<EOF
{
  "company_id": "$COMPANY_ID",
  "site_id": "$SITE_ID",
  "probe_name": "$PROBE_NAME",
  "max_uses": null
}
EOF
)

# Extract the code from the response
CODE=$(echo "$RESPONSE" | grep -o '"code":"[^"]*"' | cut -d'"' -f4 || true)

if [ -z "$CODE" ]; then
    echo "Failed to create enrollment code. Response: $RESPONSE" >&2
    exit 1
fi

echo "SUCCESS: Created enrollment code: $CODE" >&2
echo "$CODE"
