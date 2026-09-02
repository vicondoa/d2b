# `realm-identity.json` compatibility schema

This generated schema documents private transitional identity metadata. It
contains no secret bytes and is not a current authorization or credential
source.

Current identity and credential authority belongs to Zone Resources,
authenticated sessions, and Provider generations. The daemon and broker fence
all mutations by exact Zone/Guest identity, UID, generation, and revision.

Regenerate the JSON schema from the Rust DTOs:

```bash
bazel run //packages/xtask:xtask -- gen-schemas
```
