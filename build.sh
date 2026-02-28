#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
WEB_DIR="$ROOT/web/ivnc"
DIST_DIR="$WEB_DIR/dist"

# ── Parse arguments ─────────────────────────────────────────
BUILD_MODE="release"
CARGO_FEATURES="mcp"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --debug)
            BUILD_MODE="debug"
            shift
            ;;
        --release)
            BUILD_MODE="release"
            shift
            ;;
        --features)
            CARGO_FEATURES="$CARGO_FEATURES,$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--debug|--release] [--features <features>]"
            exit 1
            ;;
    esac
done

# ── 1. Clean caches ──────────────────────────────────────────
echo "=== Cleaning caches ==="
rm -rf "$DIST_DIR"
rm -rf "$WEB_DIR/node_modules/.vite"
# Remove rust-embed fingerprints so cargo re-embeds fresh assets
rm -rf "$ROOT/target/$BUILD_MODE/.fingerprint/ivnc-"*
rm -rf "$ROOT/target/$BUILD_MODE/build/ivnc-"*
rm -rf "$ROOT/target/$BUILD_MODE/deps/ivnc-"*
rm -f  "$ROOT/target/$BUILD_MODE/ivnc"
echo "Done."

# ── 2. Build frontend ────────────────────────────────────────
echo "=== Building frontend ==="
mkdir -p "$DIST_DIR/lib"
cp "$WEB_DIR/index.html" "$DIST_DIR/"
cp "$WEB_DIR/ivnc-core.js" "$DIST_DIR/"
cp "$WEB_DIR/ivnc-wr-core.js" "$DIST_DIR/"
cp "$WEB_DIR"/lib/*.js "$DIST_DIR/lib/"
cp "$WEB_DIR/manifest.json" "$DIST_DIR/"
cp "$WEB_DIR/sw.js" "$DIST_DIR/"
cp -r "$WEB_DIR/icons" "$DIST_DIR/icons"
echo "Frontend -> $DIST_DIR"
ls -R "$DIST_DIR"

# ── 3. Build backend ─────────────────────────────────────────
CARGO_ARGS=()
if [[ "$BUILD_MODE" == "release" ]]; then
    CARGO_ARGS+=(--release)
fi
if [[ -n "$CARGO_FEATURES" ]]; then
    CARGO_ARGS+=(--features "$CARGO_FEATURES")
fi

echo "=== Building backend ($BUILD_MODE) ==="
cd "$ROOT"
cargo build "${CARGO_ARGS[@]}"
echo "=== Done ==="
cp "$ROOT/target/$BUILD_MODE/ivnc" "$ROOT/ivnc"
echo "Binary: $ROOT/ivnc"
