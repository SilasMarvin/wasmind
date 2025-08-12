#!/bin/bash

# Script to compile all actors with cargo component build --release
# Shows checkmark for success or stderr output for failures

set -e

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color
BOLD='\033[1m'

# Get the script directory and project root
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
ACTORS_DIR="$PROJECT_ROOT/actors"

echo -e "${BOLD}Compiling all actors with --release flag...${NC}\n"

# Track overall success
ALL_SUCCESS=true

# Function to compile an actor
compile_actor() {
    local actor_path=$1
    local actor_name=$2
    
    # Change to actor directory
    cd "$actor_path"
    
    # Capture stderr to temporary file
    TEMP_ERR=$(mktemp)
    
    # Try to compile, capturing stderr
    if cargo component build --release 2>"$TEMP_ERR" >/dev/null; then
        echo -e "${GREEN}✓${NC} $actor_name"
    else
        echo -e "${RED}✗${NC} $actor_name"
        echo -e "${RED}Error output:${NC}"
        cat "$TEMP_ERR"
        echo ""
        ALL_SUCCESS=false
    fi
    
    # Clean up temp file
    rm -f "$TEMP_ERR"
}

# Compile standalone actors
echo -e "${BOLD}Standalone actors:${NC}"
compile_actor "$ACTORS_DIR/assistant" "assistant"
compile_actor "$ACTORS_DIR/execute_bash" "execute_bash"
compile_actor "$ACTORS_DIR/file_interaction/crates/file_interaction_actor" "file_interaction"

# Compile code_with_experts actors
echo -e "\n${BOLD}Code with experts actors:${NC}"
compile_actor "$ACTORS_DIR/code_with_experts/crates/approve" "approve"
compile_actor "$ACTORS_DIR/code_with_experts/crates/file_interaction_with_approval" "file_interaction_with_approval"
compile_actor "$ACTORS_DIR/code_with_experts/crates/request_changes" "request_changes"

# Compile delegation_network actors
echo -e "\n${BOLD}Delegation network actors:${NC}"
compile_actor "$ACTORS_DIR/delegation_network/crates/check_health" "check_health"
compile_actor "$ACTORS_DIR/delegation_network/crates/complete" "complete"
compile_actor "$ACTORS_DIR/delegation_network/crates/delegation_network_coordinator" "delegation_network_coordinator"
compile_actor "$ACTORS_DIR/delegation_network/crates/flag_issue" "flag_issue"
compile_actor "$ACTORS_DIR/delegation_network/crates/planner" "planner"
compile_actor "$ACTORS_DIR/delegation_network/crates/report_normal" "report_normal"
compile_actor "$ACTORS_DIR/delegation_network/crates/send_manager_message" "send_manager_message"
compile_actor "$ACTORS_DIR/delegation_network/crates/send_message" "send_message"
compile_actor "$ACTORS_DIR/delegation_network/crates/spawn_agent" "spawn_agent"
compile_actor "$ACTORS_DIR/delegation_network/crates/wait" "wait"

# Summary
echo ""
if [ "$ALL_SUCCESS" = true ]; then
    echo -e "${GREEN}${BOLD}All actors compiled successfully!${NC}"
    exit 0
else
    echo -e "${RED}${BOLD}Some actors failed to compile.${NC}"
    exit 1
fi