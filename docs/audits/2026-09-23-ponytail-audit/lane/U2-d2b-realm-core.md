# U2 d2b-realm-core
net: -22 lines, 0 deps   (Lean already on dead surface — the crate is a live types/admission surface with zero unreferenced pub items found; the two cuts below are within-crate byte-identical-helper consolidations)

- shrink Two byte-identical copies of `deserialize_bounded_vec` (bounded-array serde helper, 12 lines each) — routing.rs:44 and access.rs:226. Keep one copy; the other module references the shared one. [packages/d2b-realm-core/src/routing.rs:44, packages/d2b-realm-core/src/access.rs:226] (family)
- shrink Two byte-identical copies of `bound_message` (UTF-8 char-boundary message truncator, 10 lines each) — error.rs:260 and migration.rs:206. Keep one; the other calls the shared one. [packages/d2b-realm-core/src/error.rs:260, packages/d2b-realm-core/src/migration.rs:206] (family)

## Consistency notes
- `deserialize_bounded_vec` is duplicated byte-identically in `routing.rs:44` and `access.rs:226` — no shared home exists; crate has no `bounded`/`util` module to host it. Canonical home: a new crate-private shared module (or keep `routing.rs:44` as canonical and have `access.rs` re-reference). No wire-shape divergence — both enforce the same bounded-vec contract, so this is pure copy consolidation, not a shape change.
- `bound_message` is duplicated byte-identically in `error.rs:260` and `migration.rs:206`; the crate also has `bound_fingerprint` (error.rs:271) in the same family. Canonical home alongside the above. Same char-boundary truncation semantics in both — no drift.
- No duplicate type definitions or naming drift found across modules beyond these two helper families. Phantom `bind_message`-named third copies (frame.rs) are absent — confirmed `rg -l` shows exactly routing.rs and access.rs for the vec helper and exactly error.rs and migration.rs for the message helper. Hand-written `Deserialize` for bounded shapes is the correct fail-closed admission pattern here (bounded wire discipline); only the duplication is the issue.

## Reopened refusals
(none — U2 was clean, no prior refusals in ledger)

## Checked
- Read full error.rs, migration.rs, node.rs, registry.rs, payload.rs, execution.rs, stream.rs, identity_config.rs, frame.rs; structural reads of access.rs, routing.rs, route_engine.rs, allocator.rs, allocator_engine.rs, audit.rs, enrollment.rs, identity_store.rs.
- Searched crate-wide for dead pub surface (grep on `pub fn`/`pub struct` with zero external references), all live.
- Verified the two helper duplicates with byte-identical normalized-body hashes (routing == access vec helper, error == migration message helper; confirmed via LOC + hash, and frame.rs does NOT carry a third copy).
- Checked no other helper family (deserialize_bounded_set, refecoder variants) is triplicated — the set and refecoder functions are single-owner.
