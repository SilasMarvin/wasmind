# Installation

This page covers installing Wasmind and its dependencies for different use cases.

## For Users: Running Wasmind Configurations

If you want to **use Wasmind** to run AI agent configurations with the CLI:

### 1. Install Rust

```bash
# Install rustup if you haven't already
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Update to latest stable
rustup update stable
rustup default stable
```

### 2. Install the Wasmind CLI

```bash
cargo install --locked wasmind_cli
```

> **Note**: The `wasmind_cli` is just one frontend to the Wasmind library. Other applications may build different interfaces using the core `wasmind` library.

### 3. Install Docker

The `wasmind_cli` uses <a href="https://litellm.ai/" target="_blank">LiteLLM</a> via Docker to provide unified access to AI models:

- **macOS/Windows**: Install <a href="https://www.docker.com/products/docker-desktop/" target="_blank">Docker Desktop</a>
- **Linux**: Install Docker Engine from your package manager

> **Note**: Other frontends/binaries using the Wasmind library may have different LLM integration requirements. This Docker requirement is specific to `wasmind_cli`.

### 4. Install cargo-component

```bash
cargo install --locked cargo-component
```

This is required because Wasmind currently builds all actors locally. This requirement may change in future versions.

### 5. Verify Installation

```bash
wasmind_cli --help
docker --version
cargo component --version
rustc --version  # Should be 1.70+
```

You should see help output and version numbers for all commands.

## For Developers: Building Custom Actors

If you want to **build custom actors** or extend Wasmind:

> **Note**: Follow all the steps in the "For Users" section above first - the installation requirements are the same. `cargo-component` automatically installs the necessary WebAssembly targets.

## Next Steps

After installation:
- **Users**: Continue to the [User Guide](./user-guide/) to run your first configuration
- **Developers**: Continue to the [Developer Guide](./developer-guide/) to build your first actor