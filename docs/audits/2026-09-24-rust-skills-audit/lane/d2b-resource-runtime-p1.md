# d2b-resource-runtime-p1 - d2b-resource-runtime - part 1/2
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 7759 (excl. src/generated/**) | modules: manager, resource, error, provider, metadata, revision, lib, schema
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 1/2: src/manager.rs, src/resource.rs, src/error.rs, src/provider.rs, src/metadata.rs, src/revision.rs, src/lib.rs, src/schema.rs (part 2 owns src/context.rs, src/target.rs, src/guest_target.rs, src/spec_store.rs, src/watch.rs, src/driver.rs, src/identity.rs)

## idiom
- d2b-resource-runtime-p1#1 sev=low blast=leaf effort=S verdict=actionable - hand-written `impl Default` on the unit structs `ResourceManager` and `ResourceActor` delegate to `new()` where `#[derive(Default)]` is equivalent, and neither impl has any caller - fix: derive `Default` on both (or delete the impls; `new()` stays) - [packages/d2b-resource-runtime/src/manager.rs:882, packages/d2b-resource-runtime/src/resource.rs:685]
  evidence: seed `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 2 hits (both sites); census: `ResourceManager::default|ResourceActor::default` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 0 hits
- d2b-resource-runtime-p1#2 sev=low blast=leaf effort=S verdict=actionable - `ManagerActorEndpoint::rpc` and `ResourceManagerClient::rpc` are byte-identical 9-line request/reply helpers duplicated in one file - fix: extract one free `manager_rpc(actor: &ActorRef<ResourceManagerMsg>, build: impl FnOnce(oneshot::Sender<Result<T, ResourceError>>) -> ResourceManagerMsg)` and call it from both impls - [packages/d2b-resource-runtime/src/manager.rs:1419, packages/d2b-resource-runtime/src/manager.rs:1508]
  evidence: seed `let mut \w+ = (String|Vec)::new\(\)` = 1 hit (test helper, not a finding); the two rpc bodies read identical at the cited lines
- clean: seeds ran: 9 (`for \w+ in 0\.\.`, all in test polling loops), 2 (hand-written impls, both flagged), 1 (`let mut ... = Vec::new()`, test helper); no index loops or statement-style accumulation in production code

## own
- d2b-resource-runtime-p1#3 sev=low blast=leaf effort=S verdict=actionable - `ResourceView::observed_status` clones the whole `Option<ResourceStatus>` (which can carry a `DriverFailure` with comparison vectors) before the generation filter, so a stale status is copied and then discarded - fix: `self.status.as_ref().filter(|_| self.status_generation == Some(self.generation)).cloned()` - [packages/d2b-resource-runtime/src/manager.rs:205]
  evidence: seed `\.clone\(\)` = 232 hits (174 in manager.rs); this site clones only to filter by reference
- d2b-resource-runtime-p1#4 sev=low blast=leaf effort=S verdict=actionable - `ResourceActor::pre_start` clones `args.row` twice (once into the context, once into `state.row`) where one move and one clone suffice - fix: move `args.row` into `ResourceActorState.row` and clone it only for `ResourceContext::new` - [packages/d2b-resource-runtime/src/resource.rs:714, packages/d2b-resource-runtime/src/resource.rs:732]
  evidence: seed `\.clone\(\)` = 232 hits; both cited clones are of the same `StoredDesiredResource` in one function
- d2b-resource-runtime-p1#5 sev=low blast=leaf effort=S verdict=actionable - `spec_object` returns `Ok(spec.clone())` on an owned `serde_json::Value` where the move `Ok(spec)` is legal (the value is not used after) - fix: drop the `.clone()` - [packages/d2b-resource-runtime/src/metadata.rs:191]
  evidence: seed `\.clone\(\)` = 232 hits; the cited clone copies the whole decoded spec JSON on every metadata validate pass
- clean: seeds ran: 232 (`\.clone\(\)`), 57 (`\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)`), 0 (`Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<|Cow<`); the remaining clones are explainable (map-key copies, Arc clones at spawn boundaries, wire-rendering copies) and the three flagged sites are the avoidable ones

## type
- clean: seeds ran: 2 (`fn validate_\w+|fn check_\w+`, both are test fn names), 0 (`is_\w+: bool|\w+_flag: bool`), 0 (`(mode|kind|state): String`); the `effect_running`/`reconcile_pending`/`deleting` flags in `ResourceActorState` are a documented R14 coalescing hand-off with dedicated tests, not flag soup

## api
- clean: seeds ran: 145 pub items inspected plus the lib.rs re-export arms; `Arc`/`dyn` fields in `ResourceManagerArgs` and `ResourceActorArgs` genuinely share ownership (composition-owned store/hub/targets, manager-owned providers; call sites d2bd/src/resource_plane_v3.rs:3114 and d2b-resource-api/src/manager_backend/tests.rs:234), and the `MutationAdmission` seam is a documented retained extension with `AllowAll` installed by design (module docs manager.rs:31-47)

## err
- d2b-resource-runtime-p1#6 sev=medium blast=leaf effort=M verdict=actionable - `ResourceError::ManagerRpc(String)` collapses caller-distinct failures into one stringly variant: transport failures ("manager channel closed", "manager dropped the request", retryable) and semantic refusals ("owner not known", "refusing re-parent", zone mismatch, permanent) are indistinguishable without string-matching, and drivers that classify a context error (e.g. `drain_owned_children` maps every non-`ChildrenDraining` error to retryable) cannot tell a dead manager from a permanent refusal - fix: split into `ManagerUnavailable` (transport) and `ManagerRejected { reason }` (semantic), or carry a `ManagerRpcKind` enum the variant stores - [packages/d2b-resource-runtime/src/error.rs:773, packages/d2b-resource-runtime/src/manager.rs:1426, packages/d2b-resource-runtime/src/metadata.rs:200]
  evidence: seed `enum \w*Error` = 3 hits; the `ManagerRpc` variant is constructed from 3 distinct failure classes at manager.rs:1426/1515 (transport) and manager.rs:973/1100/1250/1298/1323/1353 (semantic); no caller matches on its content today (census: `ManagerRpc` over packages/ = constructions and wrappers only)
- clean: seeds ran: 210 (`\.unwrap\(\)|\.expect\(`), 24 (`let _ = `), 14 (`panic!|unreachable!|todo!|unimplemented!`), 3 (`enum \w*Error`); the production expects name invariants (resource.rs:757/762 receiver ownership in post_start, revision.rs:112 clock-before-epoch startup precondition) and the `let _ =` sites are deliberate best-effort actor/reply sends

## serde
- clean: seeds ran: 4 (`derive(...)Serialize|serde(...)|impl Deserialize|serde_json::from_|serde_json::to_`); the only production hit is the `metadata_spec_decoder` decode boundary (metadata.rs:70), the wire shapes (`wire_status`, `wire_layer`) are hand-built JSON pinned by the issue #515 contract with shape tests, and the rest are test helpers

## obs
- clean: seeds ran: 0 (`println!|eprintln!`), 2 (`(info|debug|warn|error|trace)!\(|tracing::`); the single production event (resource.rs:442) is a `tracing::warn!` with eight named fields plus the canonical issue #508 log line as message, on a cold error path

## docs
- d2b-resource-runtime-p1#7 sev=medium blast=leaf effort=M verdict=actionable - `ResourceManagerClient`, the crate-root re-exported caller-facing facade consumed by d2bd and the resource API, has 14 undocumented pub methods (`new`, `actor`, `apply`, `ensure`, `remove`, `get`, `list`, `watch`, `get_row`, `list_owned`, `ensure_child`, `register_watch`, `cancel_watch`, `reconcile_children`) with non-obvious contracts (watch gap-free-epoch semantics, ensure-child re-parent refusal, watch routing) - fix: add doc comments with `# Errors` sections naming the `ResourceError` variants each call can return - [packages/d2b-resource-runtime/src/manager.rs:1500, packages/d2b-resource-runtime/src/manager.rs:1519, packages/d2b-resource-runtime/src/manager.rs:1597]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 145 hits; the client methods at manager.rs:1500-1602 carry no `///` lines (verified by read), while the struct doc and every message variant are documented
- d2b-resource-runtime-p1#8 sev=low blast=leaf effort=S verdict=actionable - `ResourceManagerArgs` fields `store`, `providers`, and `backoff` are undocumented while the sibling fields all carry doc comments - fix: one line each (the store is the single-writer spec store, providers the per-type registry, backoff the R13 fixed reconcile backoff) - [packages/d2b-resource-runtime/src/manager.rs:850, packages/d2b-resource-runtime/src/manager.rs:851, packages/d2b-resource-runtime/src/manager.rs:870]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 145 hits; the three cited fields lack `///` lines in the struct at manager.rs:847-872
- d2b-resource-runtime-p1#9 sev=low blast=leaf effort=S verdict=actionable - five `MODULE_NAME` consts (`manager`, `resource`, `error`, `provider`, `revision`) are undocumented while `metadata`'s carries a doc line - fix: add the one-line "module declared name, asserted by the crate smoke test" doc (or fold into the A5 decision on the whole const set) - [packages/d2b-resource-runtime/src/manager.rs:51, packages/d2b-resource-runtime/src/resource.rs:36, packages/d2b-resource-runtime/src/error.rs:32, packages/d2b-resource-runtime/src/provider.rs:3, packages/d2b-resource-runtime/src/revision.rs:37]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 145 hits; the const set is the A5 not-applied row (docs/explanation/over-engineering-audit-record.md:458, no refusal reason) and the sites still match the record
- clean: seeds ran: 145 (pub items), 0 (`/// # (Examples|Errors|Panics|Safety)`), 68 (`-> Result<`); module docs exist for all eight modules, and every public type, enum variant, and constant except the flagged items carries a first-sentence doc

## perf
- d2b-resource-runtime-p1#10 sev=low blast=leaf effort=S verdict=actionable - `reconcile_children`'s obsolete scan clones every owned `StoredDesiredResource` row (spec and metadata byte vectors included) into a `Vec` when only the keys are needed to drive `remove_internal` - fix: collect `row.key.clone()` only, or iterate `state.rows` by reference and call `remove_internal(&subject, &child.key)` - [packages/d2b-resource-runtime/src/manager.rs:1390]
  evidence: static (unmeasured); seed `Vec::new\(\)|HashMap::new\(\)` = 33 hits; the cited `.cloned().collect()` copies full rows per reconcile pass over a parent's owned set
- clean: seeds ran: 13 (`format!`), 33 (`Vec::new\(\)|HashMap::new\(\)`), 25 (`\.to_string\(\)`); the `format!` sites are error paths and the log-line/wire renderers (cold), the collection literals are one-shot state construction, and no allocation sits in a measured hot path

## conc
- d2b-resource-runtime-p1#11 sev=low blast=leaf effort=S verdict=actionable - `ActorTimers.next` is a single-owner counter (ractor serializes the actor's handlers) but increments with `Ordering::SeqCst`, the strongest ordering, where `Relaxed` is the weakest correct one for a counter nobody synchronises on - fix: `self.next.fetch_add(1, Ordering::Relaxed)` - [packages/d2b-resource-runtime/src/resource.rs:247]
  evidence: seed `Atomic\w+|Ordering::` = 151 hits (the rest are test atomics and the correct Acquire/Release gate pair in test_support); the cited counter is only touched by the actor thread
- clean: seeds ran: 0 (`std::thread::|thread::spawn|thread::scope`), 10 (`\bMutex<|\bRwLock<`), 151 (`Atomic\w+|Ordering::`), 0 (`thread_local!|unsafe impl (Send|Sync) for`); the production `Mutex` is the documented plan-U4 `tokio::sync::Mutex` reached via non-blocking `try_lock` from the sync trait surface (resource.rs:249-252), and `ManualClock` uses `Relaxed` correctly

## async
- clean: seeds ran: 642 (`async fn|async move|\.await`), 5 (`tokio::spawn|spawn_blocking|JoinSet|select!|join!`), 5 (`tokio::sync::(Mutex|RwLock|Notify)`), 44 (`#[tokio::(main|test)]|Runtime::block_on`); no blocking call sits in an async context (the store and target calls are async, the only `std::fs` use is in tests), no guard is held across `.await`, the `Box::pin` recursion in `remove_internal`/`retire_row` is depth-bounded by the ownership chain with crash-resume covered by tests, the unbounded effect/watch channels are the documented KTD12 mailbox-freeing design, and the two production `.expect` receiver takes name held-in-state invariants

## unsafe
- N/A: seeds: 0/0/0/0 all zero (`\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern`, `// SAFETY:`, `transmute|from_raw|MaybeUninit|mem::zeroed`, `unsafe_code`); the crate inherits workspace lints with `unsafe_code = "forbid"` (root Cargo.toml `[workspace.lints.rust]`), matching U1 (d) 8's enumerated set

## ffi
- N/A: seeds: 0 all zero (`extern "C"|no_mangle|unsafe\(link_section`, `catch_unwind`, `repr\(C\)|repr\(transparent\)`, `CStr|CString|c_char`); no foreign boundary exists in this crate

## macro
- N/A: seeds: 0 all zero (`macro_rules!`, `proc_macro|syn::|quote!`, `\$crate`, `to_compile_error|new_spanned`); no macros defined or consumed beyond std

## test
- d2b-resource-runtime-p1#12 sev=high blast=leaf effort=S verdict=actionable - `display_shows_epoch_and_sequence` asserts `rendered.contains("[PHONE]")` on the rendering `e1728000000+42`, an assertion that cannot pass, so the test fails at HEAD (route review-pass; read-only audit) - fix: delete the stray `[PHONE]` assertion (the epoch/sequence assertions on the same line already cover the contract) - [packages/d2b-resource-runtime/src/revision.rs:157, packages/d2b-resource-runtime/src/revision.rs:71]
  evidence: seed `assert_eq!\(|assert_ne!\(|assert!\(` = 281 hits; the Display impl at revision.rs:71-75 renders `e{}+{}` with no redaction, and `[PHONE]` appears nowhere else in packages/ (grep over packages/ = 0 hits)
- d2b-resource-runtime-p1#13 sev=low blast=leaf effort=S verdict=actionable - `wire_budget_bounds_sequence_for_u32_low_word` asserts `WIRE_SEQUENCE_BUDGET == 1 << 32`, restating the constant's own definition (revision.rs:41), so it cannot fail meaningfully - fix: delete it or assert a behavioral consequence (e.g. that a sequence at the budget still packs into the u32 low word of the U8 mapping) - [packages/d2b-resource-runtime/src/revision.rs:182]
  evidence: seed `assert_eq!\(|assert_ne!\(|assert!\(` = 281 hits; the expected value is the same literal the const is defined from (revision.rs:41), the skill's same-logic expectation rule
- d2b-resource-runtime-p1#14 sev=low blast=leaf effort=S verdict=actionable - the lib.rs `modules_resolve` smoke test asserts each `MODULE_NAME` const against its own literal, pinning source text with no behavioral value (the A5 not-applied row, docs/explanation/over-engineering-audit-record.md:458, covers the consts and this test; the site still matches the record) - fix: fold into the A5 decision (delete both, or keep only as a compile-resolution check without the value assertions) - [packages/d2b-resource-runtime/src/lib.rs:66]
  evidence: seed `#\[test\]|#\[tokio::test\]` = 64 hits; the test asserts 13 const-vs-literal pairs that cannot diverge from the same-file definitions
- clean: seeds ran: 64 (`#\[test\]|#\[tokio::test\]`), 281 (`assert_eq!|assert_ne!|assert!`), 0 (`proptest!|insta::assert|rstest`), 1 (`#\[ignore`); the manager/actor/metadata/provider suites are behavior-asserting and deterministic (paused clocks, causal barriers instead of sleeps, documented ignores for the reference-doc regenerator at error.rs:1009), and the three flagged tests are the exceptions

## Coverage
- idiom: 2 finding(s)
- own: 3 finding(s)
- type: clean (seeds ran: 2/0/0)
- api: clean (seeds ran: 145 pub items inspected; Arc fields are shared-ownership with cited call sites)
- err: 1 finding(s)
- serde: clean (seeds ran: 4)
- obs: clean (seeds ran: 0/2)
- docs: 3 finding(s)
- perf: 1 finding(s)
- conc: 1 finding(s)
- async: clean (seeds ran: 642/5/5/44)
- unsafe: N/A (seeds: 0/0/0/0 all zero; workspace `unsafe_code = "forbid"` inherited)
- ffi: N/A (seeds: 0 all zero; no foreign boundary)
- macro: N/A (seeds: 0 all zero; no macros)
- test: 3 finding(s)