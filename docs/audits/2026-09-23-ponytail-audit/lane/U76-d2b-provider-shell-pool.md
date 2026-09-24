# U76 d2b-provider-shell-pool

Interaction-family member crate (~172 LOC). The crate owns `ShellPool` and its driver; the driver verbs come from the shared interaction-family engine in `d2b-provider-wayland-policy`. This is a family-surface crate, not a types/catalog crate - the Consistency notes section below is therefore **omitted** (it is types-layer-only), but the crate is included in the working consistency feed to U97 via the family note.

## Findings (biggest cut first)

- `refuse` **Cross-cutting: per-type declaration boilerplate in the interaction family** (#S3). Measured ~200 lines across the six interaction-family crates, only ~70 safely removable; shell-pool's own `*_spec_decoder()` wrapper (here: `shell_pool_spec_decoder()`) is consumed by the crate's registration test cleanup arm and its `shell_pool_descriptor()`/`ShellPoolDescriptor<%>` aliases are the driver surface the family declares. Refused per family row. [packages/d2b-provider-shell-pool/src/lib.rs + shell_pool.rs]
- `refuse` **Cross-cutting: dead per-type constants no caller reads** (#S4). The six paired `*_RESYNC` consts are each the return value of their own `InteractionType::resync()` (wayland-policy/src/interaction.rs:795); here `SHELL_POOL_RESYNC` is that return value for `ShellPool`, and it is also the crate's exported resync arm. The paired `*_CONTROLLER_REF` half was already deleted with the prior family cut. [packages/d2b-provider-shell-pool/src/shell_pool.rs]
- `refuse` **Cross-cutting: spec_ref duplicated in four crates** (#S5). No importable shared pointer-ref parser in scope - interaction engine exposes only key_ref/owned_child_ensure/resource_uid; the shell-pool copy stays. [packages/d2b-provider-shell-pool/src/shell_pool.rs]

## Consistency notes

Family-surface crate - the interaction-family note supersedes the local notes section. The family's engines stay outside the shell-pool lane (owned at wayland-policy, crosses the wayland-session/audio/shell family); the crate's verbs come from the shared engine and its descriptor/decoder arms are the family driver declaration.

## Checked

Read `shell_pool.rs` and `lib.rs`; verified `*_spec_decoder()` and the `ShellPool` descriptor arms are live family surface (the driver), RFC 4648/other hand-rolled wire rules absent from this crate (wire codecs live in d2b-core's base64_codec, already reported there). No zero-caller surface within this crate beyond the family rows above; nothing new to cut here.

**Net:** 0 lines, 0 deps. (Family rows #S3/#S4/#S5 refused/partial per ledger - lean already at the crate level.)
