#!/bin/bash
# Selkies Core Build Script

set -e

echo "==================================="
echo "Selkies Core Build Script"
echo "==================================="
echo ""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Default values
BUILD_MODE="webrtc"
FEATURES=""
RELEASE=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --mode)
            BUILD_MODE="$2"
            shift 2
            ;;
        --features)
            FEATURES="$2"
            shift 2
            ;;
        --release)
            RELEASE=true
            shift
            ;;
        --help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --mode MODE        Build mode: webrtc (default), vaapi, nvenc, qsv"
            echo "  --features FEAT    Additional features (comma-separated)"
            echo "  --release          Build in release mode"
            echo "  --help             Show this help message"
            exit 0
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            exit 1
            ;;
    esac
done

echo "Build Configuration:"
echo "  Mode: $BUILD_MODE"
echo "  Features: ${FEATURES:-none}"
echo "  Release: $RELEASE"
if [ -z "$SKIP_WEB_BUILD" ]; then
    echo "  Frontend: enabled"
else
    echo "  Frontend: skipped (SKIP_WEB_BUILD=1)"
fi
echo ""

# Build frontend assets unless explicitly skipped
if [ -z "$SKIP_WEB_BUILD" ]; then
    if [ -f "web/selkies/package.json" ]; then
        echo ""
        echo "Building frontend (web/selkies)..."
        pushd "web/selkies" >/dev/null
        npm install
        npm run build
        popd >/dev/null
        echo "Frontend build completed."
        echo ""
    else
        echo -e "${YELLOW}Frontend package.json not found; skipping frontend build.${NC}"
    fi
fi

# Build command
BUILD_CMD="cargo build"

if [ "$RELEASE" = true ]; then
    BUILD_CMD="$BUILD_CMD --release"
fi

# Configure features based on mode
case $BUILD_MODE in
    webrtc)
        echo -e "${GREEN}Building with WebRTC support (default)${NC}"
        ;;
    vaapi)
        echo -e "${GREEN}Building with VA-API hardware acceleration${NC}"
        BUILD_CMD="$BUILD_CMD --features vaapi"
        ;;
    nvenc)
        echo -e "${GREEN}Building with NVIDIA NVENC hardware acceleration${NC}"
        BUILD_CMD="$BUILD_CMD --features nvenc"
        ;;
    qsv)
        echo -e "${GREEN}Building with Intel Quick Sync hardware acceleration${NC}"
        BUILD_CMD="$BUILD_CMD --features qsv"
        ;;
    *)
        echo -e "${RED}Unknown build mode: $BUILD_MODE${NC}"
        exit 1
        ;;
esac

# Add additional features
if [ -n "$FEATURES" ]; then
    BUILD_CMD="$BUILD_CMD --features $FEATURES"
fi

echo ""
echo "Running: $BUILD_CMD"
echo ""

# Execute build
$BUILD_CMD

if [ $? -eq 0 ]; then
    echo ""
    echo -e "${GREEN}==================================="
    echo "Build completed successfully!"
    echo "===================================${NC}"

    if [ "$RELEASE" = true ]; then
        echo ""
        echo "Binary location: target/release/selkies-core"
    else
        echo ""
        echo "Binary location: target/debug/selkies-core"
    fi
else
    echo ""
    echo -e "${RED}==================================="
    echo "Build failed!"
    echo "===================================${NC}"
    exit 1
fi
