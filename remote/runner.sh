#!/bin/bash

BASE_DIR="/home/user/blentinel-builder"
REPO_DIR="$BASE_DIR/blentinel"
BRANCH="main"

cd "$REPO_DIR" || exit 1

echo "Blentinel runner started..."

while true; do
    git fetch origin "$BRANCH" >/dev/null 2>&1

    REMOTE_HASH=$(git rev-parse origin/$BRANCH)
    LOCAL_HASH=$(git rev-parse HEAD)

    echo "Updating deploy scripts..."

    cp -f blentinel/remote/deploy_hub.sh deploy_hub.sh
    chmod +x deploy_hub.sh

    if [ "$REMOTE_HASH" != "$LOCAL_HASH" ]; then
        echo "New commit detected."

        git pull origin "$BRANCH" || {
            echo "Git pull failed"
            sleep 20
            continue
        }

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

    sleep 20
done
