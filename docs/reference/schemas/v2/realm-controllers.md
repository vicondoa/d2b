# `realm-controllers.json` compatibility schema

This generated schema documents a private transitional artifact. It is not a
current Realm hierarchy or lifecycle authority.

Current Nix authoring is `d2b.zones.<zone>.resources.<name>`. d2bd resolves
Zone Resources and the Guest controller owns direct child lifecycle. The
compatibility artifact may be loaded by the daemon bridge, but it cannot
create, discover, or authorize a current Guest.

Regenerate the JSON schema from the Rust DTOs:

```bash
bazel run //packages/xtask:xtask -- gen-schemas
```
