# d2b-core-controller-p1 - d2b-core-controller - part 1/2
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 6805 (excl. src/generated/**) | modules: controller_assignment.rs, binding_children.rs, coordinator.rs, main.rs, controllers.rs, lib.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 1/2 per U1 (f): src/controller_assignment.rs, src/binding_children.rs, src/coordinator.rs, src/main.rs, src/controllers.rs, src/lib.rs (part 2: authority.rs, owner_reconcile.rs, authority_persistence.rs, migration.rs)

## idiom
- clean: seeds `for \w+ in 0\.\.` = 0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 1, `let mut \w+ = (String|Vec)::new\(\)` = 0; the single hand-written Default (controllers.rs:266) preserves the all-eleven-kinds registry invariant a field-wise derive would break, and every hand-written Debug impl is a partial redaction (the A6 class from docs/explanation/over-engineering-audit-record.md:459) that the all-redact `redacted_debug!` macro cannot express; no index loops, no statement-style accumulation.

## own
- clean: seeds `\.clone\(\)` = 163, `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 61, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 0, `Cow<` = 0; every clone inspected is an owned value that must outlive the borrow (map keys and grant/lease/snapshot construction, e.g. controller_assignment.rs:548, 1028-1029, 1676-1691, 2177-2181, coordinator.rs:219, 247, 367), and every to_owned/to_string is wire rendering or string-literal ownership at a boundary; no clone silences a borrow-checker conflict.

## type
- clean: seeds `fn validate_\w+|fn check_\w+` = 5, `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0; the five validate_* functions (binding_children.rs:435, controller_assignment.rs:1793, 2356, 3054, 3087) are cross-field contract checks on already-parsed types (ResourceRef, ResourceUid, ZoneRevision), not parse-once candidates, and the coordinator's lock bools are guarded begin/finish lease tokens with no illegal combination constructible; VM identity stays a String key (coordinator.rs:174) consistent with the repo-wide string VM naming, so no newtype is warranted under the stopping rule.

## api
- d2b-core-controller-p1#1 sev=medium blast=leaf effort=S verdict=actionable - `observed_child_from_resource` (binding_children.rs:173) is a pub fn re-exported at lib.rs:35 with zero callers anywhere in the repo; the observed-child adapter is dead public surface - fix: delete the fn and its lib.rs re-export arm, or wire it into the owner_reconcile relist path if the digest-from-stored-body adapter is still intended - [packages/d2b-core-controller/src/binding_children.rs:173, packages/d2b-core-controller/src/lib.rs:35]
  evidence: census: `observed_child_from_resource` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel/*.bzl = 0 hits outside the definition and its re-export
- d2b-core-controller-p1#2 sev=medium blast=leaf effort=S verdict=actionable - eight pub methods on ControllerAssignmentRegistry/ResourceClientLease have zero callers: reserve_epoch_after, rebind_revision, record_child, remove_child, child_uids, validate_writer, observation_is_stale, target_for; because record_child is never called the children set is always empty, so release()'s ChildrenRemain fence, ChildLimit, and MAX_ASSIGNED_CHILDREN are unreachable - fix: delete the eight methods and, if the child index stays in owner_reconcile's OwnerIndex (which already tracks children), the `children` field and MAX_ASSIGNED_CHILDREN const - [packages/d2b-core-controller/src/controller_assignment.rs:2707, 2866, 2943, 2957, 2970, 3054, 3120, 2628]
  evidence: census: `.reserve_epoch_after(`, `.rebind_revision(`, `.record_child(`, `.remove_child(`, `.child_uids(`, `.validate_writer(`, `.observation_is_stale(`, `.target_for(` over packages/ including d2bd, d2bd-runtime, d2b-bus, d2b-resource-client, d2b-provider-guest-cloud-hypervisor = 0 hits each
- d2b-core-controller-p1#3 sev=medium blast=leaf effort=S verdict=actionable - ten pub ZoneCoordinator methods have zero callers: begin_usbip_reconcile, finish_usbip_reconcile, set_force_shutdown_generation, clear_force_shutdown_generation, stage_configuration, commit_configuration, abort_configuration, commit_configuration_ordinal, abort_configuration_ordinal, zone_count; the daemon uses only the ordinal staging half (d2bd/src/composition.rs:20248), the generation-based family is unwired, and stage_configuration silently overwrites a pending generation where stage_configuration_ordinal rejects a conflict - fix: delete the ten methods and the generation-based staging fields, or wire the usbip reconcile lease into the daemon reconcile flow - [packages/d2b-core-controller/src/coordinator.rs:252, 262, 292, 317, 330, 361, 374, 387, 393, 233]
  evidence: census: each method name over packages/ = 0 hits outside coordinator.rs (its own tests); live coordinator calls in d2bd/d2bd-runtime are register_zone, bind_vm, zone_for_vm, snapshot, begin_activation, finish_activation, note_force_shutdown_request, stage_configuration_ordinal
- d2b-core-controller-p1#4 sev=low blast=leaf effort=S verdict=actionable - three types re-exported from lib.rs have no external consumers: CoreHandlerRegistry, CurrencyAggregation, BindingChildResource; only their sibling types (CoreHandlerKind, materialize_child_create_payload) are used outside the crate - fix: drop the three names from the lib.rs pub use arms and mark the types pub(crate) - [packages/d2b-core-controller/src/lib.rs:33-51, packages/d2b-core-controller/src/controllers.rs:131, 262, packages/d2b-core-controller/src/binding_children.rs:23]
  evidence: census: `CoreHandlerRegistry`, `CurrencyAggregation`, `BindingChildResource` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 0 files outside the crate's src; CoreHandlerKind alone is used by d2bd-runtime/src/resource_runtime_support.rs

## err
- clean: seeds `\.unwrap\(\)|\.expect\(` = 285 (283 inside #[cfg(test)]), `let _ = |\.ok\(\);` = 0, `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 0, `enum \w*Error` = 7; the two non-test sites (controller_assignment.rs:632 expect on a json! literal, binding_children.rs:50 expect on a closed-enum resource type) name their invariants; the seven error enums are split by caller action with stable identity-free Display codes.

## serde
- d2b-core-controller-p1#5 sev=medium blast=wide effort=L verdict=actionable - the assignment evidence codec is hand-rolled JSON: encode_assignment/decode_assignment/require_exact_keys/encode_bounded_json/decode_string_set walk serde_json::Value with per-field get() chains and exact-key lists where derive(Deserialize) with deny_unknown_fields plus the existing canonical-ordering checks would give the same admission; refusal-ledger row C3 names this codec and it is still present - fix: replace the Value-walking encode/decode with typed serde structs (rename_all = "camelCase", deny_unknown_fields) preserving the wire shape pinned by the transport tests (exact keys, version 1, sorted verb arrays, canonical bytes, bounded size), and replace the json!-literal payload builder in materialize_child_create_payload with a typed envelope builder - [packages/d2b-core-controller/src/controller_assignment.rs:596, 735, 891, 901, 2013, packages/d2b-core-controller/src/binding_children.rs:367-430]
  evidence: seed `serde_json::from_|serde_json::to_` = 26 (serde derive seeds 0 in this part); row C3 at docs/explanation/over-engineering-audit-record.md:474 is "not applied" with no refusal reason, and the site is confirmed still present at baseline

## obs
- N/A: seeds `\bprintln!\(|\beprintln!\(` = 0, `(info|debug|warn|error|trace)!\("` = 0, `\.instrument\(|#\[instrument` = 0, `tracing::|log::` = 0; the manifest declares no tracing/log dependency (Cargo.toml deps: three contracts crates, d2b-controller-toolkit, serde, serde_json, sha2, tokio)

## docs
- d2b-core-controller-p1#6 sev=low blast=leaf effort=M verdict=actionable - no Result-returning pub item carries an `# Errors` section (0 of 83 `-> Result<` items); failure conditions live only in the error-enum variant docs, so a caller must read the enum to learn which errors a fence method returns - fix: add `# Errors` sections naming the returned variants to the pub Result-returning methods, starting with the fence methods (validate_for, validate_writer, query, admit, publish_readiness, bind_vm) - [packages/d2b-core-controller/src/controller_assignment.rs:1793, 3054, 2501, 2720, packages/d2b-core-controller/src/main.rs:188, packages/d2b-core-controller/src/coordinator.rs:206]
  evidence: seeds `^\s*pub (fn|struct|enum|trait|const|type)` = 242, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 83
- d2b-core-controller-p1#7 sev=low blast=leaf effort=S verdict=actionable - pub struct fields without doc comments: RuntimeReadiness (3 of 5 fields), RecoverySnapshot (5 of 7), HandlerStatus (all 8); the undocumented fields (checkpoint_revision, last_reconciled_tick, retry_after_tick, provider_lease_count) are non-obvious - fix: add one-line field docs to RuntimeReadiness, RecoverySnapshot, and HandlerStatus - [packages/d2b-core-controller/src/main.rs:33-56, packages/d2b-core-controller/src/controllers.rs:198-209]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 242 with the three structs' field blocks read in full

## perf
- clean: seeds `format!\(` = 2 (both inside #[cfg(test)]), `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 20 (cold admission, snapshot, and digest paths), `\.to_string\(\)` = 1 (wire rendering); no allocation site sits in a loop over unbounded input (mutation arrays capped at 128, verb sets at 64, status collections at MAX_STATUS_COLLECTION_ENTRIES); static (unmeasured)

## conc
- clean: seeds `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 0, `Atomic\w+|Ordering::` = 13, `thread_local!|unsafe impl (Send|Sync) for` = 0; the atomics (AssignmentLeaseState phase/stale_observation at controller_assignment.rs:2661-2685, authority_epoch at main.rs:99) use correct Acquire/Release store/load pairs and AcqRel on an epoch counter; no lock, thread, or manual Send/Sync claim in the part.

## async
- N/A: seeds `async fn|async move|\.await|tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(|tokio::sync::(Mutex|RwLock|Notify)|#\[tokio::(main|test)\]|Runtime::block_on` = 0 over the part's six files; all tokio usage in this crate lives in part 2 (authority.rs, authority_persistence.rs)

## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern|// SAFETY:|transmute|from_raw|MaybeUninit|mem::zeroed|unsafe_code` = 0; no unsafe blocks, SAFETY comments, or lint overrides in the part

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section|catch_unwind|repr\(C\)|repr\(transparent\)|CStr|CString|c_char` = 0

## macro
- N/A: seeds `macro_rules!|proc_macro|syn::|quote!|\$crate|to_compile_error|new_spanned` = 0; redacted_debug! is invoked (controller_assignment.rs:68), not defined, in this part

## test
- clean: seeds `#\[test\]|#\[tokio::test\]` = 50 in src plus 3 in tests/owned_children.rs, `assert_eq!\(|assert_ne!\(|assert!\(` = 183 plus 21, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0; tests assert behavior (phase transitions, fence rejections, canonical round-trips, zone isolation, transport tamper rejection) with human-written expectations, no network or clock dependence, and the integration test consumes the public owner-reconcile API as a real caller.

## Coverage
- idiom: clean (seeds: 0/1/0)
- own: clean (seeds: 163/61/0/0)
- type: clean (seeds: 5/0/0)
- api: 4 finding(s)
- err: clean (seeds: 285/0/0/7)
- serde: 1 finding(s)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency in manifest)
- docs: 2 finding(s)
- perf: clean (seeds: 2/20/1)
- conc: clean (seeds: 0/0/13/0)
- async: N/A (seeds: 0 all zero; all tokio usage in part 2)
- unsafe: N/A (seeds: 0 all zero)
- ffi: N/A (seeds: 0 all zero)
- macro: N/A (seeds: 0 all zero)
- test: clean (seeds: 53/204/0/0)