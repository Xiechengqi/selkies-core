#!/bin/bash
# Quick start script for Selkies Core

set -e

echo "==================================="
echo "Selkies Core - Quick Start"
echo "==================================="
echo ""

# Check if config exists
if [ ! -f "config.toml" ]; then
    echo "Creating config.toml from example..."
    cp config.example.toml config.toml
    echo "✓ Config file created"
fi

# Check if binary exists
if [ ! -f "target/release/selkies-core" ]; then
    echo ""
    echo "Binary not found. Building..."
    ./build.sh --release
fi

echo ""
echo "Starting Selkies Core..."
echo ""

# Set environment
export DISPLAY=${DISPLAY:-:0}
export SELKIES_LOG=${SELKIES_LOG:-info}

# Run
./target/release/selkies-core --config config.toml
