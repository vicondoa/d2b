# d2b-provider-volume-virtiofs - d2b-provider-volume-virtiofs
Baseline: 6ebdd4cec | LOC audited: 2,025 (excl. src/generated/**) | modules: whole crate (bindings, controller, error, lib, port, socket_path, testing, worker; tests/lifecycle.rs)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none (single-part lane) 
## idiom
- clean: seeds all zero (for-loop index 0/0, hand-written derive-able impls 0/0, let-mut accumulation 0/0); hand-written Debug on SocketIdentity/StoredBinding redact deliberately and hand-written Serialize on SocketIdentity is the hex wire rendering; none are findings. 
## own
- clean: seeds ran: 13/0/0/0; all 13 .clone() sites are explainable value copies (spec.clone() before mutating the envelope, uid.clone() into an owned fence, BoundedToken clone() into owned status reports, test-double snapshot clones after try_lock, plan/projection clones pushed into recorded history); no to_owned/Rc/RefCell/Arc<Mutex>/Arc<RwLock>/Cow. 
## type
- clean: seeds ran: 0/0/1; the one hit (worker.rs:100 sandbox_mode: String) judges clean: WorkerSandbox::declared() is a one-shot admission gate over an adapter-reported posture,and assert_conformant() validates it once against the ADR 0021 frozen singleton before launch; an enum would make the misbehaving report unrepresentable instead of rejected, which is exactly the fail-closed check the controller must keep. 
## api
- d2b-provider-volume-virtiofs#1 sev=low blast=leaf effort=S verdict=actionable - dead pub visibility on crate-internal items: resolve_view (controller.rs:23), SANDBOX_MODE (worker.rs:18), USER_NAMESPACE_MAPPING_CLASS (worker.rs:23), and WorkerSandbox plus its 3 pub fns (worker.rs:97,106,121,126) are declared pub in private modules,and never re-exported at lib.rs, so the pub is unreachable surface - fix: reduce to pub(crate)/private on those items, keeping the lib.rs re-export list as the single surface- - [packages/d2b-provider-volume-virtiofs/src/controller.rs:23, packages/d2b-provider-volume-virtiofs/src/worker.rs:97] 
  evidence: census: (resolve_view|WorkerSandbox|SANDBOX_MODE|USER_NAMESPACE_MAPPING_CLASS) over (packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel+*.bzl) =0 external hits (only in-crate uses; d2b-provider-volume-local resolve_view in views.rs:18 is a distinct symbol).
- d2b-provider-volume-virtiofs#2 sev=low blast=leaf effort=S verdict=actionable - pub mod testing exports 323 LOC of test doubles (ScriptedPort, PortCall, block_on, fixtures) unconditionally in the production lib with no feature gate, so tokio (sync) stays a runtime dependency purely for test support - fix: gate pub mod testing behind a test-support feature (with dep:tokio resolved for the feature), so the lib ships no test doubles and tokio goes conditional; keep testing.rs itself (lifecycle test uses the fixtures).- - [packages/d2b-provider-volume-virtiofs/src/lib.rs:42, packages/d2b-provider-volume-virtiofs/Cargo.toml:25] 
  evidence: census; (volume_virtiofs::testing) over (packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel+*.bzl) =0 external hits; sole consumer isthe crate own tests/lifecycle.rs:5; Cargo.toml has no [features] section, and tokio= { workspace = true, features = ["sync"] } under [dependencies].. 
## err
- clean: seeds ran: 20/0/0/1; non-test unwrap/expect =2, both on literally-built invariants (BoundedToken::parse("volume-virtiofs").expect at controller.rs:44, StatusCode::parse(reason.code().expect at bindings.rs:237, whose grammar is asserted by the every_code test in error.rs); VirtiofsBindingError is a closed #[non_exhaustive] enum with code() plus ALL - clean shape; no swallowed Results, no panics. 
## serde
- clean: seeds ran:3/7/0/3; Serialize-only wire outputs (BindingPhase kebab-case, BindingStatusReport camelCase with skip_serializing_if plus serialize_with, VirtiofsdWorkerPlan camelCase); the hand-written envelope parse is a live admission gate over serde_json::Value (per refusal ledger class), not a Deserialize impl; tests lifecycle.rs round-trips the report into JSON,and asserts the forbidden-fragment privacy pin. 
## obs
- clean: seeds ran:0/15/0/15; all 15 events are tracing::warn!/debug! with named fields (binding, provider, reason, worker), % lazy forms, static messages; warn for handled failures,and debug for documented recurring steady states (cardinality comments); no println/eprintln, no secrets, no subscriber install. 
## docs
- d2b-provider-volume-virtiofs#3 sev=low blast=leaf effort=S verdict=actionable - public Result-returning fns lack the canonical # Errors section even though rejection conditions are described in prose; from_resource_spec, worker_principal, worker_process_ref, endpoint_ref, resolve_view, reconcile, drain, for_binding, assert_conformant - fix; add a # Errors doc section to each listing the VirtiofsBindingError variant(s) it can return- - [packages/d2b-provider-volume-virtiofs/src/bindings.rs:122, packages/d2b-provider-volume-virtiofs/src/controller.rs:66, packages/d2b-provider-volume-virtiofs/src/worker.rs:64] 
  evidence: docs seed 3 (-> Result<) =14 hits across those fns; seed 2 (canonical sections) =0 hits
- d2b-provider-volume-virtiofs#4 sev=low blast=leaf effort=S verdict=actionable - VIRTIOFS_REPAIR_INTERVAL_SECS =30 documents what but not why; no rationale for the 30-second bound, while an external consumer (d2b-provider-volume-binding/src/driver.rs:112 BINDING_RESYNC) relies on it as its resync cadence - fix; extend the doc with one sentence naming the bound (e.g., matching the family repair cadence,or the socket-readiness deadline budget)- - [packages/d2b-provider-volume-virtiofs/src/controller.rs:19-20] 
  evidence: docs seed 3 (-> Result<) =14 hits; the const doc ends at "for virtiofs workers." with no why 
## perf
- d2b-provider-volume-virtiofs#5 sev=low blast=leaf effort=S verdict=actionable - derive_child_ref builds the hex suffix with 10 per-byte format! allocations (digest[..10].iter().map(|byte| format!("{byte:02x}").collect::<String>() onthe async reconcile path (twice per binding pass; worker_process_ref plus endpoint_ref), instead of one with_capacity String plus write!- fix; replace the per-byte format! chain with a String::with_capacity(20) plus write!/push_str hex loop (mirroring SocketIdentity::to_hex)- - [packages/d2b-provider-volume-virtiofs/src/bindings.rs:294-296] 
  evidence: static (unmeasured); perf seed 1 (format!() =4 hits of which 2 are this loop, 1 is worker_principal (cold), 1 is a test fixture 
## conc
- clean: seeds ran:0/4/0/0; the 4 Mutex< hits are tokio::sync::Mutex inthe test double ScriptedPort (documented plan-U4 try_lock surface for sync consumers plus lock().await for async methods); no threads, atomsics, or manual Send/Sync inthe crate. 
## async
- clean: seeds ran:27/0/0/0; async surface is controll controller.compute_report/reconcile/drain awaiting only injected effect-port futures (no locks held across awaits in production, no spawn/JoinSet/select, no blocking work, no runtime started in lib); testing.rs busypoll block_on is a documented plain-#[test] driver; the trait -> impl Future plus Send (over async fn) deliberately keeps the Send promise. 
## unsafe
- N/A (seeds: 0/0/0/1 (seed 4 = unsafe_code = "forbid" in Cargo.toml:9; no unsafe_code="allow" manifest); seeds 1-3 all zero; card; seed 4 alone does not make lens applicable. 
## ffi
- N/A (seeds: 0/0/0/0 all zero; no extern/no_mangle/CStr/repr boundary in this crate. 
## macro
- N/A (seeds: 0/0/0/0 all zero; no macro_rules!/proc-macro/$crate in this crate. 
## test
- clean: seeds ran:27/~/0/0 (#[test] =27 (bindings 4, error 1, worker 4, lifecycle 18), assertions throughout (assert_eq!/assert!/assert_ne), proptest/insta/rstest =0, #[ignore] =0); the suite is hermetic (ScriptedPort doubles, block_on driver, no virtiofsd binary/socket/network), deterministic, behavior-focused (call ordering, phases, fence validity, delete-before-confirm, privacy fragments, ownership pins), asserts error variants not Display strings,and hangs meaningful failure messages; no test restates implementation. 
## Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: clean (seeds ran: 13/0/0/0)
- type: clean (seeds ran: 0/0/1; judged admission gate, not a finding)
- api: 2 finding(s)
- err: clean (seeds ran: 20/0/0/1; non-test unwrap/expect both literally-built invariants)
- serde: clean (seeds ran: 3/7/0/3)
- obs: clean (seeds ran: 0/15/0/15)
- docs: 2 finding(s)
- perf: 1 finding(s)
- conc: clean (seeds ran: 0/4/0/0; test-only tokio::sync::Mutex)
- async: clean (seeds ran: 27/0/0/0)
- unsafe: N/A (seeds: 0/0/0/1; seeds 1-3 all zero; unsafe_code=forbid, no allow manifest)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 27/assert-mass/0/0)
