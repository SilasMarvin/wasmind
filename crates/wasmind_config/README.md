# Wasmind Configuration Specification

Wasmind uses [TOML](https://toml.io/) for configuration files. Throughout this documentation, we use TOML's dotted key notation (e.g., `[table.subtable]`) for clarity and consistency, though the inline table syntax is equally valid.

Configuration is managed through two key concepts:
1.  **The User Configuration File:** A TOML file provided by the user that defines which actor *instances* are available to their Wasmind system, provides runtime configuration, and sets up the environment.
2.  **The `Wasmind.toml` Actor Manifest:** A file created by an actor *developer* that declares an actor's canonical identity and its dependencies on other actors.

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
*   **`auto_spawn` (boolean, optional):** If `true`, this actor will be spawned automatically into every new scope including the starting scope. Defaults to `false`.
*   **`required_spawn_with` (array of strings, optional):** A list of actor logical names that must be spawned together when this actor is spawned. These actors will spawn as a group when the parent spawns. Defaults to an empty list.
*   **`config` (table, optional):** A free-form table containing configuration values passed to the actor upon creation.

### Defining the Actor `source`

The `source` table supports multiple ways to locate actor code, providing flexibility for local development and production deployments.

#### Path Source

Loads an actor from a local filesystem path. This is ideal for local development.

*   `path` (string): A relative or absolute path to the actor's directory.
*   `package` (string, required for cargo workspaces): If the path points to a Rust workspace, this specifies the full subpath to the package directory containing the actor. Wasmind will look for the `Wasmind.toml` manifest at `{path}/{package}/`. 

```toml
# Simple path source (single-package project)
[actors.local_assistant]
source = { path = "/Users/silas/Wasmind/actors/assistant" }

# Path source pointing to a Rust workspace - package field is REQUIRED
# Wasmind will look for the manifest at: /Users/silas/Wasmind/crates/some_utility/Wasmind.toml
[actors.another_actor]
source = { path = "/Users/silas/Wasmind", package = "crates/some_utility" }
```

**⚠️ Important Note for Cargo Workspaces:**

When working with cargo workspaces, the `package` field is **required**, not optional. This is because Wasmind currently needs to know the exact package location to find the compiled WASM output after building. If you omit the `package` field when pointing to a workspace, you'll encounter "WASM not found" errors.

```toml
# ❌ This will NOT work for cargo workspaces:
[actors.workspace_actor]
source = { path = "/path/to/workspace/crates/my_actor" }

# ✅ This WILL work for cargo workspaces:
[actors.workspace_actor] 
source = { path = "/path/to/workspace", package = "crates/my_actor" }
```

This limitation will be improved in future versions to automatically detect workspace structures.

#### Git Source

Clones a remote Git repository to fetch the actor's code.

*   `url` (string): The URL of the Git repository.
*   `git_ref` (table, optional): Specifies which Git reference to check out. Can be a branch, tag, or specific revision hash. Defaults to the repository's default branch.
*   `package` (string, required for cargo workspaces): If the repository is a Rust workspace, this specifies the full subpath to the package directory containing the actor. Similar to path sources, Wasmind will look for the manifest at `{repository_root}/{package}/` for Git sources with packages. **Important:** This field is currently required when the Git repository contains a cargo workspace, as Wasmind needs it to locate the compiled WASM output correctly.

```toml
# Clones the 'main' branch of a repository (single-package)
[actors.assistant_from_git]
source = { url = "https://github.com/my-org/Wasmind-assistant", git_ref = { branch = "main" } }

# Uses a specific version tag (single-package)
[actors.bash_v1]
source = { url = "https://github.com/my-org/Wasmind-execute-bash", git_ref = { tag = "v1.0.2" } }

# Pins to an exact commit revision (single-package)
[actors.stable_tool]
source = { url = "https://github.com/my-org/Wasmind-tools", git_ref = { rev = "a1b2c3d4e5f6" } }

# Clones a monorepo/workspace - package field is REQUIRED
[actors.specific_tool]
source = { url = "https://github.com/my-org/Wasmind-tools", git_ref = { tag = "v1.1.0" }, package = "crates/data_parser" }
```

---

## The `Wasmind.toml` Actor Manifest

**IMPORTANT:** Every actor MUST have a `Wasmind.toml` manifest file. There are no fallbacks or auto-generation - this is a strict requirement for all actors in the Wasmind system.

This manifest is created by the actor developer and is bundled with the actor's source code. It provides essential metadata, making the actor a self-describing package.

*   **`actor_id` (string, required):** The canonical, globally unique identifier for the actor *type*. The recommended format is `namespace:name` (e.g., `Wasmind:execute-bash`).
*   **`required_spawn_with` (array of strings, optional):** A list of dependency logical names that must be spawned together when this actor is spawned. These dependencies must be declared in the `[dependencies]` section. Defaults to an empty list.
*   **`[dependencies]` (table, optional):** Declares other actors that this actor depends on. Each key is a logical name for the dependency.

### Example: Actor with Dependencies

The `delegation_network_coordinator` requires other actors to function. Its developer declares these in its `Wasmind.toml`.

**Important Note on Relative Paths:** When dependencies use relative paths, they are resolved from the location of the `Wasmind.toml` file declaring them. For actors in Rust workspaces (using the `package` field), this means paths are relative to `crates/{package}/`, not the workspace root.

**File: `/path/to/delegation_network_coordinator/Wasmind.toml`**
```toml
actor_id = "my-co:delegation-network-coordinator"

# These actors will spawn together with the coordinator when it's spawned
required_spawn_with = ["sender", "receiver"]

# The keys "sender" and "receiver" are logical names for the dependencies.
# The Wasmind system will use the source path to find and load them.
# Note: Relative paths are resolved from the location of this Wasmind.toml file.
[dependencies.sender]
source = { path = "../delegation_network_message_sender" }

[dependencies.receiver]
source = { path = "../delegation_network_message_receiver" }
```

---

## Understanding Actor Configuration

### The Big Picture

When you use an actor in Wasmind, that actor might need other actors to work properly. These are called **dependencies**.

For example:
- A web server actor might depend on a logger actor and a database actor
- A chat bot actor might depend on an LLM client actor

**Where do dependencies come from?** Each actor declares its dependencies in its own `Wasmind.toml` file.

### How Configuration Works

You have **two ways** to configure actors:

1. **`[actors.NAME]`** - Add new actors to your system
2. **`[actor_overrides.NAME]`** - Modify actors that already exist as dependencies

---

## Step-by-Step Example

Let's say you want to use a chat bot that depends on a logger. Here's how it works:

### Step 1: The Actor's Dependencies

Your chat bot actor has this `Wasmind.toml` file:

```toml
# File: ./actors/chatbot/Wasmind.toml
actor_id = "my-company:chatbot"

[dependencies.logger]
source = { path = "../simple_logger" }
```

This means the chatbot **needs** a logger actor to work.

### Step 2: Your Basic Configuration

```toml
# Your config file
starting_actors = ["my_chatbot"]

# Add the chatbot to your system
[actors.my_chatbot]
source = { path = "./actors/chatbot" }

[actors.my_chatbot.config]
personality = "helpful"
```

**What happens:** Wasmind loads your chatbot, sees it needs a logger, and automatically loads the logger too.

### Step 3: Customize the Logger (Optional)

What if you want the logger to be more verbose? Use `actor_overrides`:

```toml
starting_actors = ["my_chatbot"]

[actors.my_chatbot]
source = { path = "./actors/chatbot" }

[actors.my_chatbot.config]
personality = "helpful"

# Customize the logger that your chatbot uses
[actor_overrides.logger.config]
level = "debug"
format = "json"
```

**What happens:** Wasmind loads your chatbot and its logger dependency, but applies your custom configuration to the logger.

---

## More Examples

### Adding Multiple Actors

```toml
starting_actors = ["chatbot", "web_server"]

# First actor
[actors.chatbot]
source = { path = "./actors/chatbot" }

# Second actor (independent)
[actors.web_server]
source = { path = "./actors/web_server" }
auto_spawn = true
```

### Completely Replacing a Dependency

```toml
starting_actors = ["chatbot"]

[actors.chatbot]
source = { path = "./actors/chatbot" }

# Replace the logger dependency with your own custom logger
[actor_overrides.logger]
source = { path = "./my_custom_logger" }
auto_spawn = true

[actor_overrides.logger.config]
output_file = "/var/log/myapp.log"
```

---

## How to Know What You Can Override

**Question:** "How do I know what dependencies exist?"

**Answer:** Look at the `Wasmind.toml` files! 

If your actor's `Wasmind.toml` has:
```toml
[dependencies.logger]
source = { path = "../logger" }

[dependencies.database]
source = { path = "../db" }
```

Then you can override `logger` and `database`:
```toml
[actor_overrides.logger.config]
level = "debug"

[actor_overrides.database.config]
connection_string = "postgres://localhost/mydb"
```

---

## Common Mistakes (And How to Fix Them)

### ❌ Mistake 1: Defining the same name twice

```toml
# BAD: Don't do this
[actors.logger]
source = { path = "./my_logger" }

[actor_overrides.logger.config]  # ERROR!
level = "debug"
```

**Fix:** Set the config on `actors.logger`:
```toml
# Option A: Use only [actors] (adds new logger)
[actors.logger]
source = { path = "./my_logger" }

[actors.logger.config]
level = "debug"
```

### ❌ Mistake 2: Trying to override something that doesn't exist

```toml
# BAD: If no actor depends on "nonexistent"
[actor_overrides.nonexistent.config]  # ERROR!
some_setting = "value"
```

**Fix:** Either add it as a new actor, or check the dependency names:
```toml
# Add as new actor instead
[actors.nonexistent]
source = { path = "./actors/nonexistent" }

[actors.nonexistent.config]
some_setting = "value"
```

### ❌ Mistake 3: Name conflicts

```toml
# BAD: If your main_app already depends on "logger"
[actors.main_app]
source = { path = "./app" }

[actors.logger]  # ERROR! logger already exists as dependency
source = { path = "./my_logger" }
```

**Fix:** Use actor_overrides to modify the existing dependency:
```toml
[actors.main_app]
source = { path = "./app" }

[actor_overrides.logger]  # Modify the existing logger dependency
source = { path = "./my_logger" }
```

---

## Quick Decision Guide

**"I want to add a brand new actor to my system"**
→ Use `[actors.NAME]`

**"I want to modify how an existing dependency works"**  
→ Use `[actor_overrides.NAME]`

**"I'm not sure if something is a dependency"**
→ Check the `Wasmind.toml` files of your actors

## Troubleshooting Your Configuration

Wasmind performs rigorous checks on your configuration file and all actor manifests before starting. This "fail-fast" approach helps catch issues early and prevents unpredictable runtime behavior. Here are some of the most common errors you might encounter and how to resolve them.

### 1. Circular Dependency

This error occurs when an actor's dependency tree contains a cycle. For example, Actor A depends on Actor B, which in turn depends back on Actor A.

**Why it happens:** Wasmind needs to build a clear, acyclic graph of actors to manage their lifecycle and configuration. A circular dependency creates an infinite loop during this process.

**Example `Wasmind.toml` Files:**

```toml
# /path/to/actor-a/Wasmind.toml
actor_id = "my-co:actor-a"

[dependencies.b_instance]
source = { path = "../actor-b" }
```

```toml
# /path/to/actor-b/Wasmind.toml
actor_id = "my-co:actor-b"

[dependencies.a_instance]
source = { path = "../actor-a" }
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

**Why it happens:** Wasmind cannot determine which version of the shared dependency (`common-tool` in this case) to use.

**Example `Wasmind.toml` Files:**

```toml
# /path/to/app/Wasmind.toml
actor_id = "my-co:app"

[dependencies.parser]
source = { path = "../parser" }

[dependencies.validator]
source = { path = "../validator" }
```
```toml
# /path/to/parser/Wasmind.toml
actor_id = "my-co:parser"

[dependencies.tool]
source = { path = "../../tools/common-tool-v1" }
```
```toml
# /path/to/validator/Wasmind.toml
actor_id = "my-co:validator"

[dependencies.tool]
source = { path = "../../tools/common-tool-v2" }
```

**Expected Error:**
```
Error: Conflicting sources for dependency 'tool' required by 'my-co:app'.
- Path via 'parser' resolves to '.../tools/common-tool-v1'
- Path via 'validator' resolves to '.../tools/common-tool-v2'
```

**How to Fix:** You must resolve the ambiguity. Update the `Wasmind.toml` manifests of the intermediate actors (`parser` and `validator` in this example) to point to a single, canonical version of the shared dependency.

---

### 3. Source Path or Package Not Found

This is a common error when the `source` table in your configuration contains an incorrect `path` or `package` name.

**Why it happens:** Wasmind cannot locate the files it needs to build the actor.

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
*   For path errors, verify the path is correct relative to where you are running the Wasmind application.
*   For package errors, check the `name` field in the `[package]` table of the `Cargo.toml` for the actor you are trying to load. Ensure it matches the `package` value in your configuration.

---

### 4. Missing Actor Manifest

This error occurs when an actor is referenced but doesn't have a `Wasmind.toml` manifest file.

**Why it happens:** Every actor in Wasmind MUST have a `Wasmind.toml` manifest file. This is a strict requirement with no exceptions or fallbacks.

**Example User Configuration:**
```toml
[actors.my_actor]
source = { path = "./actors/my_actor" }
# But there's no Wasmind.toml at ./actors/my_actor/Wasmind.toml
```

**Expected Error:**
```
Error: Actor 'my_actor' at 'path: ./actors/my_actor' is missing required Wasmind.toml manifest file. 
All actors must have a Wasmind.toml file that declares their actor_id.
```

**How to Fix:** Create a `Wasmind.toml` file in the actor's directory with at least the required `actor_id` field:
```toml
# ./actors/my_actor/Wasmind.toml
actor_id = "my-namespace:my-actor"
```

For Rust workspaces using the `package` field, ensure the `Wasmind.toml` is at the specified package path:
```toml
# If your config has:
[actors.my_workspace_actor]
source = { path = "./my_workspace", package = "crates/my_package" }

# Then Wasmind.toml must be at: ./my_workspace/crates/my_package/Wasmind.toml
```

---

### 5. Global Override for Non-existent Actor

This is handled gracefully by the system. If you define a global actor that is never actually used as a dependency, it simply won't be loaded.

**Example User Configuration:**
```toml
# This actor is defined but never used by any other actor
[actors.unused_tool]
source = { path = "./actors/some_tool" }
auto_spawn = false  # Since it's not auto_spawn and not a dependency, it won't load
```

**Behavior:** The system will parse this configuration but won't load the actor unless:
- It's listed in `starting_actors`
- It has `auto_spawn = true`
- It's a dependency of another actor that gets loaded
- It's in another actor's `required_spawn_with` list

**Note:** This is not an error - it's a feature that allows you to pre-configure actors that might be used conditionally.
