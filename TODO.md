1. Update the system_state.rs tets to test that the task gets rendered properly.
2. First read in the LLM.md file to understand the codebase better. Your task is to fix the logging and log    │
│   parsing. The log parser is used in the integration tests. It can be found in tests/log_parser/mod.rs The    │
│   goal is to decode structured messages we log when running in debug mode. You can find these messages being  │
│   logged by searching for "debug!". We don't care about decoding all debug logs but we want to be able to     │
│   decode `Message` in the src/actors/mod.rs file and the `InterAgentMessage` in src/actors/agent.rs IDEALLY   │
│   we should NOT redefine these enums in the log_parser. We should only define one enum like                   │
│   `StructuredMessages {}` that has two variants, one for each message type we support parsing and then we     │
│   can use serde to deserialize into them.
