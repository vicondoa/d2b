# U50 d2b-provider-volume-local

net: -0 lines, -0 deps already. Ship.

Crate re-audited after prior U50 pass (S51–S61). Every prior rejection and partial documented with its customer evidence re-confirmed in-tree; no new material.

## Findings (new)

- <refusal> `yagni` nix/store.nix + nix/sync-json.nix "orphaned" (S53 refused). [packages/d2b-provider-volume-local/nix/] — still both loaded: `bazel/checks/nix/BUILD.bazel:55-56` includes `store.nix` and `sync-json.nix` in the storage-volume eval surface; `nix/` today lists both plus zones-volumes/zone-storage variants. Refusal stays.
- <refusal> `shrink` zone-compiler duplicate Volume validator (S58 not applied). Duplicate remains in `nix/resources-zones-volumes.nix:11` (its own `modePattern`, already drifted from `resources-volume.nix:23`); no commit touched the crate Nix dir since. Refusal stays.
- <refusal> `shrink` `/docs/d2b-provider-volume-local` finalization/marker/scaling module notes (S61 method note, S53/S58 collateral). No new evidence passes; stays refused.

## Checked

- Re-read lib.rs (the 82-visible-surface re-export block in full) and cross-checked every re-exported symbol against caller search across `packages/` + `d2bd/` + `d2bd/tests` (workspace-wide rustasis grep, then fine-grained HTML ranged re-reads: adapter.rs 1-30/73.4KB, bindings.rs 1-80, source.rs 1-271 and quota.rs 1-40, views.rs 1-20, acl.rs, layout.rs, port.rs, store_view.rs, identity.rs, marker.rs).
- Verified the S51/S54/S55/S56/S59 deletions are applied: no migration/relocation/sealing/snapshot planner files, no src/path.rs, no store-view validator modules, no swtpm-volume policy module, no test-only ACL planner (acl.rs 342->241 done), no `src/generated/`. Marker/source/content/adapter/lock/atomic/views/tag_envelope/dtos all remain with live callers.
- Confirmed the two refused nix artifacts still load at bazel/checks/nix/BUILD.bazel:55-56 (visible in workspace BUILD output) and the zone-compiler duplicate still lives in the crate's nix/ directory, which prior pass left untouched. No new finding beyond them.
- Ledger-honored: zero-caller claims verified workspace-wide; no false positives surfaced against live callers (`desired_binding_intents` at d2bd/src/resource_runtime.rs:6085, `admit_quota` at controller.rs:515, `desired_binding_constraints` at d2bd/src/composition.rs:1633 etc.).
