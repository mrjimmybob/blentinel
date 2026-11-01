#!/bin/sh
# Check that staging's probes are reporting on the new version (proposal 0013 §10.4 E).
# Called by the agent after the hub smoke tests pass.
#
# Usage: smoke-probes.sh <hub-url> <tls-pin> <expected-version> <timeout-seconds>
#
# Exit 0 if all probes are reporting on the expected version; 1 otherwise.

set -u

HUB_URL=$1
TLS_PIN=$2
EXPECTED_VERSION=$3
TIMEOUT=$4

deadline=$(($(date +%s) + TIMEOUT))

while :; do
    problem=

    # Ask the hub for the list of probes
    probes=$(curl -sS -k --pinnedpubkey "$TLS_PIN" --max-time 10 \
        "$HUB_URL/api/probes" 2>/dev/null || true)

    if [ -z "$probes" ]; then
        problem="can't reach the hub's probes API"
    else
        # Check if any probes exist
        count=$(echo "$probes" | grep -c '"id"' || true)
        if [ "$count" -eq 0 ]; then
            problem="no probes reporting yet"
        else
            # Check if all probes are on the expected version
            bad=$(echo "$probes" | grep -o '"version":"[^"]*"' | grep -v "\"$EXPECTED_VERSION\"" | wc -l || true)
            if [ "$bad" -gt 0 ]; then
                problem="$bad probe(s) on wrong version"
            fi
        fi
    fi

    if [ -z "$problem" ]; then
        echo "All $count probes reporting on $EXPECTED_VERSION" >&2
        return 0
    fi

    if [ "$(date +%s)" -ge "$deadline" ]; then
        echo "Probe smoke tests failed for ${TIMEOUT}s: $problem" >&2
        return 1
    fi

    sleep 5
done
