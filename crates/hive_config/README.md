# Hive Configuration Specification

This document outlines the configuration system for the Hive framework. The system is designed to be powerful for developers while remaining intuitive for users.

Configuration is managed through two key concepts:
1.  **The User Configuration File:** A TOML file provided by the user that defines which actor *instances* are available to their Hive system, provides runtime configuration, and sets up the environment.
2.  **The `Hive.toml` Actor Manifest:** A file created by an actor *developer* that declares an actor's canonical identity and its dependencies on other actors.

---

## The User Configuration File

This file is the primary entry point for a user. It uses TOML tables to define a collection of named actor instances.

### Defining Actor Instances

Actors are defined under the `[actors]` table, where each actor instance is its own sub-table. The key of the sub-table serves as the unique `name` for that instance. This name is used to identify the actor within the system, for example, in the `starting_actors` list.

```toml
# The key "my_llm_assistant" is the unique name for this actor instance.
[actors.my_llm_assistant]
source = { path = "./actors/assistant" }
auto_spawn = true

[actors.my_llm_assistant.config]
model_name = "gpt-4o"

# The key "secure_shell" is the unique name for this instance.
[actors.secure_shell]
source = { path = "./actors/execute_bash" }
# auto_spawn defaults to false if not specified.
```

An actor instance definition has the following fields:

*   **`source` (table, required):** Specifies where to load the actor's WASM component from.
*   **`auto_spawn` (boolean, optional):** If `true`, this actor will be spawned automatically when the Hive system starts. Defaults to `false`.
*   **`config` (table, optional):** A free-form table containing configuration values passed to the actor upon creation.

### Defining the Actor `source`

The `source` table supports multiple ways to locate actor code, providing flexibility for local development and production deployments.

#### Path Source

Loads an actor from a local filesystem path. This is ideal for local development.

*   `path` (string): A relative or absolute path to the actor's directory.
*   `package` (string, optional): If the path points to a Rust workspace, this specifies which package contains the actor. When using `package`, Hive will look for the `Hive.toml` manifest in `crates/{package}/` within the workspace. Note: Package support is currently designed for Rust workspaces but will be expanded to other languages in the future.

```toml
# Simple path source
[actors.local_assistant]
source = { path = "/Users/silas/hive/actors/assistant" }

# Path source pointing to a Rust workspace with a package
# Hive will look for the manifest at: /Users/silas/hive/crates/some_utility/Hive.toml
[actors.another_actor]
source = { path = "/Users/silas/hive", package = "some_utility" }
```

#### Git Source

Clones a remote Git repository to fetch the actor's code.

*   `url` (string): The URL of the Git repository.
*   `git_ref` (table, optional): Specifies which Git reference to check out. Can be a branch, tag, or specific revision hash. Defaults to the repository's default branch.
*   `package` (string, optional): If the repository is a Rust workspace, this specifies which package contains the actor. Similar to path sources, Hive will look for the manifest in `crates/{package}/` for Git sources with packages.

```toml
# Clones the 'main' branch of a repository
[actors.assistant_from_git]
source = { url = "https://github.com/my-org/hive-assistant", git_ref = { branch = "main" } }

# Uses a specific version tag
[actors.bash_v1]
source = { url = "https://github.com/my-org/hive-execute-bash", git_ref = { tag = "v1.0.2" } }

# Pins to an exact commit revision
[actors.stable_tool]
source = { url = "https://github.com/my-org/hive-tools", git_ref = { rev = "a1b2c3d4e5f6" } }

# Clones a monorepo and targets a specific package within it
[actors.specific_tool]
source = { url = "https://github.com/my-org/hive-tools", git_ref = { tag = "v1.1.0" }, package = "data_parser" }
```

---

## The `Hive.toml` Actor Manifest

**IMPORTANT:** Every actor MUST have a `Hive.toml` manifest file. There are no fallbacks or auto-generation - this is a strict requirement for all actors in the Hive system.

This manifest is created by the actor developer and is bundled with the actor's source code. It provides essential metadata, making the actor a self-describing package.

*   **`actor_id` (string, required):** The canonical, globally unique identifier for the actor *type*. The recommended format is `namespace:name` (e.g., `hive:execute-bash`).
*   **`[dependencies]` (table, optional):** Declares other actors that this actor depends on. Each key is a logical name for the dependency, used for configuration overrides.

### Example: Actor with Dependencies

The `delegation_network_coordinator` requires other actors to function. Its developer declares these in its `Hive.toml`.

**Important Note on Relative Paths:** When dependencies use relative paths, they are resolved from the location of the `Hive.toml` file declaring them. For actors in Rust workspaces (using the `package` field), this means paths are relative to `crates/{package}/`, not the workspace root.

**File: `/path/to/delegation_network_coordinator/Hive.toml`**
```toml
actor_id = "my-co:delegation-network-coordinator"

# The keys "sender" and "receiver" are logical names for the dependencies.
# The Hive system will use the source path to find and load them.
# Note: Relative paths are resolved from the location of this Hive.toml file.
[dependencies]
sender = { source = { path = "../delegation_network_message_sender" } }
receiver = { source = { path = "../delegation_network_message_receiver" } }
```

---

## Dependency Configuration Overrides

This system allows a user to configure an actor's transitive dependencies without needing to know their location or even add them to the main `[actors]` list. This is done by nesting a `dependencies` table inside an actor's `config`.

### Example: Overriding a Dependency's Configuration

A user wants to deploy the `delegation_network_coordinator` and needs to customize the behavior of its `sender` component.

**User Configuration File:**
```toml
# List of actor instances to spawn at startup.
starting_actors = ["my_delegator"]

# The user only defines the top-level actor instance.
[actors.my_delegator]
source = { path = "/path/to/actors/delegation_network_coordinator" }

# Configuration for the 'my_delegator' instance itself.
[actors.my_delegator.config]
coordination_strategy = "broadcast"

# By nesting 'dependencies', the user targets dependencies declared in the actor's Hive.toml.
# The key 'sender' must match the logical name from the coordinator's Hive.toml.
[actors.my_delegator.config.dependencies.sender]
# You can override the source (useful for using a different version or local development)
source = { path = "/my/local/fork/of/sender" }
auto_spawn = false

# Note: The parameters below are hypothetical and for illustrative purposes only.
# The actual configurable parameters depend on the 'sender' actor's implementation.
[actors.my_delegator.config.dependencies.sender.config]
max_queue_size = 1000
delivery_guarantee = "at-least-once"
```

### How It Works

1.  Hive parses the user's config and finds the `my_delegator` actor instance.
2.  It loads the actor from its `source` and reads its `Hive.toml` manifest.
3.  From the manifest, it discovers the dependency logically named `sender`.
4.  When the `delegation_network_coordinator` requests to spawn its `sender` dependency, the Hive system prepares its configuration.
5.  It checks the user's config for `my_delegator` at the path `[config.dependencies.sender.config]`.
6.  It finds the user's overrides (`max_queue_size`, etc.) and merges them with the `sender` actor's default configuration.
7.  The `sender` actor is spawned with the custom configuration, achieving a powerful separation of concerns between user-level and developer-level definitions.

## Troubleshooting Your Configuration

Hive performs rigorous checks on your configuration file and all actor manifests before starting. This "fail-fast" approach helps catch issues early and prevents unpredictable runtime behavior. Here are some of the most common errors you might encounter and how to resolve them.

### 1. Circular Dependency

This error occurs when an actor's dependency tree contains a cycle. For example, Actor A depends on Actor B, which in turn depends back on Actor A.

**Why it happens:** Hive needs to build a clear, acyclic graph of actors to manage their lifecycle and configuration. A circular dependency creates an infinite loop during this process.

**Example `Hive.toml` Files:**

```toml
# /path/to/actor-a/Hive.toml
actor_id = "my-co:actor-a"
[dependencies]
b_instance = { source = { path = "../actor-b" } }
```

```toml
# /path/to/actor-b/Hive.toml
actor_id = "my-co:actor-b"
[dependencies]
a_instance = { source = { path = "../actor-a" } }
```

**Expected Error:**
```
Error: Circular dependency detected while resolving 'my-co:actor-a'.
Resolution path: my-co:actor-a -> my-co:actor-b -> my-co:actor-a
```

**How to Fix:** Re-evaluate your architecture to remove the cycle. An actor cannot have a direct or indirect startup dependency on itself. You may need to introduce a third actor or change how they communicate via messages rather than direct spawning.

---

### 2. Ambiguous Dependency Source (Diamond Problem)

This happens when a top-level actor depends on two different actors that, in turn, both depend on a third actor with the *same logical name* but point to *different sources*.

**Why it happens:** Hive cannot determine which version of the shared dependency (`common-tool` in this case) to use.

**Example `Hive.toml` Files:**

```toml
# /path/to/app/Hive.toml
actor_id = "my-co:app"
[dependencies]
parser = { source = { path = "../parser" } }
validator = { source = { path = "../validator" } }
```
```toml
# /path/to/parser/Hive.toml
actor_id = "my-co:parser"
[dependencies]
tool = { source = { path = "../../tools/common-tool-v1" } }
```
```toml
# /path/to/validator/Hive.toml
actor_id = "my-co:validator"
[dependencies]
tool = { source = { path = "../../tools/common-tool-v2" } }
```

**Expected Error:**
```
Error: Conflicting sources for dependency 'tool' required by 'my-co:app'.
- Path via 'parser' resolves to '.../tools/common-tool-v1'
- Path via 'validator' resolves to '.../tools/common-tool-v2'
```

**How to Fix:** You must resolve the ambiguity. Update the `Hive.toml` manifests of the intermediate actors (`parser` and `validator` in this example) to point to a single, canonical version of the shared dependency.

---

### 3. Source Path or Package Not Found

This is a common error when the `source` table in your configuration contains an incorrect `path` or `package` name.

**Why it happens:** Hive cannot locate the files it needs to build the actor.

**Example User Configuration:**
```toml
# Case 1: Incorrect path
[actors.my_actor]
source = { path = "./non_existent_directory" }

# Case 2: Incorrect package name in a workspace
[actors.my_tool]
source = { path = "./my_workspace", package = "non_existent_package" }
```

**Expected Errors:**
```
# For Case 1
Error: Failed to load actor 'my_actor'. Source path './non_existent_directory' not found.

# For Case 2
Error: Failed to load actor 'my_tool'. Package 'non_existent_package' not found in workspace at './my_workspace'.
```

**How to Fix:**
*   For path errors, verify the path is correct relative to where you are running the Hive application.
*   For package errors, check the `name` field in the `[package]` table of the `Cargo.toml` for the actor you are trying to load. Ensure it matches the `package` value in your configuration.

---

### 4. Missing Actor Manifest

This error occurs when an actor is referenced but doesn't have a `Hive.toml` manifest file.

**Why it happens:** Every actor in Hive MUST have a `Hive.toml` manifest file. This is a strict requirement with no exceptions or fallbacks.

**Example User Configuration:**
```toml
[actors.my_actor]
source = { path = "./actors/my_actor" }
# But there's no Hive.toml at ./actors/my_actor/Hive.toml
```

**Expected Error:**
```
Error: Actor 'my_actor' at 'path: ./actors/my_actor' is missing required Hive.toml manifest file. 
All actors must have a Hive.toml file that declares their actor_id.
```

**How to Fix:** Create a `Hive.toml` file in the actor's directory with at least the required `actor_id` field:
```toml
# ./actors/my_actor/Hive.toml
actor_id = "my-namespace:my-actor"
```

For Rust workspaces using the `package` field, ensure the `Hive.toml` is in `crates/{package}/`:
```toml
# If your config has:
[actors.my_workspace_actor]
source = { path = "./my_workspace", package = "my_package" }

# Then Hive.toml must be at: ./my_workspace/crates/my_package/Hive.toml
```

---

### 5. Orphaned Dependency Configuration

This is a non-fatal warning that occurs when you provide a configuration override for a dependency that isn't actually declared in the actor's `Hive.toml`.

**Why it happens:** This is usually caused by a typo in the dependency's logical name or by using an outdated configuration file after an actor's dependencies have changed.

**Example User Configuration:**
```toml
[actors.my_app]
source = { path = "./app" }

# The 'Hive.toml' for 'my_app' does not declare a dependency named 'imaginary_tool'.
[actors.my_app.config.dependencies.imaginary_tool]
auto_spawn = false
```

**Expected Warning:**
```
Warning: Configuration for unknown dependency 'imaginary_tool' in actor 'my_app' will be ignored. Check for typos or outdated configuration.
```

**How to Fix:** Remove the unnecessary configuration block or correct the dependency's logical name (e.g., `imaginary_tool`) to match the name defined in the actor's `Hive.toml` manifest.
