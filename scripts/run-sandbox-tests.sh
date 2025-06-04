#!/bin/bash

# Copilot Sandbox Testing Script
# This script runs end-to-end tests in a safe Docker environment

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "🚀 Copilot Sandbox Testing"
echo "=========================="

# Check if Docker is installed
if ! command -v docker &> /dev/null; then
    echo "❌ Docker is not installed. Please install Docker first."
    echo "   Visit: https://docs.docker.com/get-docker/"
    exit 1
fi

# Check if Docker Compose is installed
if ! command -v docker-compose &> /dev/null; then
    echo "❌ Docker Compose is not installed. Please install Docker Compose first."
    echo "   Visit: https://docs.docker.com/compose/install/"
    exit 1
fi

# Check if Docker daemon is running
if ! docker info &> /dev/null; then
    echo "❌ Docker daemon is not running. Please start Docker first."
    exit 1
fi

echo "✅ Docker environment is ready"

cd "$PROJECT_ROOT"

# Function to cleanup Docker resources
cleanup() {
    echo "🧹 Cleaning up Docker resources..."
    docker-compose -f tests/docker/docker-compose.test.yml down -v --remove-orphans 2>/dev/null || true
    docker system prune -f --volumes 2>/dev/null || true
}

# Set trap to cleanup on exit
trap cleanup EXIT

# Build the sandbox container
echo "🔨 Building sandbox container..."
docker-compose -f tests/docker/docker-compose.test.yml build

# Test the sandbox environment first
echo "🧪 Testing sandbox environment..."
docker-compose -f tests/docker/docker-compose.test.yml up -d
sleep 5

# Wait for container to be healthy
echo "⏳ Waiting for container to be ready..."
for i in {1..30}; do
    if docker exec copilot-test-sandbox python3 /workspace/validate-tests.py &>/dev/null; then
        echo "✅ Container is ready"
        break
    fi
    if [ $i -eq 30 ]; then
        echo "❌ Container failed to become ready"
        docker logs copilot-test-sandbox
        exit 1
    fi
    sleep 1
done

# Stop the test container
docker-compose -f tests/docker/docker-compose.test.yml down

# Parse command line arguments
RUN_ALL=false
SPECIFIC_TEST=""
VERBOSE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --all)
            RUN_ALL=true
            shift
            ;;
        --test)
            SPECIFIC_TEST="$2"
            shift 2
            ;;
        --verbose|-v)
            VERBOSE=true
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --all                Run all sandbox tests"
            echo "  --test <test_name>   Run specific test"
            echo "  --verbose, -v        Verbose output"
            echo "  --help, -h           Show this help"
            echo ""
            echo "Available tests:"
            echo "  - environment        Test sandbox setup"
            echo "  - file-reading       Test file reading workflow"
            echo "  - command-execution  Test command execution"
            echo "  - error-recovery     Test error handling"
            echo "  - multi-step         Test complex workflows"
            echo ""
            echo "Examples:"
            echo "  $0 --all                          # Run all tests"
            echo "  $0 --test environment             # Run environment test"
            echo "  $0 --test file-reading --verbose  # Run file reading test with verbose output"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

# Determine which tests to run
if [ "$RUN_ALL" = true ]; then
    TESTS=(
        "test_sandbox_environment_setup"
        "test_sandboxed_file_reading_workflow"
        "test_sandboxed_command_execution_workflow"
        "test_sandboxed_error_recovery"
        "test_sandboxed_multi_step_workflow"
    )
elif [ -n "$SPECIFIC_TEST" ]; then
    case $SPECIFIC_TEST in
        environment)
            TESTS=("test_sandbox_environment_setup")
            ;;
        file-reading)
            TESTS=("test_sandboxed_file_reading_workflow")
            ;;
        command-execution)
            TESTS=("test_sandboxed_command_execution_workflow")
            ;;
        error-recovery)
            TESTS=("test_sandboxed_error_recovery")
            ;;
        multi-step)
            TESTS=("test_sandboxed_multi_step_workflow")
            ;;
        *)
            echo "❌ Unknown test: $SPECIFIC_TEST"
            echo "Use --help to see available tests"
            exit 1
            ;;
    esac
else
    echo "🔍 Running basic environment test..."
    TESTS=("test_sandbox_environment_setup")
fi

# Run the tests
echo "🧪 Running sandbox tests..."
PASSED=0
FAILED=0

for test in "${TESTS[@]}"; do
    echo ""
    echo "🧪 Running test: $test"
    echo "----------------------------------------"
    
    if [ "$VERBOSE" = true ]; then
        CARGO_CMD="cargo test $test -- --ignored --nocapture"
    else
        CARGO_CMD="cargo test $test -- --ignored"
    fi
    
    if eval "$CARGO_CMD"; then
        echo "✅ $test PASSED"
        ((PASSED++))
    else
        echo "❌ $test FAILED"
        ((FAILED++))
    fi
done

# Summary
echo ""
echo "📊 Test Summary"
echo "==============="
echo "Passed: $PASSED"
echo "Failed: $FAILED"
echo "Total:  $((PASSED + FAILED))"

if [ $FAILED -eq 0 ]; then
    echo "🎉 All tests passed!"
    exit 0
else
    echo "💥 Some tests failed!"
    exit 1
fi