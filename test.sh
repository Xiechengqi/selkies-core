#!/bin/bash
# Selkies Core Test Script

set -e

echo "==================================="
echo "Selkies Core Test Script"
echo "==================================="
echo ""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Default values
TEST_MODE="all"
VERBOSE=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --mode)
            TEST_MODE="$2"
            shift 2
            ;;
        --verbose)
            VERBOSE=true
            shift
            ;;
        --help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --mode MODE    Test mode: all (default), unit, integration, websocket"
            echo "  --verbose      Enable verbose output"
            echo "  --help         Show this help message"
            exit 0
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            exit 1
            ;;
    esac
done

echo "Test Configuration:"
echo "  Mode: $TEST_MODE"
echo "  Verbose: $VERBOSE"
echo ""

# Test command
TEST_CMD="cargo test"

if [ "$VERBOSE" = true ]; then
    TEST_CMD="$TEST_CMD -- --nocapture"
fi

case $TEST_MODE in
    all)
        echo -e "${GREEN}Running all tests${NC}"
        ;;
    unit)
        echo -e "${GREEN}Running unit tests${NC}"
        TEST_CMD="$TEST_CMD --lib"
        ;;
    integration)
        echo -e "${GREEN}Running integration tests${NC}"
        TEST_CMD="$TEST_CMD --test '*'"
        ;;
    websocket)
        echo -e "${YELLOW}Running WebSocket-only tests${NC}"
        TEST_CMD="$TEST_CMD --no-default-features --features websocket-legacy"
        ;;
    *)
        echo -e "${RED}Unknown test mode: $TEST_MODE${NC}"
        exit 1
        ;;
esac

echo ""
echo "Running: $TEST_CMD"
echo ""

# Execute tests
eval $TEST_CMD

if [ $? -eq 0 ]; then
    echo ""
    echo -e "${GREEN}==================================="
    echo "All tests passed!"
    echo "===================================${NC}"
else
    echo ""
    echo -e "${RED}==================================="
    echo "Tests failed!"
    echo "===================================${NC}"
    exit 1
fi
