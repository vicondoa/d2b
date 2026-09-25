# d2b-resource-runtime-p2 - d2b-resource-runtime - part 2/2
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 7756 (excl. src/generated/**) | modules: context, target, guest_target, spec_store, watch, driver, identity
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 2/2: src/context.rs, src/target.rs, src/guest_target.rs, src/spec_store.rs, src/watch.rs, src/driver.rs, src/identity.rs

## idiom
- d2b-resource-runtime-p2#1 sev=low blast=leaf effort=S verdict=actionable - two hand-written comparator closures where the sort_by_key form is the idiomatic one - fix: replace sort_by(|l, r| identity_order(l).cmp(&identity_order(r))) with sort_by_key(identity_order) in TargetDirectory::assignments_for and GuestTargetRuntime::instances - [packages/d2b-resource-runtime/src/target.rs:732, packages/d2b-resource-runtime/src/guest_target.rs:514]
  evidence: seed `for \w+ in 0\.\.` = 7 hits (6 in test modules); static read of both sort sites.
- d2b-resource-runtime-p2#2 sev=low blast=leaf effort=S verdict=actionable - hand-written impl Default for TargetDirectory where a derive yields the identical value - fix: replace the impl with #[derive(Default)] on TargetDirectory (DirectoryState already derives Default and Arc<Mutex<T>>: Default) - [packages/d2b-resource-runtime/src/target.rs:581-584]
  evidence: seed `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 2 hits (target.rs:581, watch.rs:147); the watch.rs:147 Default is not derivable (constant defaults), target.rs:581 is.
- d2b-resource-runtime-p2#3 sev=low blast=leaf effort=S verdict=actionable - inherent ResourceProvenance::from_str shadows the FromStr trait name - fix: implement std::str::FromStr for ResourceProvenance and parse at the single use site in row_from - [packages/d2b-resource-runtime/src/spec_store.rs:83-88, packages/d2b-resource-runtime/src/spec_store.rs:556]
  evidence: seed `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 2 hits; static read of spec_store.rs:83-88.

## own
- d2b-resource-runtime-p2#4 sev=low blast=leaf effort=S verdict=actionable - insert_new clones the entire row (spec and metadata Vecs included) only to stamp generation/deleting/created_at - fix: take row: StoredDesiredResource by value in insert_new and destructure it, dropping `..row.clone()`; the caller at spec_store.rs:346 does not use row afterwards - [packages/d2b-resource-runtime/src/spec_store.rs:520-541]
  evidence: seed `\.clone\(\)` = 86 hits in lane; this site copies both envelope byte vectors on every ensure-create.
- d2b-resource-runtime-p2#5 sev=low blast=leaf effort=S verdict=actionable - SpecStore::list clones the selector's zone/type_name/owner_uid fields to bind SQL params - fix: bind borrowed forms (selector.zone.as_deref(), selector.type_name.as_deref(), selector.owner_uid.as_deref()), which rusqlite params accept - [packages/d2b-resource-runtime/src/spec_store.rs:596-598]
  evidence: seed `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 36 hits in lane; the three clones at spec_store.rs:596-598 are avoidable.
- d2b-resource-runtime-p2#6 sev=low blast=leaf effort=S verdict=actionable - TargetDirectory::assign clones the assignment twice (once into the map, once for the return value) - fix: insert the owned assignment and clone from the map for the return, halving the copies of the 3-string ResourceKey and the resolved handle - [packages/d2b-resource-runtime/src/target.rs:655, packages/d2b-resource-runtime/src/target.rs:663]
  evidence: seed `\.clone\(\)` = 86 hits in lane; assign() runs on the daemon spawn path (per resource spawn).
- d2b-resource-runtime-p2#7 sev=low blast=leaf effort=S verdict=actionable - json_object clones every member Value into the map although every caller constructs the member array inline - fix: take members: impl IntoIterator<Item = (&'static str, Value)> and move values into the map - [packages/d2b-resource-runtime/src/guest_target.rs:1004-1009]
  evidence: seed `\.clone\(\)` = 86 hits in lane; this clone runs on every target-control frame encode (guest_target.rs:820-863).

## type
- clean: seeds `fn validate_\w+|fn check_\w+`, `is_\w+: bool|\w+_flag: bool`, `(mode|kind|state): String` = 0/0/0 hits; structs and enums audited by read; WatchCondition::Custom is a documented not-implemented extension point (the recorded false-positive class), and the wire rows (StoredDesiredResource, ResourceKey) are schema-mirroring shapes.

## api
- d2b-resource-runtime-p2#8 sev=medium blast=family effort=M verdict=actionable - ResourceContext::new accepts `_target: TargetHandle` and discards it; every caller supplies a value that is silently dropped - fix: remove the parameter and update the 21 call sites (provider family, resource.rs, metadata.rs, context.rs test_support), or store it and expose ResourceContext::target() per U4's "coarse handle a driver context exposes" - [packages/d2b-resource-runtime/src/context.rs:364-366]
  evidence: census: `ResourceContext::new` over packages/ = 22 hits (definition plus 21 call sites, all passing TargetHandle::Host except resource.rs:558).
- d2b-resource-runtime-p2#9 sev=medium blast=leaf effort=S verdict=actionable - TargetBinding::directory() returns &Arc<TargetDirectory> (internals leak in a public signature) and has zero callers - fix: remove the accessor, or return &TargetDirectory if a caller appears - [packages/d2b-resource-runtime/src/target.rs:491-493]
  evidence: census: `\.directory\(\)` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 2 hits, both d2b-audit's own writer.directory() (a different type); 0 hits for TargetBinding.
- d2b-resource-runtime-p2#10 sev=low blast=leaf effort=S verdict=actionable - GuestTargetRuntime::reference() returns TargetRef by value (clones the name String) and has zero callers - fix: remove the accessor, or return &TargetRef - [packages/d2b-resource-runtime/src/guest_target.rs:460-462]
  evidence: census: `\.reference\(\)` over the workspace = hits only on GuestTargetHandle::reference (target.rs) and d2bd row.reference() (foundation_seed.rs); 0 hits for GuestTargetRuntime::reference.

## err
- d2b-resource-runtime-p2#11 sev=medium blast=leaf effort=S verdict=actionable - row_from silently substitutes [0; 16] when a stored uid or owner_uid column is not exactly 16 bytes, giving a corrupt row a zero identity that collides with every other zero-uid row - fix: return a typed error (a new SpecStoreError::CorruptRow { zone, type_name, name } variant) instead of try_into().unwrap_or([0; 16]) - [packages/d2b-resource-runtime/src/spec_store.rs:553, packages/d2b-resource-runtime/src/spec_store.rs:555]
  evidence: seed `\.unwrap\(\)|\.expect\(` = 96 hits in lane, 8 non-test, all named invariants (OnceCell cache, in-transaction row presence, literally-built JSON, u64 sequence overflow); the row_from substitution is not seed-caught (unwrap_or) and was found by static read.
- clean: seed `let _ = |\.ok\(\);` = 23 hits, all deliberate best-effort cleanup or test stubs (reply.send to a dropped caller, ROLLBACK on error, permission tightening, join); seed `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 21 hits, all in test modules; seed `enum \w*Error` = 3 (TargetError, SpecStoreError, GuestTargetError), each a closed internal taxonomy with a stable Display code.

## serde
- clean: seeds `derive\([^)]*(De)?[Ss]erialize` = 0, `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` = 0, `impl .*Deserialize.*for` = 0, `serde_json::from_|serde_json::to_` = 8 hits, all the target-control frame encode/decode (guest_target.rs:823, 829, 863, 869); the hand-rolled Value walking is a live protocol admission gate mapping every failure to ProtocolMismatch, the recorded refusal class for hand-written admission gates, and the frame shape is pinned by the guest/host contract; the two expects on literally-built JSON are the recorded false-positive class.

## obs
- clean: seeds `\bprintln!\(|\beprintln!\(` = 0, `(info|debug|warn|error|trace)!\("` = 0, `\.instrument\(|#\[instrument` = 0, `tracing::|log::` = 0 in this partition; the crate's tracing use lives in the part 1 files (manager.rs, resource.rs).

## docs
- d2b-resource-runtime-p2#12 sev=low blast=leaf effort=S verdict=actionable - TargetControlAssignment's five methods (new, source, source_uid, assignment_generation, session_generation) are the only wire-carried type accessors without doc comments while sibling wire types (TargetResourceInstance, GuestRealizeRequest, TargetControlFrame) document every method - fix: add one-line docs to each - [packages/d2b-resource-runtime/src/guest_target.rs:205-228]
  evidence: docs seeds `^\s*pub (fn|struct|enum|trait|const|type)` = 148 hits, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 45; static read of guest_target.rs:204-229.
- d2b-resource-runtime-p2#13 sev=low blast=leaf effort=S verdict=actionable - constructor and accessor doc gaps on public items - fix: add one-line docs to ResourceTypeName::new/as_str, ResourceKey::new, ResourceProvenance::as_str, GuestTargetRuntime::new, TargetBinding::new, GuestTargetHandle::is_bound, SpecStore::path - [packages/d2b-resource-runtime/src/identity.rs:20, packages/d2b-resource-runtime/src/identity.rs:24, packages/d2b-resource-runtime/src/spec_store.rs:61, packages/d2b-resource-runtime/src/spec_store.rs:75, packages/d2b-resource-runtime/src/spec_store.rs:761, packages/d2b-resource-runtime/src/guest_target.rs:449, packages/d2b-resource-runtime/src/target.rs:486, packages/d2b-resource-runtime/src/target.rs:196]
  evidence: docs seeds `^\s*pub (fn|struct|enum|trait|const|type)` = 148 hits; static read of each cited site confirms no doc comment; the crate otherwise documents its public surface thoroughly (no missing_docs lint is enabled anywhere).

## perf
- d2b-resource-runtime-p2#14 sev=low blast=leaf effort=S verdict=actionable - hex rendering allocates a fresh String per byte via format! inside the loop at two sites - fix: write!(&mut rendered, "{byte:02x}") with use std::fmt::Write into the pre-sized String - [packages/d2b-resource-runtime/src/guest_target.rs:178, packages/d2b-resource-runtime/src/guest_target.rs:1212]
  evidence: seed `format!\(` = 15 hits in lane; the two loop sites are the only per-iteration allocations (target_local_spec_digest and hex_encode, both on the realize/frame wire path); static (unmeasured).

## conc
- d2b-resource-runtime-p2#15 sev=low blast=leaf effort=S verdict=actionable - parking_lot (banned outright by clippy.toml:40-43, plan KD3, except the R4 worker boundary) is a [dependencies] entry consumed only by #[cfg(test)] code - fix: move parking_lot to [dev-dependencies] or replace the two test uses with std::sync::Mutex - [packages/d2b-resource-runtime/Cargo.toml:33, packages/d2b-resource-runtime/src/context.rs:800, packages/d2b-resource-runtime/src/context.rs:1044]
  evidence: census: `parking_lot` over packages/d2b-resource-runtime = 3 hits (manifest plus 2 cfg(test) sites); supply-tagged manifest posture (the workspace-level supply lens is lane X1).
- clean: seeds `std::thread::|thread::spawn|thread::scope` = 1 (the spec-store writer thread, the sanctioned R4 bounded worker with its documented allow), `\bMutex<|\bRwLock<` = 8, `Atomic\w+|Ordering::` = 33, `thread_local!|unsafe impl (Send|Sync) for` = 0; the atomics are Relaxed counters or the documented AcqRel/acquire generation fence in bind_session, and the tokio Mutex + non-blocking try_lock sync surfaces are the documented plan U4 shape.

## async
- clean: seeds `async fn|async move|\.await` = 150+ hits (tool-truncated), `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 6+ (tool-truncated), `tokio::sync::(Mutex|RwLock|Notify)` = 8, `#\[tokio::(main|test)\]|Runtime::block_on` = 21 (all #[tokio::test]); SpecStore::call is try_send refuse-don't-queue with a documented unbounded reply await (loader_worker doctrine), WatchStream::recv is cancellation-safe (the missed AtomicBool is checked before the biased notify select and re-checked at loop top), no guard is held across an await anywhere (locks are scoped per iteration in adopt/session_authority), and every sync surface uses the documented plan U4 non-blocking try_lock.

## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0, `unsafe_code` = 0; the crate inherits workspace `unsafe_code = "forbid"` via `[lints] workspace = true` (Cargo.toml:170).

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0; the crate crosses no foreign boundary.

## macro
- N/A: seeds `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0.

## test
- clean: seeds `#\[test\]|#\[tokio::test\]` = 30+ hits (tool-truncated), `assert_eq!\(|assert_ne!\(|assert!\(` = 60+ hits (tool-truncated), `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0; the sampled tests assert real contracts with failure messages (watch gap-free LIST->WATCH replay and Missed frontier, spec-store commit-before-return durability and crash survival, driver classification at the erased boundary, target-control generation fencing) and none restates the implementation; no test that cannot fail was found.

## Coverage
- idiom: 3 finding(s)
- own: 4 finding(s)
- type: clean (seeds ran: 0/0/0)
- api: 3 finding(s)
- err: 1 finding(s)
- serde: clean (seeds ran: 0/0/0/8)
- obs: clean (seeds ran: 0/0/0/0)
- docs: 2 finding(s)
- perf: 1 finding(s)
- conc: 1 finding(s)
- async: clean (seeds ran: 150+/6+/8/21)
- unsafe: N/A (seeds: 0/0/0/0 all zero; workspace unsafe_code = "forbid")
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 30+/60+/0/0)