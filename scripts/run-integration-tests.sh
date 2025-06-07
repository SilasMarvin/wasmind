#!/bin/bash

# Hive Integration Testing Script
# This script builds a Docker container with hive and runs integration tests inside it

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "🚀 Hive Integration Testing"
echo "=========================="

# Check if Docker is installed
if ! command -v docker &> /dev/null; then
    echo "❌ Docker is not installed. Please install Docker first."
    echo "   Visit: https://docs.docker.com/get-docker/"
    exit 1
fi

# Check if Docker daemon is running
if ! docker info &> /dev/null; then
    echo "❌ Docker daemon is not running. Please start Docker first."
    exit 1
fi

echo "✅ Docker environment is ready"

cd "$PROJECT_ROOT"

# Parse command line arguments
TEST_NAME=""
VERBOSE=false
BUILD_ONLY=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --test)
            TEST_NAME="$2"
            shift 2
            ;;
        --verbose|-v)
            VERBOSE=true
            shift
            ;;
        --build-only)
            BUILD_ONLY=true
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --test <test_name>   Run specific test (default: all)"
            echo "  --verbose, -v        Verbose output"
            echo "  --build-only         Only build the Docker image"
            echo "  --help, -h           Show this help"
            echo ""
            echo "Examples:"
            echo "  $0                    # Run all integration tests"
            echo "  $0 --test message_order   # Run specific test"
            echo "  $0 --verbose          # Run with verbose output"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

# Build the integration test Docker image
echo "🔨 Building integration test container..."
docker build -f tests/Dockerfile -t hive-integration-tests .

if [ "$BUILD_ONLY" = true ]; then
    echo "✅ Build complete"
    exit 0
fi

# Run the integration tests
echo "🧪 Running integration tests..."

# Always set debug logging for tests
DOCKER_ARGS="-e HIVE_LOG=debug"

if [ -n "$TEST_NAME" ]; then
    DOCKER_ARGS="$DOCKER_ARGS -e TEST_NAME=$TEST_NAME"
fi

# Add API keys if available
if [ -n "$DEEPSEEK_API_KEY" ]; then
    DOCKER_ARGS="$DOCKER_ARGS -e DEEPSEEK_API_KEY=$DEEPSEEK_API_KEY"
fi

if [ -n "$FIREWORKS_API_KEY" ]; then
    DOCKER_ARGS="$DOCKER_ARGS -e FIREWORKS_API_KEY=$FIREWORKS_API_KEY"
fi

if [ -n "$OPENAI_API_KEY" ]; then
    DOCKER_ARGS="$DOCKER_ARGS -e OPENAI_API_KEY=$OPENAI_API_KEY"
fi

if [ -n "$ANTHROPIC_API_KEY" ]; then
    DOCKER_ARGS="$DOCKER_ARGS -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY"
fi

# Create a logs directory in the project root
LOG_DIR="$PROJECT_ROOT/test-logs"
rm -r "$LOG_DIR" || true
mkdir -p "$LOG_DIR"
LOG_FILE="$LOG_DIR/hive-test-$(date +%Y%m%d-%H%M%S).log"

echo "📁 Logs will be saved to: $LOG_DIR"

# Run the tests with volume mount for logs
docker run --rm \
    -v "$LOG_DIR:/logs" \
    $DOCKER_ARGS \
    hive-integration-tests \
    sh -c 'if [ -n "$TEST_NAME" ]; then cargo test --release --no-default-features --features headless "$TEST_NAME" -- --nocapture; else cargo test --release --no-default-features --features headless -- --nocapture; fi; EXIT_CODE=$?; if [ -f /workspace/log.txt ]; then cp /workspace/log.txt /logs/log.txt; fi; exit $EXIT_CODE'

TEST_EXIT_CODE=$?

# Check if log file was copied
if [ -f "$LOG_DIR/log.txt" ]; then
    mv "$LOG_DIR/log.txt" "$LOG_FILE"
    echo ""
    echo "📋 TEST LOGS SAVED TO:"
    echo "   $LOG_FILE"
    echo ""
    echo "   File size: $(ls -lh "$LOG_FILE" | awk '{print $5}')"
    echo "   To view: cat $LOG_FILE"
    echo ""
else
    echo ""
    echo "⚠️  No log file generated"
    echo "   - Make sure HIVE_LOG environment variable is set (e.g., HIVE_LOG=debug)"
    echo "   - Check that tests are calling init_test_logger()"
    echo ""
fi

if [ $TEST_EXIT_CODE -eq 0 ]; then
    echo "✅ Integration tests passed"
else
    echo "❌ Integration tests failed with exit code: $TEST_EXIT_CODE"
fi

# Always show the log file location at the end if it exists
if [ -f "$LOG_FILE" ]; then
    echo ""
    echo "🔍 View test logs with:"
    echo "   cat $LOG_FILE"
    echo ""
fi

exit $TEST_EXIT_CODE
