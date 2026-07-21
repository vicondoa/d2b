# Reference: per-realm secrets lifecycle

> Component: W8 `secrets-lifecycle`. Owns
> `packages/d2b-priv-broker/src/ops/secrets_lifecycle.rs`,
> `packages/d2b-priv-broker/src/ops/secrets_rotation_audit.rs`, and
> `packages/d2b-sk-frontend/src/secrets_channel.rs`. This is the
> storage/atomicity/rotation-state layer only — see "Non-goals" below
> for what it deliberately does not do.

This page is the contract for the provision/rotate/rollback/retire
engine that tracks per-realm secrets material: TPM-bound credentials,
guest signing keys, and security-key channel state (the closed
[`SecretKind`] set). It follows the broker's existing fd-relative,
anchored, atomic, identity-bound, fail-closed conventions (the same
family as `swtpm_dir.rs`'s hardening step and `store_sync_audit.rs`'s
typed audit schema), applied generically across all three secret
kinds through one shared engine rather than three separate ones.

## Scope and invariants

- **Generic across kinds.** [`SecretKind::TpmBoundCredential`],
  [`SecretKind::GuestSigningKey`], and
  [`SecretKind::SecurityKeyChannelState`] all flow through the same
  `provision`/`rotate`/`rollback`/`retire` functions, parameterized by
  [`SecretsLifecyclePaths`] (derived per `(vm_id, kind)` via
  `derive_paths`). Generating the raw material itself — running
  `ssh-keygen`, deriving a TPM attestation blob, or producing
  channel-binding wire material — stays the caller's concern; this
  engine only owns storage, atomicity, and rotation-state tracking.
- **Fd-relative and anchored.** Every filesystem mutation goes through
  `crate::sys::path_safe`'s existing primitives (`ensure_dir`,
  `open_at`, `mkdir_at_exclusive`, `create_file_at_safe`,
  `atomic_replace_fd_with_owner`, `remove_path_safe`, `fstat_fd`,
  `fstatat_nofollow`) — no bare `std::fs` path-string call ever
  touches a role-writable directory.
- **Atomic generation activation.** Each `(vm, kind)` has an on-disk
  layout of `<per_vm_state_root>/secrets/<kind-slug>/generations/<n>/material`
  plus a `current` symlink, atomically swapped with a hidden-name
  symlink-then-`renameat` sequence directly against the already-open
  `secrets_dir` fd (the same rename-based atomic-swap idiom used
  elsewhere in the broker/host, reimplemented locally here rather than
  reusing `d2b_host::hardlink_farm::swap_current_symlink`, which is
  coupled to that module's own `marker.json` schema).
- **Identity-bound tamper guard.** A dedicated marker file per
  `(vm, kind)` under `/var/lib/d2b/secrets-lifecycle-markers/<vm_id>/<kind-slug>`
  (root-owned, `0600`, JSON) records the active generation number plus
  the active generation directory's `st_dev`/`st_ino`. `rotate`,
  `rollback`, and `retire` all re-verify that identity against the
  live directory before mutating, so a TOCTOU replacement of the
  active generation's content is caught rather than silently rotated
  or retired forward. This marker tree is independent of
  `swtpm_dir.rs`'s own per-VM marker
  (`/var/lib/d2b/swtpm-markers/<vm>`) — this component never reads or
  writes that file.
- **Fail-closed on drift, not just on error.** Mirroring the
  `swtpm_dir.rs` philosophy:
  - `provision` refuses (`already-provisioned`) if active material
    already exists;
  - `provision` fails closed (`previously-provisioned-material-missing`)
    if the marker says active but the generation vanished;
  - `provision` fails closed (`marker-tampered-or-missing-material`) if
    material exists on disk with **no** matching active marker — never
    silently adopted;
  - `rotate`/`rollback`/`retire` all fail closed
    (`marker-tampered-or-missing-material`) if the live active
    generation's identity does not match the marker.
- **Bounded retention.** Exactly one previous generation is retained
  at a time (tracked by the marker's `generation`/`previous_generation`
  pair, never by directory enumeration). `rotate` prunes anything older
  than the immediately-prior generation on a best-effort basis (a
  prune failure never rolls back an already-committed rotate).
  `rollback` restores the immediately-prior generation and fails
  closed (`no-rollback-target`) if none is tracked. `retire` is
  idempotent and removes every generation the marker still tracks.
- **Redaction.** No public function in this module ever returns or
  logs a raw path, a raw secret byte, or a raw `io::Error` message.
  Errors are a closed-set `&'static str` reason slug
  (`secrets_lifecycle::reasons::*`) plus a fully-formed
  [`SecretsLifecycleAuditFields`] record. That record itself never
  carries a path (only an FNV1a-64 `base_dir_hash`, parity with
  `crate::ops::hosts::stable_hash_str`) or raw material (only a
  SHA-256 `material_digest_hex` fingerprint). [`SecretMaterial`]'s
  `Debug` impl never prints its bytes, and the bytes are held in a
  `zeroize::Zeroizing<Vec<u8>>` so they are wiped on drop.
- **Guest-side channel state** (`secrets_channel.rs`, in
  `d2b-sk-frontend`) mirrors the same lifecycle shape in memory only:
  [`ChannelState::provision`]/`rotate`/`retire`/`current`, with
  `rotate` refusing a non-strictly-increasing generation
  (`stale-reconnect-generation`) as a replay/rollback defense. It is a
  standalone module with zero dependency on the rest of that crate
  (only `std`), so it can be validated independently of the guest
  session/transport wiring.

## Non-goals

- This component does not add a new broker op enum family, edit
  `runtime.rs`/`lib.rs`, or add a new wire request/response DTO. Those
  are explicit integration follow-ups (see below).
- This component never touches `swtpm_dir.rs`'s physical TPM NVRAM
  state, `security_key.rs`'s CTAPHID transport, or any
  `guest_material_*` file directly. `SecretKind::TpmBoundCredential`
  tracks rotation/retirement bookkeeping layered *atop* swtpm's own
  identity; it is not a replacement for `swtpm_dir.rs`.
- This component does not decide *how* material is generated for any
  kind (ssh-keygen invocation, TPM attestation derivation, or the
  exact security-key channel wire encoding). It only stores, activates,
  and tracks generations of whatever `SecretMaterial` bytes a caller
  supplies.

## Audit record shape

[`SecretsLifecycleAuditFields`] (schema version 1) is a path-free,
material-free JSON-serializable record: `vm_id`, `kind`, `action`
(`provision`/`rotate`/`rollback`/`retire`), `base_dir_hash`, `result`
(`created`/`rotated`/`rolled_back`/`retired`/`verified_clean`/`denied`/
`failed_closed`), `marker_result`
(`created`/`verified`/`tombstoned`/`unchanged`/`failed_closed`), an
optional `generation`, a bounded `retained_generations` list (at most
`MAX_AUDITED_RETAINED_GENERATIONS`, currently 8), an optional
`material_digest_hex` (present only for `provision`/`rotate`), and an
optional `fail_reason` (present exactly for `denied`/`failed_closed`
results). `SecretsLifecycleAuditFields::validate` enforces every
cross-field invariant listed above; every constructor
(`provisioned`/`rotated`/`rolled_back`/`retired`/`verified_clean`/
`denied`/`failed`) returns an already-valid record.

## Marker and path layout

```
<per_vm_state_root>/secrets/<kind-slug>/
  generations/
    <n>/material          # 0600, expected_uid:expected_gid
  current -> generations/<n>

/var/lib/d2b/secrets-lifecycle-markers/<vm_id>/<kind-slug>   # 0600 JSON marker
```

`<kind-slug>` is one of `tpm-bound-credential`, `guest-signing-key`,
`security-key-channel-state` ([`SecretKind::as_slug`]).

## Integration wiring points (not performed by this component)

This component's public functions are ready to call but are not wired
into any shared sink. An integrator still needs to:

1. Add `pub mod secrets_lifecycle;` and
   `pub mod secrets_rotation_audit;` to
   `packages/d2b-priv-broker/src/ops/mod.rs`.
2. Add an
   `OperationFields::SecretsLifecycle(SecretsLifecycleAuditFields)`
   variant (and matching `from_operation_value` arm) to
   `packages/d2b-priv-broker/src/ops/audit_op.rs`.
3. Add new wire request/response DTOs (in `d2b-contracts`) and a
   `runtime.rs` dispatch path calling
   `provision`/`rotate`/`rollback`/`retire` and emitting the returned
   audit record via `crate::audit::AuditLog`. The plan text for this
   component explicitly forbids adding a new broker op enum family
   from within it, so this step is deliberately deferred.
4. Decide what bundle-resolved field supplies `per_vm_state_root` for
   `derive_paths` (mirrors `swtpm_dir::derive_paths`'s
   `&SpawnRunnerPlan` parameter).
5. Decide whether `SecretKind::GuestSigningKey` material comes from
   `exec_reconcile::run_ssh_keygen` output fed into `rotate`'s
   `material` parameter, or stays separate.
6. Decide the exact coupling between `SecretKind::TpmBoundCredential`
   rotation and `swtpm_dir.rs`'s physical NVRAM (e.g. whether a rotate
   here should also trigger a swtpm reseal) — a product/security
   decision beyond this component's scope.
7. Add `pub mod secrets_channel;` to `packages/d2b-sk-frontend/src/lib.rs`,
   and wire `services/security_key/mod.rs`'s `SessionConfig` to source
   its `channel_binding`/`reconnect_generation` from
   `ChannelState::current`/`ChannelMaterial::into_session_config_args`
   instead of the static `D2B_SK_CHANNEL_BINDING_HEX`/
   `D2B_SK_RECONNECT_GENERATION` environment variables it reads today.
8. Confirm or replace `ChannelMaterial::from_wire_bytes`'s proposed
   32-byte-binding + 8-byte-big-endian-generation wire format against
   whatever the future broker dispatch arm for
   `SecretKind::SecurityKeyChannelState` actually serializes — it is a
   proposal, not a settled contract.
9. Run `gen-nix-unit-pins.sh` after this component lands so
   `tests/unit/nix/pinned/*.txt` picks up the new
   `w8-secrets-lifecycle-eval.nix` case names (not run here since the
   pinned files are not owned by this component).

## See also

- [How to rotate secrets](../how-to/rotate-secrets.md)
- [`docs/reference/cgroup-delegation.md`](./cgroup-delegation.md) —
  sibling reference doc style this page follows.
- `packages/d2b-priv-broker/src/ops/swtpm_dir.rs` — the physical TPM
  NVRAM state this component's `TpmBoundCredential` kind layers atop
  without editing.
- `packages/d2b-priv-broker/src/ops/store_sync_audit.rs` — the
  existing standalone-audit-schema precedent this component's
  `secrets_rotation_audit.rs` mirrors.
