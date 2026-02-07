#!/bin/bash
# Quick build script for blentinelmake

echo "Building blentinelmake..."

cargo build -p blentinelmake --release

if [ $? -eq 0 ]; then
    echo ""
    echo "blentinelmake built successfully!"
    echo "Binary location: target/release/blentinelmake"
    echo ""
    echo "To use it:"
    echo "  ./target/release/blentinelmake --help"
    echo "  ./target/release/blentinelmake probe build --release"
    echo "  ./target/release/blentinelmake hub publish"
else
    echo "Build failed!"
    exit 1
fi
