# Test Coverage Analysis & Focused Unit Test Plan

## Current Test Coverage Assessment

**Current Tests (485 lines across 3 files):**
- ✅ Basic dependency resolution with manifests
- ✅ Basic global override of config and auto_spawn
- ✅ Basic global override of source path  
- ✅ Missing manifest error handling
- ✅ Package path loading with new subpath format
- ✅ Git source error handling

## Critical Edge Cases NOT Covered

### Phase 1: Core Logic Unit Tests

#### 0. **Actor Overrides Feature Testing** (NEWLY ADDED - CRITICAL)
- **Basic functionality**: Test that actor_overrides properly override config, source, auto_spawn, required_spawn_with
- **Precedence validation**: Ensure actor_overrides have highest precedence over manifest defaults and global actors
- **Config-only overrides**: Test [actor_overrides.NAME.config] syntax
- **Complete field overrides**: Test overriding all fields in actor_overrides
- **Override validation**: Test behavior when actor_overrides reference non-existent actors
- **Mixed override scenarios**: Combine [actors] and [actor_overrides] for same logical names

#### 1. **required_spawn_with Field Testing** (COMPLETELY MISSING)
- **Basic functionality**: Test that required_spawn_with actually propagates to LoadedActor
- **Global override behavior**: User-defined required_spawn_with vs manifest defaults
- **Invalid actor names**: required_spawn_with referencing non-existent dependencies
- **Empty vs non-empty override**: Test behavior when user overrides to empty list vs non-empty
- **Dependency validation**: Ensure required_spawn_with actors exist in dependencies

#### 2. **Global Override Edge Cases** (PARTIALLY COVERED)
- **Partial vs complete override**: What happens when global override only sets some fields
- **Override precedence**: Multiple actors with same logical name in different parts of tree
- **Config merging behavior**: Currently unclear if manifest config gets merged with global config
- **Multiple dependency levels**: A→B→C where C is globally overridden

#### 3. **Package Path Logic** (BASIC COVERAGE ONLY)
- **New format validation**: Test various subpath formats beyond "crates/name"
- **Relative path resolution**: How paths resolve from different package locations
- **Invalid package paths**: Test error handling for malformed package paths
- **Git sources with packages**: Ensure new package format works with git sources

#### 4. **Core Logic Functions** (NO UNIT TESTS)
- **resolve_relative_source()**: Test with various path combinations
- **sources_match()**: Test all source type combinations and edge cases
- **Circular dependency detection**: With global overrides affecting resolution order

### Phase 2: Complex Scenario Tests

#### 5. **Dependency Chain Logic**
- **Diamond dependencies**: A→B,C; B→D; C→D where D is globally overridden
- **Deep chains**: Multi-level dependency resolution with overrides at different levels
- **Mixed source types**: Path sources depending on git sources and vice versa

#### 6. **Error Handling Completeness**
- **Malformed Hive.toml**: Invalid TOML syntax, missing required fields
- **Source conflicts**: Same logical name with conflicting sources in dependency tree
- **Build directory creation**: Package paths that can't be created

## Specific Test Cases To Implement

### Actor Overrides Tests
```rust
#[test]
fn test_actor_overrides_config_only() {
    // Test [actor_overrides.NAME.config] syntax for config-only overrides
}

#[test]
fn test_actor_overrides_precedence() {
    // Test that actor_overrides beat global actors for same logical name
}

#[test]
fn test_actor_overrides_all_fields() {
    // Test overriding source, config, auto_spawn, required_spawn_with via actor_overrides
}

#[test]
fn test_actor_overrides_partial_fields() {
    // Test overriding only some fields (e.g., just auto_spawn)
}

#[test]
fn test_mixed_actors_and_overrides() {
    // Test when both [actors.NAME] and [actor_overrides.NAME] exist
}

#[test]
fn test_actor_overrides_nonexistent_actor() {
    // Test graceful handling when actor_overrides reference unused actors
}
```

### Required Spawn With Tests
```rust
#[test]
fn test_required_spawn_with_basic_functionality() {
    // Test that required_spawn_with from manifest propagates to LoadedActor
}

#[test]
fn test_required_spawn_with_global_override() {
    // Test user can override required_spawn_with via global config
}

#[test]
fn test_required_spawn_with_invalid_actor_names() {
    // Test required_spawn_with referencing non-existent dependencies
}

#[test]
fn test_required_spawn_with_empty_override() {
    // Test overriding to empty list vs non-empty
}
```

### Global Override Edge Cases
```rust
#[test]
fn test_partial_global_override() {
    // Test when global override only sets some fields (e.g., only auto_spawn)
}

#[test]
fn test_config_merge_vs_replace() {
    // Test if manifest config merges with global config or replaces it
}

#[test]
fn test_deep_dependency_global_override() {
    // Test A→B→C where C is globally overridden
}
```

### Package Path Format Tests
```rust
#[test]
fn test_various_package_subpaths() {
    // Test "packages/actor", "src/actors/main", etc.
}

#[test]
fn test_invalid_package_paths() {
    // Test malformed package paths
}

#[test]
fn test_git_source_with_package_subpath() {
    // Test git sources with new package format
}
```

### Core Logic Function Tests
```rust
#[test]
fn test_resolve_relative_source_logic() {
    // Direct unit tests for resolve_relative_source()
}

#[test]
fn test_sources_match_all_combinations() {
    // Test sources_match() with all source type combinations
}

#[test]
fn test_circular_dependency_detection_with_overrides() {
    // Test circular detection when global overrides affect resolution
}
```

## Implementation Priority

**Phase 1** (~200 lines): Critical missing functionality tests
0. **Actor overrides validation suite** (~50 lines) - **HIGHEST PRIORITY**
1. required_spawn_with validation suite (~40 lines)
2. Global override edge cases (~30 lines)
3. Package path format tests (~30 lines)
4. Core logic function unit tests (~50 lines)

**Phase 2** (~100 lines): Complex scenario coverage
5. Multi-level dependency chains with overrides
6. Source type combinations (path/git dependencies)
7. Error boundary testing for malformed inputs

**Total**: ~300 lines of focused unit tests addressing the most critical gaps in current coverage.

**Focus**: Small, focused tests of specific logic pieces rather than large integration tests.