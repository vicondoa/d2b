# d2b-provider-volume-binding - d2b-provider-volume-binding
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 2900 (src 2739 + tests 161; no src/generated/**) | modules: driver, effects_service, facets, lib, row_readers, test_support; tests/registration.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate (single part)

## idiom
- clean: seeds ran: for-index-0 | impl-manual-derive-0 | let-mut-accumulate-0 (all zero); the crate reads Rust throughout - iterator chains (row_readers projections, uid_hex byte fold, sort_by_key teardown ordering), edition-2024 let-chains (delete, parent_volume), derives on every plain value type, and BoundedToken newtypes at the wire boundary.

## own
- d2b-provider-volume-binding#2 sev=low blast=leaf effort=S verdict=actionable - parsed_binding_spec clones the whole parsed spec object to end the as_object_mut() borrow before from_value (row_readers.rs:44), a clone a scoped block removes by letting `spec` move into the conversion - fix: bound the removal borrow in a block (let o = spec.as_object_mut()?; for f in [..] { o.remove(f); }) then serde_json::from_value::<VolumeBindingSpec>(spec) without Value::Object(object.clone()) - [packages/d2b-provider-volume-binding/src/row_readers.rs:38-44]
  evidence: seed `\.clone\(\)` own=81 hits (per-file clone sites: row_readers 39/44, driver 286/288/378/382/449/657, rest in test-support and cfg(test) fixtures); of the non-test clones, only row_readers.rs:44 is avoidable - the rest are required (envelope raw copy, factory arg per create, error-detail string, sort key materialization).
- clean: seeds ran: clone (78 in src, most test-support/fixtures) | to_owned/to_vec/to_string | Arc<Mutex</Rc/RefCell/Cow; every non-test clone inspected is explainable in one sentence (listed above); argument positions take &str/&[T] (parsed_binding_spec, binding_readiness_current read &StoredResource, ResourceKey::new takes &zone), no Rc/RefCell/Cow anywhere.

## type
- clean: seeds ran: validate/check-fn-0 | bool-flag-0 | stringly-state-0 (all zero); the crate's state is already typed - BindingDriverErrorKind/FailureClass split, BindingDriverStatus enum over a converged/socket/mount struct, DerivedPlan as the in-memory handle. The one stringly-typed field (BindingDriverArgs.zone: String, re-parsed per pass) is anchored under err#1 rather than re-listed here because it is unreachable by type seeds.

## api
- clean: seeds ran: pub-surface-36 hits | pub-ownership-0 | pub-use-7 hits; the exported surface is the house single-path pattern (lib.rs pub use of driver/effects_service/facets/row_readers items, feature-gated pub mod test_support), every item deliberately contract-shaped, pub(crate) for the internals (BINDING_PROVIDER_REF, WORKER_PROVIDER_REF, BindingDriverFactory, status/error types). Arc<dyn ...> appears only where ownership is genuinely shared (decoder factory return, facet set held by the driver), and the sole dependency type in a signature (Arc<dyn SpecDecoder>) is the shared decoder-factory convention used by every driver crate.

## err
- d2b-provider-volume-binding#1 sev=medium blast=family effort=S verdict=actionable - BindingDriver re-parses the zone as a BoundedToken with a per-pass .expect (driver.rs:426-427) because BindingDriverArgs.zone: String (driver.rs:344) can represent a non-bounded zone, and the sole production caller already holds a BoundedToken (d2bd resource_plane_v3.rs:2954 inputs.zone.as_str().to_owned()), so the invariant is re-checked on every validate/reconcile/recover/delete pass where a parse-at-the-boundary would check it once - fix: store BoundedToken on BindingDriverArgs/BindingDriver (parse or construct once; ResourceKey::new(&self.zone.as_str(), ...), socket_identity(&self.zone)), deleting zone_bounded and its expect - [packages/d2b-provider-volume-binding/src/driver.rs:344, packages/d2b-provider-volume-binding/src/driver.rs:426-427, packages/d2bd/src/resource_plane_v3.rs:2954]
  evidence: seed `\.unwrap\(\)|\.expect\(` err=76 hits, of which exactly one expect is production code reachable per verb (driver.rs:427; driver.rs:937 is the literally-built projection false-positive class); seed `let _ = ` = 1 site (driver.rs:1038, the deliberately ignored retirement bool, errors still propagated via ?); panic!/unreachable! only in cfg(test) match arms; census: BindingDriverArgs over packages/ = 1 production construction site (resource_plane_v3.rs:2954) plus tests/registration.rs and the driver test module.
- clean: seeds ran: unwrap/expect | let _ =/ok() | panic/unreachable/todo/unimplemented | enum Error = 0; the taxonomy (BindingDriverErrorKind with class() + failure_kind() mapping to the FailureKinds wire catalog) is a clean enum split by caller action, details carry FailureComparison context, no swallowed errors outside the documented fail-closed reads (row_readers .ok()?, reconcile guest_mount_ready unwrap_or(false)).

## serde
- clean: seeds ran: derive-0 | serde-attr-0 | impl-Deserialize-0 | from_/to_-24 hits; the crate declares no serde derive of its own and crosses the wire only by consuming contract types (VolumeBindingSpec, VolumeBindingStatusResource) through serde_json from_slice/from_value/to_vec with explicit map_err into the typed error kinds; the two read-side projections fail closed on unparseable rows by design (documented), and parsed_binding_spec's reserved-envelope-field strip is covered by a dedicated test. No round-trip hazard: this crate holds no serde wire shape.

## obs
- clean: seeds ran: println/eprintln-0 | interpolated-no-fields-0 | instrument-0 | tracing/log-1 hit; the single telemetry site is a structured tracing::warn!(plane = ?plane, key = %key, detail = %error_detail, ..) with named fields over a diagnostic detail, and no macro interpolates a message without fields; no secret-bearing field is logged.

## docs
- d2b-provider-volume-binding#3 sev=low blast=leaf effort=S verdict=actionable - the four public Result-returning seam methods state no # Errors section (which conditions fail and how the driver classifies them): facets.rs SocketRemoveSource::remove, GuestMountSource::guest_mount_ready, driver.rs BindingDriverEffects::remove_socket/guest_mount_ready - fix: add # Errors to each naming the daemon-adapter failure conditions and the fail-closed handling - [packages/d2b-provider-volume-binding/src/facets.rs:52, packages/d2b-provider-volume-binding/src/facets.rs:65, packages/d2b-provider-volume-binding/src/driver.rs:307-310, packages/d2b-provider-volume-binding/src/driver.rs:325-330]
  evidence: docs seed `-> Result<` docs=58 hits; the four pub trait-method Result returns above are the only pub Result items whose doc comment lacks a canonical # Errors section (the crate runs #![deny(missing_docs)], which covers item presence, not section completeness); every other pub item has a one-line first sentence and the item list checks out.
- clean: seeds ran: pub-items-36 | canonical-sections-0 | -> Result< -58 hits; module docs (//!) present in all six modules, first sentences carry the load, deny(missing_docs) keeps every pub item documented, the BindingDriverError/Status internals stay pub(crate) so their detailed docs are not surface.

## perf
- clean: seeds ran: format!-6 | Vec::new-7 | to_string-11 (raw match lines over src; most in cfg(test) fixtures); production format! sites are cold (uid_hex hex-spelling in error comparisons, status-projection paths), the Vec::new()s are construction-time/mandatory metadata buffers, and the only per-pass allocations (zone re-parse in zone_bounded, worker/endpoint child-spec rebuild in worker_child_specs) are static (unmeasured) and small - no hot-loop format!/to_string, no with_capacity opportunity named by any benchmark.

## conc
- d2b-provider-volume-binding#4 sev=low blast=leaf effort=S verdict=actionable - the production [dependencies] compiles the KD3-banned parking_lot (clippy.toml:82 disallows its lock outright, revocation recorded) solely for the feature-gated/cfg(test) recording doubles, so every production consumer of this crate carries the banned crate in its lockfile; the per-site #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")] permits the lock calls but not the manifest posture - fix: swap parking_lot::Mutex -> std::sync::Mutex in FakeServingEffects/RecordingManager/RecordingRequeue (the guards are already statement-scoped, so the sanctioned allows survive unchanged) and drop the Cargo.toml dependency, or gate it behind test-support as an optional dep - [packages/d2b-provider-volume-binding/Cargo.toml:29-31, packages/d2b-provider-volume-binding/src/test_support.rs:17, packages/d2b-provider-volume-binding/src/driver.rs:1139-1142, clippy.toml:82]
  evidence: conc seed `\bMutex<` conc=24 hits, every one in test-support/cfg(test) recorders carrying async-gate-allow markers; census: zero locks, threads, or atomics in the non-test src files (driver.rs/effects_service.rs/facets.rs/row_readers.rs/lib.rs) - all synchronization is the recording doubles. parking_lot appears nowhere else in production scope (grep over packages/ for parking_lot in this crate lists only the manifest + test-support sites); the atomics use SeqCst on single-threaded scripted bools, which is harmless but not a finding class here.
- clean: seeds ran: thread-0 | Mutex-10 | Atomic/Ordering-14 | thread_local/unsafe-SendSync-0; the concurrency model is the simplest correct one for a stateless driver - no shared production state beyond the purchased Arc<dyn BindingDriverEffects> seam, the test doubles' locks are never held across an await (async-gate-allow recorded), and no ordering argument needs defending in two sentences.

## async
- clean: seeds ran: async-141 hits | spawn/JoinSet/select-0 | tokio-sync-0 | tokio-main/test-13; no blocking work inside async contexts (all waits await trait seams; child-spec JSON builds are small and synchronous but not blocking I/O), no guard held across an await (the recorder locks are statement-scoped with async-gate-allow markers, and clippy deny await_holding_lock is on), the delete/finalize paths are documented idempotent under retry so cancellation mid-teardown converges, and the driver's only shared state (watched Vec, effects Arc) is never contended across tasks.

## unsafe
- N/A: seeds ran: unsafe-block-0 | SAFETY-comment-0 | transmute/from_raw/MaybeUninit-0; the manifest sets unsafe_code = "forbid" and no block, fn, impl, or extern exists, so the lens never applies.

## ffi
- N/A: seeds ran: extern/no_mangle/link_section-0 | catch_unwind-0 | repr(C)/transparent-0 | CStr/CString/c_char-0 all zero; the crate crosses no foreign boundary.

## macro
- N/A: seeds ran: macro_rules-0 | proc_macro/syn/quote-0 | $crate-0 | to_compile_error/new_spanned-0 all zero; the crate defines no macros.

## test
- clean: seeds ran: test-attr-13 | assert-120 | proptest/insta/rstest-0 | ignore-0; the suite is regression-targeted - F1 persist-before-spawn ordering, endpoint-first/process-last teardown, the fenced-projection currency rules (stale/foreign/ahead revisions), owner guard, absent-parent retry vs owner-mismatch terminal, argv-free worker child - asserted on error variants and FailureClass, never on Display strings; deterministic (scripted manager, no clock/network), and registration.rs is the policy-required declaration boundary shim. No #[ignore], no snapshot/property tooling, and no test that cannot fail was found.

## Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: 1 finding(s)
- type: clean (seeds ran: 0/0/0); the one stringly-typed field (driver.rs:344) is anchored under err#1
- api: clean (seeds ran: 36/0/7 hits; single-path pub use surface, deliberate contract exports)
- err: 1 finding(s)
- serde: clean (seeds ran: 0/0/0/24 hits; no crate-local serde derives, contract types consumed at the boundary)
- obs: clean (seeds ran: 0/0/0/1 hits; one structured tracing::warn!, zero println)
- docs: 1 finding(s)
- perf: clean (seeds ran: 24 hits; format!/to_string sites test- or error/cold-path, static (unmeasured))
- conc: 1 finding(s)
- async: clean (seeds ran: 141/0/0/13 hits; no blocking in async, no await-held guard, cancellation-safe teardown)
- unsafe: N/A (seeds: 0/0/0 all zero; unsafe_code = "forbid" manifest, no blocks/fns/impls)
- ffi: N/A (seeds: 0/0/0/0 all zero; no foreign boundary)
- macro: N/A (seeds: 0/0/0/0 all zero; no macro definitions)
- test: clean (seeds ran: 134 hits; behavior/ordering/variant assertions, no #[ignore]/proptest/insta/rstest)
