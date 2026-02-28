#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BASE_DIR="$SCRIPT_DIR"
REPO_DIR="$BASE_DIR/blentinel"
BRANCH="main"

cd "$REPO_DIR" || exit 1

echo "Blentinel runner started..."
git fetch origin "$BRANCH" >/dev/null 2>&1

REMOTE_HASH=$(git rev-parse origin/$BRANCH)
LOCAL_HASH=$(git rev-parse HEAD)

if [ "$REMOTE_HASH" != "$LOCAL_HASH" ]; then

    echo "New commit detected."

    git pull origin "$BRANCH" || {
            echo "Git pull failed"
            sleep 20
            continue
    }

    CHANGED_FILES=$(git diff --name-only "$LOCAL_HASH" "$REMOTE_HASH")
    # CHANGED_FILES=$(git diff --name-only HEAD@{1} HEAD)

    changed() {
        grep -q "$1" <<< "$CHANGED_FILES"
    }

    if changed "remote/runner.sh"; then
        echo "Runner update detected. Deploying new runner script..."
        cp -f "$REPO_DIR/remote/runner.sh" "$BASE_DIR/runner.sh"
        chmod +x "$BASE_DIR/runner.sh"

        echo "Restarting blentinel runner service..."
        sudo systemctl restart blentinel-runner
        exit 0
    fi

    if changed "remote/deploy_hub.sh"; then
        echo "Deploy script updated. Deploying new deploy_hub script..."
        cp -f "$REPO_DIR/remote/deploy_hub.sh" "$BASE_DIR/deploy_hub.sh"
        chmod +x "$BASE_DIR/deploy_hub.sh"
    fi

    echo "Building build tool..."
    chmod u+x ./build_blentinelmake.sh
    ./build_blentinelmake.sh || {
        echo "Build tool failed"
        sleep 20
        continue
    }

    echo "Publishing hub..."
    ./target/release/blentinelmake hub publish || {
        echo "Publish failed"
        sleep 20
        continue
    }

    echo "Deploying to VPS..."
    "$BASE_DIR/deploy_hub.sh" || {
        echo "Deploy failed"
        sleep 20
        continue
    }

    echo "Pipeline finished successfully."
fi
