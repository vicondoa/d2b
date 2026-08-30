# `allocator.json` compatibility schema

This generated schema documents private allocator metadata retained by the
daemon bridge. It is not a current Realm, environment, or host-mutation API.

Current allocation is Zone-scoped and broker-owned. Resource controllers
request typed effects through assigned Providers; the broker resolves private
paths, locks, cgroups, devices, and leases and preserves foreign state.

Regenerate the JSON schema from the Rust DTOs:

```bash
bazel run //packages/xtask:xtask -- gen-schemas
```
