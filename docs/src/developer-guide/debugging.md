# Debugging

## Viewing Message Flow

To see all messages flowing through the system, set the `WASMIND_LOG` environment variable:

```bash
# Debug level shows all messages
WASMIND_LOG=debug wasmind_cli

# Other log levels
WASMIND_LOG=info wasmind_cli   # Default
WASMIND_LOG=warn wasmind_cli   # Warnings and errors only
WASMIND_LOG=error wasmind_cli  # Errors only
```

## Memory Constraints

WebAssembly components have a **4GB memory limit** due to 32-bit addressing (WASM uses i32 for memory addresses). Watch out for:

- **Out of Memory (OOM) errors** - Large file operations or data processing can hit the 4GB limit
- **Large message payloads** - Broadcast messages go to all actors, consuming memory

If your actor crashes unexpectedly, check for OOM issues by looking for memory-related errors in the logs.

## Actor Crashes

If `wasmind_cli` shows that an actor's main function errored, check the logs for detailed error messages. The logs will contain the actual error that caused the crash, including stack traces and error descriptions.

## More Coming Soon

This section will be expanded with additional debugging strategies and troubleshooting guides.
