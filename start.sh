#!/bin/bash
# Quick start script for iVnc

set -e

echo "==================================="
echo "iVnc - Quick Start"
echo "==================================="
echo ""

# Check if config exists
if [ ! -f "config.toml" ]; then
    echo "Creating config.toml from example..."
    cp config.example.toml config.toml
    echo "✓ Config file created"
fi

# Check if binary exists
if [ ! -f "target/release/ivnc" ]; then
    echo ""
    echo "Binary not found. Building..."
    ./build.sh --release
fi

echo ""
echo "Starting iVnc..."
echo ""

# Set environment
export DISPLAY=${DISPLAY:-:0}
export IVNC_LOG=${IVNC_LOG:-info}

# Run
./target/release/ivnc --config config.toml
