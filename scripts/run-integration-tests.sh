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

DOCKER_ARGS=""
if [ "$VERBOSE" = true ]; then
    DOCKER_ARGS="$DOCKER_ARGS -e HIVE_LOG=debug"
fi

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

# Run the tests
docker run --rm $DOCKER_ARGS hive-integration-tests

echo "✅ Integration tests complete"