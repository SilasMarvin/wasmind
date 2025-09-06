#!/bin/bash

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Default to dry run
DRY_RUN=true

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --publish|--real|--force)
            DRY_RUN=false
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [--publish|--real|--force] [--dry-run]"
            echo ""
            echo "Options:"
            echo "  --publish, --real, --force    Actually publish crates (default is dry-run)"
            echo "  --dry-run                     Only show what would be published (default)"
            echo "  -h, --help                    Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown option $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

echo -e "${YELLOW}Publishing all crates in workspace...${NC}"
if [ "$DRY_RUN" = true ]; then
    echo -e "${YELLOW}Running in DRY RUN mode. Use --publish to actually publish.${NC}"
else
    echo -e "${RED}REAL PUBLISH MODE - This will actually publish crates!${NC}"
    read -p "Are you sure you want to continue? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Aborted."
        exit 1
    fi
fi

cd "$PROJECT_ROOT"

# Define crates in dependency order (must be published in this order)
CRATE_DIRS=(
    "crates/wasmind_llm_types"
    "crates/wasmind_config"
    "crates/wasmind_actor_bindings"
    "crates/wasmind_actor_utils_macros"
    "crates/wasmind_actor_utils_common_messages"
    "crates/wasmind_actor_utils"
    "crates/wasmind_actor_loader"
    "crates/wasmind"
    "crates/wasmind_cli"
)

# Verify all crates exist
echo -e "${GREEN}Crates to publish (in dependency order):${NC}"
for crate_dir in "${CRATE_DIRS[@]}"; do
    if [ ! -f "$crate_dir/Cargo.toml" ]; then
        echo -e "${RED}Error: $crate_dir/Cargo.toml not found${NC}"
        exit 1
    fi
    crate_name=$(grep "^name" "$crate_dir/Cargo.toml" | head -1 | sed 's/name = "\(.*\)"/\1/')
    echo "  - $crate_name ($crate_dir)"
done

echo ""

# Publish each crate in dependency order
for crate_dir in "${CRATE_DIRS[@]}"; do
    crate_name=$(grep "^name" "$crate_dir/Cargo.toml" | head -1 | sed 's/name = "\(.*\)"/\1/')
    
    echo -e "${YELLOW}Processing $crate_name...${NC}"
    
    cd "$PROJECT_ROOT/$crate_dir"
    
    if [ "$DRY_RUN" = true ]; then
        echo "  Running: cargo publish --dry-run"
        if cargo publish --dry-run; then
            echo -e "  ${GREEN}✓ $crate_name would publish successfully${NC}"
        else
            echo -e "  ${RED}✗ $crate_name would fail to publish${NC}"
        fi
    else
        echo "  Running: cargo publish"
        if cargo publish; then
            echo -e "  ${GREEN}✓ $crate_name published successfully${NC}"
        else
            echo -e "  ${RED}✗ $crate_name failed to publish${NC}"
            echo "  Continuing with remaining crates..."
        fi
        # Add a small delay between publishes to avoid rate limiting
        sleep 2
    fi
    
    cd "$PROJECT_ROOT"
    echo ""
done

if [ "$DRY_RUN" = true ]; then
    echo -e "${GREEN}Dry run completed. Use --publish to actually publish crates.${NC}"
else
    echo -e "${GREEN}Publishing completed!${NC}"
fi