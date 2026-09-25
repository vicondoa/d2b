# xtask-p4 - xtask - part 4/5
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 9015 (excl. src/generated/**) | modules: delivery/storage, delivery/model, delivery/mod, production_closure, zone_schema, operation_row_authority, inventory
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: packages/xtask/src/delivery/storage.rs, packages/xtask/src/production_closure.rs, packages/xtask/src/zone_schema.rs, packages/xtask/src/delivery/model.rs, packages/xtask/src/operation_row_authority.rs, packages/xtask/src/inventory.rs, packages/xtask/src/delivery/mod.rs

## idiom
- clean: seeds ran: 0/0/39; every `let mut ... = String|Vec::new()` hit is a state-machine parser, an error collector, an fd-walk chain,a bounded read buffer, or a Nix/JSON text builder where an iterator collect would not be clearer or would split a single multi-output pass.

## own
- xtask-p4#1 sev=low blast=leaf effort=S verdict=actionable - `compute_context(root, spec)` takes `ContextSpec` by value,so both caller loops clone the spec they still need afterwards - fix: change `fn compute_context(root: &Path, spec: ContextSpec)` (and the `compute_lock_context` recursion at production_closure.rs:431) to take `spec: &ContextSpec`,removing the `.clone()` at both loop call sites - [packages/xtask/src/production_closure.rs:263, packages/xtask/src/production_closure.rs:379]
 evidence: own seed 1 `\.clone()` = ~47 hits; sites 263/379 are loop-boundary clones where the caller reads spec again after the call (spec.key(), spec.system, spec.target, spec.name, or the surviving `&contexts` for `write_advisory_skeleton`).
- xtask-p4#2 sev=low blast=leaf effort=S verdict=actionable - `check_outputs` binds `ApprovalProjection` twice in a row,but the first binding is never read after the second clone - fix: replace `let approval = advisory.approval.clone();` followed by `Some(approval.clone())` with one `Some(advisory.approval.clone())` - [packages/xtask/src/production_closure.rs:383, packages/xtask/src/production_closure.rs:386]
 evidence: own seed 1 `\.clone()` = ~47 hits; the clone at :383 is consumed only by the clone at :386,and nothing else in the loop body reads `approval`.

## type
- xtask-p4#3 sev=medium blast=leaf effort=S verdict=actionable - inventory.rs re-implements the crate's own `delivery::model::validate_repo_relative_path` with the same invariant (minus the empty-path check), so two validators drift apart - fix: delete the private copy at inventory.rs:230,and call `crate::delivery::model::validate_repo_relative_path(Path::new(path))`, keeping the stricter empty check - [packages/xtask/src/inventory.rs:230, packages/xtask/src/delivery/model.rs:557]
 evidence: type seed 1 `fn validate_\w+|fn check_\w+` = 14 hits; census: `validate_repo_relative_path` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 14 hits (model's pub helper already serves delivery/snapshot.rs; inventory carries its own private copy).
- xtask-p4#4 sev=low blast=leaf effort=M verdict=needs-contract - `EdgeRecord.kind: String` carries the closed Cargo dependency-kind vocabulary {"normal","build","dev","proc-macro"} as a free string through traverse/filter/emit - fix: introduce a closed `EdgeKind` enum parsed once at the metadata boundary (dep_kinds reads at :660-676), serde-renamed to preserve the wire spelling "proc-macro",and regenerate the committed packages/policy-inputs/** closures - [packages/xtask/src/production_closure.rs:107, packages/xtask/src/production_closure.rs:660, packages/xtask/src/production_closure.rs:724]
 evidence: type seed 3 `(mode|kind|state): String` is 1 site (EdgeRecord.kind); the vocabulary is closed per `production_kinds()`/`policy_kinds()` sets (lines 74-82),and `package_is_proc_macro()`; the emitted closure.json projections are committed generated shapes,so the change is needs-contract.

## api
- clean: seeds ran: 45/0/4; xtask is bin-only (no `src/lib.rs`, no `[lib]` target, `default-run = "xtask"`), so pub and re-exported items are crate-internal and no external API contract exists to judge or break.

## err
- clean: seeds ran: ~80/7/2/1; every surviving unwrap/expect site is cfg(test), an invariant expect on an already-checked map lookup,a literally-built JSON value, or `write!` to String; the `let _ =` sites are documented best-effort Drop cleanups; the two panic sites are tests; the one enum hit is `DeliveryErrorKind` (the kind half of the already-correct struct-with-private-kind pattern).

## serde
- xtask-p4#5 sev=low blast=leaf effort=S verdict=actionable - `SnapshotView` (mod.rs:46-47) lacks `#[serde(deny_unknown_fields)]` while every nested wire type in the same artifact (CandidateMaterial, RepositoryRecord, Fingerprint, DependencyEdge, digest newtypes) denies, so a hand-edited snapshot can carry silently-ignored top-level keys - fix: add `#[serde(deny_unknown_fields)]` to SnapshotView; the `schema_version` gate already handles version drift,so there is no forward-compat cost - [packages/xtask/src/delivery/mod.rs:46, packages/xtask/src/delivery/model.rs:111]
 evidence: serde seed 2 `#[serde(...)]` attr scan = ~30 attr sites; SnapshotView is the only Deserialize-wire type in the lane without an attr (every sibling denies at model.rs:111-112, 144-145, 234-235, 244-245, 269-270, 305-307).

## obs
- clean: seeds ran: 4/0/0/0; all four println!/eprintln! sites are CLI product output per the cli-contract (result JSON on stdout, diagnostics on stderr); no tracing/log, instrument, or interpolated log macros exist in these modules.

## docs
- clean: seeds ran: 45/0/60; xtask is bin-only per the skill's own carve-out,and every public contract-bearing item (StateRoot, CandidateDir, model wire types, SnapshotView, DeliveryError/DeliveryErrorKind) carries full API docs;# Errors sections are N/A on crate-internal Result fns.

## perf
- clean: seeds ran: ~38/~40/4; every site is a cold one-shot CLI path, an error diagnostic, or a deliberate Nix/JSON artifact text builder (recorded false-positive classes); no hot loop allocates,and no benchmark exists (static, unmeasured).

## conc
- clean: seeds ran: 1/2/7/2; the only production sync site is the `Relaxed` atomic temp-suffix counter (recorded atomics-as-counters class); all other sync sites are cfg(test) race-hook/override machinery (test-only synchronization class).

## async
- N/A (seeds: 0/0/0/0 all zero; no async fns, awaits, spawns, runtimes, or tokio sync guards exist in these files).

## unsafe
- N/A (seeds: 0/0/2-false-positive/0; the two `from_raw` hits are safe `rustix::fs::FileType::from_raw_mode` conversions, not unsafe ops; no unsafe block/fn/impl or `// SAFETY:` comment exists in scope,and the crate's `unsafe_code = "forbid"` lint setting is untouched).

## ffi
- N/A (seeds: 0/0/0/0 all zero; no `extern "C"`, `no_mangle`, `catch_unwind`, `reprC()`/`repr(transparent)`, or CStr/CString/c_char surface exists; rustix/nix syscall wrappers never cross a foreign caller).

## macro
- clean: seeds ran: 1/1/0; the sole `digest_identifier!` macro is impl-per-type generation for the three digest newtypes (one of the skill's three genuine macro uses), with ident/literal fragment specifiersand  no external paths to shadow; the one proc_macro/syn hit is the fn name `package_is_proc_macro` (false positive).

## test
- clean: seeds ran: 50/~150/0/0; tests are behavioral fixture-driven unit tests (parity/drift gates, digest identity matrix, path-safety matrix, exit-code contract), table-driven where apt; no ignored or tautological tests found.

## Coverage
- idiom: clean(seeds ran: 0/0/39)
- own: 2 finding(s)
- type: 2 finding(s)
- api: clean(seeds ran: 45/0/4)
- err: clean(seeds ran: ~80/7/2/1)
- serde: 1 finding(s)
- obs: clean(seeds ran: 4/0/0/0)
- docs: clean(seeds ran: 45/0/60)
- perf: clean(seeds ran: ~38/~40/4)
- conc: clean(seeds ran: 1/2/7/2)
- async: N/A (seeds: 0/0/0/0 all zero; no async code)
- unsafe: N/A (seeds: 0/0/2-false-positive/0; no real unsafe sites)
- ffi: N/A (seeds: 0/0/0/0 all zero; no FFI surface)
- macro: clean(seeds ran: 1/1/0)
- test: clean(seeds ran: 50/~150/0/0)
