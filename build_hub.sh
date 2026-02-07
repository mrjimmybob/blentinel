#!/usr/bin/env bash
set -e

SHOW_HELP=false
RELEASE=false
WATCH=false
CLEAN=false

for arg in "$@"; do
  case "$arg" in
    --help) SHOW_HELP=true ;;
    --release) RELEASE=true ;;
    --watch) WATCH=true ;;
    --clean) CLEAN=true ;;
  esac
done

if $SHOW_HELP; then
  echo "Leptos Hub Build Script"
  echo "======================="
  echo ""
  echo "Usage: ./build_hub.sh [options]"
  echo ""
  echo "Options:"
  echo "  --release     Build in release mode"
  echo "  --watch       Watch for changes"
  echo "  --clean       Clean build artifacts"
  echo "  --help        Show this help"
  exit 0
fi

# Ensure cargo-leptos exists
if ! cargo leptos --help >/dev/null 2>&1; then
  echo "cargo-leptos not found. Installing..."
  cargo install cargo-leptos
fi

if $CLEAN; then
  echo "Cleaning HUB..."
  rm -rf target/front target/site
  cargo clean -p hub
  echo "Clean complete."
  exit 0
fi

ARGS=(leptos build)

$RELEASE && ARGS+=(--release)
$WATCH && ARGS+=(--watch)

echo "Running: cargo ${ARGS[*]}"
START=$(date +%s)

cargo "${ARGS[@]}"

END=$(date +%s)
echo "Build completed in $((END - START)) seconds"
