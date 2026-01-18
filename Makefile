.PHONY: help build build-release build-websocket test clean install check fmt clippy

# Default target
help:
	@echo "Selkies Core - Makefile"
	@echo ""
	@echo "Available targets:"
	@echo "  make build           - Build in debug mode (WebRTC)"
	@echo "  make build-release   - Build in release mode (WebRTC)"
	@echo "  make build-websocket - Build WebSocket-only mode"
	@echo "  make build-vaapi     - Build with VA-API support"
	@echo "  make build-nvenc     - Build with NVENC support"
	@echo "  make test            - Run all tests"
	@echo "  make check           - Check code without building"
	@echo "  make fmt             - Format code"
	@echo "  make clippy          - Run clippy linter"
	@echo "  make clean           - Clean build artifacts"
	@echo "  make install         - Install to /usr/local/bin"

# Build targets
build:
	cargo build

build-release:
	cargo build --release

build-websocket:
	cargo build --release --no-default-features --features websocket-legacy

build-vaapi:
	cargo build --release --features vaapi

build-nvenc:
	cargo build --release --features nvenc

build-qsv:
	cargo build --release --features qsv

# Test targets
test:
	cargo test

test-websocket:
	cargo test --no-default-features --features websocket-legacy

# Code quality
check:
	cargo check --all-features

fmt:
	cargo fmt

clippy:
	cargo clippy --all-features -- -D warnings

# Clean
clean:
	cargo clean

# Install
install: build-release
	sudo cp target/release/selkies-core /usr/local/bin/
	@echo "Installed to /usr/local/bin/selkies-core"
