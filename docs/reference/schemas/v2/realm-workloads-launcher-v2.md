# `realm-workloads-launcher-v2.json` compatibility schema

This generated schema documents the argv-free launcher metadata artifact
retained for current daemon bridge compatibility. It is daemon-served
metadata, not a lifecycle authority.

Current launch requests carry a typed Zone/Guest target, item ID, and
operation ID. Provider configuration, credentials, argv, and private runtime
locators remain inside the owning execution context.

Regenerate the JSON schema from the Rust DTOs:

```bash
bazel run //packages/xtask:xtask -- gen-schemas
```
