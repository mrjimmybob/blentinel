#!/bin/sh
# Check that staging's probes are reporting on the new version (proposal 0013 §10.4 E).
# Called by the agent after the hub smoke tests pass, if probes are enrolled.
#
# Usage: smoke-probes.sh <hub-url> <tls-pin> <expected-version> <timeout-seconds> [--optional]
#
# Exit codes:
#   0 = all enrolled probes reporting on the expected version (or --optional and no probes)
#   1 = probes enrolled but not reporting correctly (BLOCKS deployment)
#   75 = can't reach the hub or other transient issue (retry next run)

set -u

HUB_URL=$1
TLS_PIN=$2
EXPECTED_VERSION=$3
TIMEOUT=$4
OPTIONAL="${5:-}"

deadline=$(($(date +%s) + TIMEOUT))

while :; do
    problem=

    # Ask the hub for the list of probes
    probes=$(curl -sS -k --pinnedpubkey "$TLS_PIN" --max-time 10 \
        "$HUB_URL/api/probes" 2>/dev/null || true)

    if [ -z "$probes" ]; then
        problem="can't reach the hub's probes API"
        # Transient issue, not a deployment blocker
        if [ "$(date +%s)" -ge "$deadline" ]; then
            echo "Can't reach hub for ${TIMEOUT}s" >&2
            exit 75
        fi
    else
        # Check if any probes exist
        count=$(echo "$probes" | grep -c '"id"' || true)
        if [ "$count" -eq 0 ]; then
            if [ "$OPTIONAL" = "--optional" ]; then
                # No probes enrolled yet; this is OK during initial setup
                echo "No probes enrolled yet (optional)" >&2
                exit 0
            else
                problem="no probes enrolled"
                # Without --optional, missing probes block deployment
                if [ "$(date +%s)" -ge "$deadline" ]; then
                    echo "No probes enrolled after ${TIMEOUT}s" >&2
                    exit 1
                fi
            fi
        else
            # Check if all probes are on the expected version
            bad=$(echo "$probes" | grep -o '"version":"[^"]*"' | grep -v "\"$EXPECTED_VERSION\"" | wc -l || true)
            if [ "$bad" -gt 0 ]; then
                problem="$bad of $count probe(s) on wrong version"
                # Wrong version blocks deployment
                if [ "$(date +%s)" -ge "$deadline" ]; then
                    echo "Probes not ready after ${TIMEOUT}s: $problem" >&2
                    exit 1
                fi
            else
                # All probes on correct version
                echo "All $count probes reporting on $EXPECTED_VERSION" >&2
                exit 0
            fi
        fi
    fi

    sleep 5
done
