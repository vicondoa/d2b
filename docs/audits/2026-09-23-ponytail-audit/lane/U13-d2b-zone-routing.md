# U13 d2b-zone-routing
net: -60 lines, -0 deps

- shrink The crate's redacted-Debug mechanism is written three times: two byte-identical 16-line `macro_rules!` (`redacted_service_debug!` src/service.rs:116, `redacted_topology_debug!` src/resolver.rs:50, differing only in the macro name) plus 11 hand-rolled `impl Debug` bodies each writing `write_str(concat!(stringify!(..), "(<redacted>)"))` or `write_str("<Type>(<redacted>)")` (src/service.rs ×3, src/serving.rs ×1, src/enrollment.rs ×5, src/engine.rs ×2). Fold onto one crate-internal `redacted_debug!` used by every file: delete the resolver.rs copy and the 11 hand-written impls, replacing each hand-rolled body with a one-line macro invoke. (leaf)

## Consistency notes
- N/A: this is a runtime/engine crate (U13 is not a types-layer crate U2-U11), so no topology-duplication feed is due. The redaction Debug impls here are the crate's own privacy posture and are correctly **not** being deleted by surface removal; they stay as macro invokes.

## Reopened refusals
- none (no prior findings listed against U13 in the refusal ledger).

## Checked
Verified the redaction Debug impls and their Debug macros across src/{service,resolver,serving,enrollment,engine}.rs. Byte-compared the two 16-line macro bodies after name-normalization: identical (diff clean), so resolver.rs:50-65 is a pure byte-identical copy. Counted the hand-rolled `<redacted>` Debug impls per file with grep (11 impls, exact line spans listed above) — they are the macro-equivalent bodies the crate's own macro was written to produce but that were left hand-rolled. Swept `MAX_ZONE_REPLAY_KEYS`, `ZoneRouteInventoryEntry`, `route_inventory`, `prune_expired`, `admit_relay_hop`, and the `decide_*`/`admit_*` engine methods for out-of-crate callers across packages/, tests/, benches/, and the nixos-modules tree: engine.rs public methods have real callers (router.rs, resolver.rs, serving.rs, tests, benches, and `d2b-zone-routing/src/serving.rs` from other crates at `route_inventory=1`, `prune_expired=5`, `admit_relay_hop=1`-level usage), so none are zero-caller; no dead public surface flagged. `src/generated/` not present in this crate. No correctness/security/perf items considered (out of scope).
