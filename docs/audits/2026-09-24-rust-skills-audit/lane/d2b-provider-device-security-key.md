# d2b-provider-device-security-key - d2b-provider-device-security-key
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 4232 (excl. src/generated/**; src 3853 + tests 379) | modules: whole crate
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none (single-part lane)

## idiom
- d2b-provider-device-security-key#1 sev=low blast=leaf effort=S verdict=actionable - `declared_dependency_refs` accumulates through a `let mut refs = Vec::new()` plus a push closure instead of an iterator chain, the one statement-style accumulation in the crate - fix: collect the two `Option<ResourceRef>` probes with an iterator chain (`[a, b].into_iter().flatten().collect()`) and delete the closure - [packages/d2b-provider-device-security-key/src/driver.rs:380-390]
  evidence: seed 3 `let mut \w+ = (String|Vec)::new\(\)` = 1 hit (driver.rs:380); seeds 1-2 = 0
- clean: seeds ran: 0/0/1 - no index loops, no hand-written derive-replaceable impls (all Debug impls redact secrets, deliberate per repo false-positive list), one accumulation site flagged above

## own
- clean: seeds ran: 37/30/0/0 - every clone inspected: owned-arg clones into the contracts `explicit_binding_children` helper (controller.rs:224-266), `metadata.clone()` reuse for the second ChildEnsure (driver.rs:494), thread-boundary `vm_id`/Arc clones (relay_service.rs:402,440-442), lease holder/admission clones that must outlive the borrow (lease.rs:121-122,171,194,276-277), test-fixture clones; no Rc/RefCell/Arc<Mutex>/Cow anywhere
- clean: `test_support.rs:32` `self.calls.lock().clone()` returns a snapshot from a Mutex guard - the standard pattern for a recording double, not a borrow-checker workaround

## type
- clean: seeds ran: 0/1/0 - the single hit `watched_configuration_is_dependency: bool` (controller.rs:83) is one runner-contract knob mirroring the shared-Runner cutover, not flag soup; lease state is a closed `LeaseState` enum with transition checks, and `backing`/`authorized_*` Option pairs are enforced by the state machine at every use site (`.ok_or(AuthorizationDenied)`), so typestate would not buy a real bug class

## api
- d2b-provider-device-security-key#2 sev=low blast=leaf effort=S verdict=actionable - `pub mod relay` and `pub mod relay_service` (lib.rs:16-17) expose a second path for every root-re-exported item, while `facets`/`effects_service`/`test_support` module paths are the ones the daemon actually consumes - fix: make `relay` and `relay_service` private modules, keep the lib.rs `pub use` arms as the single surface (house pattern) - [packages/d2b-provider-device-security-key/src/lib.rs:16-17, packages/d2b-provider-device-security-key/src/lib.rs:45-55]
  evidence: census `device_security_key::relay::` over packages/nixos-modules/tests/docs/reference/labs = 1 hit (a doc comment in d2b-broker/src/ops/security_key.rs:8); `device_security_key::relay_service::` = 0; `::facets::`/`::effects_service::`/`::test_support::` = 16 hits in d2bd (resource_plane_v3.rs:1875,2019,2263,3786,3948; shared_provider_effects.rs:2704,3500)
- clean: seed 2 `pub .*Arc<` = 2 hits, both genuine shared ownership with cited consumers: `SecurityKeyEffectFacets.runtime: Arc<dyn SecurityKeyRuntime>` (facets.rs:34) is constructed field-by-field by d2bd (resource_plane_v3.rs:2019-2023) and `SkAcceptHandle.state: Arc<parking_lot::Mutex<SecurityKeyState>>` (relay_service.rs:297) is shared across the accept thread and connection tasks; the relay hosting exports have no external callers but are dossier-pinned (ADR-046 D046, docs/specs/ADR-046-decision-register.md:68; prior audit row U60 "No dead surface beyond dossier-pinned relay") - not flagged
- clean: seed 3 `pub use` = 8 arms, the house single-surface pattern; the crate root `#![deny(missing_docs)]` (lib.rs:6) plus per-item docs give a deliberate, documented surface

## err
- clean: seeds ran: 70/5/6/5 - production unwrap/expect reduced to the canonical-const class (controller.rs:228,265 `ResourceRef::parse)...).expect("... is canonical")` on a literal; repo false positive); all other hits are `#[cfg(test)]`/`tests/` fixtures; `let _ =` sites are deliberate best-effort (oneshot notify relay_service.rs:282, aborted-task awaits 587/589, cancel packet 596); panics only in test doubles (exact_authority.rs:22, mutual_exclusion.rs:22); the five error enums are closed, split by caller action, and carry stable `code()` wire strings with per-variant docs
- clean: error context survives via the boundary that handles it: `SecurityKeyControllerError::Admission` (controller.rs:123) collapses binding-child detail into a stable code, but the inner error is logged with named fields at the same site (controller.rs:232-235) - deliberate closed-code design, not a swallowed failure

## serde
- clean: seeds ran: 0/0/0/9 - no derive/attribute/manual-impl surface; all JSON crossing is untyped `serde_json::Value`/`to_vec` with `map_err(|_| invalid())` mapped to `SharedProviderDeclarationError::SpecInvalid` (driver.rs:437-528, effects_service.rs:59); the `binding_child_ensure` Value round-trip (driver.rs:514-517) extracts a known shape from an in-tree trusted payload, not untrusted wire input - no typed boundary to judge

## obs
- clean: seeds ran: 0/0/0/31 - zero println/eprintln, zero interpolated-message events, zero instrument spans; all 31 tracing events carry named fields (device, error, reason, vm, selector) with the message as the final literal; no secret material in any field (device UIDs and VM ids are identity, not secrets; selector_label is a stable label); redaction is enforced structurally by hand-written Debug impls and pinned by tests/redaction.rs

## docs
- d2b-provider-device-security-key#3 sev=low blast=leaf effort=S verdict=actionable - `#![allow(missing_docs)]` at relay.rs:7 defeats the crate-root `#![deny(missing_docs)]` for a pub module re-exported at the root, letting undocumented items ship: `CidTranslator::new` (relay.rs:144), `LeaseId::as_u64` (relay.rs:209), and the pub fields of `CtaphidInitPacket`/`CtaphidContPacket` (relay.rs:54-66) - fix: document the handful of items and drop the module-level allow - [packages/d2b-provider-device-security-key/src/relay.rs:7, packages/d2b-provider-device-security-key/src/relay.rs:144, packages/d2b-provider-device-security-key/src/relay.rs:209]
  evidence: seed 1 `^\s*pub (fn|struct|enum|trait|const|type)` = 120 hits; the allow attribute at relay.rs:7 is the only missing_docs suppression in the crate
- d2b-provider-device-security-key#4 sev=medium blast=leaf effort=S verdict=actionable - no `# Errors` section exists on any of the 24 public `Result`-returning items, and the lease/controller state-machine failures are the non-obvious kind the section exists for (`SessionConflict` vs `InvalidTransition` vs `AuthorizationDenied` vs `Effect`) - fix: add `# Errors` sections naming the returned variants to `SecurityKeyLease::{acquire, acquire_authorized, rebind_authorized, complete, cancel, expire}` and `SecurityKeyController::{new, new_authorized, child_resources, child_resources_for_user, acquire, acquire_authorized, rebind_authorized, complete}` - [packages/d2b-provider-device-security-key/src/lease.rs:150-303, packages/d2b-provider-device-security-key/src/controller.rs:162-186, packages/d2b-provider-device-security-key/src/controller.rs:209-267]
  evidence: seed 2 `/// # (Examples|Errors|Panics|Safety)` = 0 hits; seed 3 `-> Result<` = 24 hits
- clean: first sentences are one-line and load-bearing, module docs (`//!`) present in every module, error enums carry per-variant docs, magic values documented with the why (CEREMONY_TIMEOUT relay.rs:37-39, ring bounds lib.rs:67-72)

## perf
- clean: seeds ran: 6/3/2 - every allocation site is cold: error construction (relay_service.rs:85,158), thread names (relay_service.rs:404), one-shot dependency collection (driver.rs:380-390), process-name derivation (process.rs:167), test helpers; `Vec::new` sites are the empty-case-common shape; no format!/to_string in any loop or hot path - static (unmeasured) per lens rules

## conc
- d2b-provider-device-security-key#5 sev=medium blast=leaf effort=M verdict=actionable - 14 `parking_lot::Mutex::lock` sites (8 production-path in relay_service.rs:129,281,489,497,527,532,592,599 and 6 in the test-support-gated test_support.rs:32,43,44,55,78,89) carry no `#[allow(clippy::disallowed_methods, reason = "...")]` attribute, while the committed blocking-census baseline records `parking_lot::Mutex::lock = 0` for this crate and parking_lot is banned outright by clippy.toml (KD3) with only the R4 worker boundary exempt - the census bookkeeping and the manifest's recorded `disallowed_methods = "deny"` level disagree with the source, and the `// async-gate-allow:` markers cover only the async-gate scanner, not the clippy/census side - fix: add the sanctioned per-site allows (`reason = "synchronous path"`) or convert the short critical sections to the clippy.toml-named `tokio::sync::Mutex` replacement, then regenerate the census baseline to match - [packages/d2b-provider-device-security-key/src/relay_service.rs:129, packages/d2b-provider-device-security-key/src/relay_service.rs:281, packages/d2b-provider-device-security-key/src/relay_service.rs:489-599, packages/d2b-provider-device-security-key/src/test_support.rs:32-89]
  evidence: seed `\.lock\(\)` = 16 hits - 14 outside any `#[allow(clippy::disallowed_methods)]` fn (relay_service.rs:129,281,489,497,527,532,592,599; test_support.rs:32,43,44,55,78,89), 2 inside cfg(test) fns with the sanctioned "cfg(test) helper" allow (relay_service.rs:1031,1087); packages/xtask/data/blocking-census-baseline.json `packages/d2b-provider-device-security-key` -> `parking_lot::Mutex::lock: 0`; the census's prod/test split (blocking_census.rs `split_contexts`/`is_test_dir`) counts non-cfg(test) lines as production, so the test_support.rs sites count too; gate state not run (read-only lane) - the row asserts the baseline-vs-source mismatch, not a gate failure; clippy.toml disallowed-methods entry `parking_lot::Mutex::lock` with KD3 reason
- clean: the concurrency model fits the workload - one dedicated accept thread owning its own current-thread runtime (relay_service.rs:403-416, sanctioned "synchronous path" allow), shared `SecurityKeyState` behind a mutex with guards never held across an await, `AtomicU64` counters with Relaxed ordering (relay.rs:205-206, relay_service.rs:620-626) are the repo-sanctioned counter shape; no `unsafe impl Send/Sync`, no `thread_local!`, no `static mut`

## async
- d2b-provider-device-security-key#6 sev=medium blast=leaf effort=S verdict=actionable - `run_connection` releases the ceremony lease only in its final statement (relay_service.rs:599), so an aborted task leaks the lease: `SkSessionTable::stop_vm`/`register`-replacement or accept-loop abort drops the accept thread's runtime and aborts every in-flight connection task mid-await, leaving `SecurityKeyState` `Leased` until `CEREMONY_TIMEOUT` (120s) expiry evicts it - other VMs are rejected (15s queue wait) for that whole window - fix: release the lease from a Drop guard (a small struct owning `Arc<parking_lot::Mutex<SecurityKeyState>>` + vm_id + lease_id, dropped on task abort) so cancellation is resumable - [packages/d2b-provider-device-security-key/src/relay_service.rs:470-600, packages/d2b-provider-device-security-key/src/relay_service.rs:380-383, packages/d2b-provider-device-security-key/src/relay_service.rs:366-371]
  evidence: seed 1 `async fn|async move|\.await` = 60+ hits; seed 2 `tokio::spawn|select!` = 8 hits; static trace: abort path (SkAcceptAbort::abort -> accept-loop break -> runtime drop -> spawned task abort) skips the release statement at relay_service.rs:599
- clean: no guard is held across an await (all `parking_lot` guards are temporaries dropped before any suspension point - verified at relay_service.rs:489,497,527,532,592,599); `runtime.block_on` at relay_service.rs:416 is the sanctioned dedicated-thread boundary with an inline allow; `tokio::spawn` closures are `Send + 'static` via Arc; the select-then-await-other shape (587-590) correctly awaits the aborted task; all lock sites carry `// async-gate-allow:` markers recorded in the inventory - cited, not re-flagged

## unsafe
- N/A: seeds 1-3 (`\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern`, `// SAFETY:`, `transmute|from_raw|MaybeUninit|mem::zeroed`) all zero; the single seed-4 hit is a doc-comment mention of the workspace `forbid(unsafe_code)` lint (relay_service.rs:34), not a site; manifest `[lints.rust] unsafe_code = "forbid"` (Cargo.toml)

## ffi
- N/A: seeds 1-4 (`extern "C"|no_mangle|unsafe\(link_section`, `catch_unwind`, `repr\(C\)|repr\(transparent\)`, `CStr|CString|c_char`) all zero - no foreign boundary in this crate; fd crossing is safe `File::from(OwnedFd)` under the workspace forbid

## macro
- N/A: seeds 1-4 (`macro_rules!`, `proc_macro|syn::|quote!`, `\$crate`, `to_compile_error|new_spanned`) all zero - no macro definitions or proc-macro surface

## test
- d2b-provider-device-security-key#7 sev=medium blast=leaf effort=M verdict=actionable - no test exercises the relay's core forwarding path: `run_connection`'s guest->hidraw and hidraw->guest loops, CID translation inside the loop, and cancel-on-close are untested while every component (framing, hidraw wrapper, lease, reject paths) has its own unit test - contract behavior with no test; `test_hidraw()` already provides a socket-pair hidraw double, so a full-loop test is feasible - fix: add a `#[tokio::test(flavor = "current_thread")]` that runs `run_connection` with a socket-pair hidraw and a connected guest stream, asserts report forwarding in both directions and a `CTAPHID_CANCEL` packet on peer close - [packages/d2b-provider-device-security-key/src/relay_service.rs:470-600, packages/d2b-provider-device-security-key/src/relay_service.rs:642-648]
  evidence: seed 1 `#\[test\]|#\[tokio::test\]` = 50 hits; seed 2 `assert_eq!\(|assert_ne!\(|assert!\(` = 90+ hits; the 33 relay_service tests cover parsing/CID/lease/framing/hidraw/auth/reject paths but none drives the forwarding loop
- clean: seeds ran: 50/90+/0/0 - no `#[ignore]`, no proptest/insta/rstest; assertions target behavior and error variants, not Display strings (lease_state_machine.rs:91-97 asserts `Err(SecurityKeyLeaseError::Effect(Transient))`); redaction is pinned by tests/redaction.rs; tests are deterministic (socket pairs, current-process peer creds, no network/clock injection); integration/provider_lifecycle.rs is a real executable scenario, not a scaffold (this crate is not on the 18-crate policy-scaffold list)

## Coverage
- idiom: 1 finding
- own: clean (seeds ran: 37/30/0/0)
- type: clean (seeds ran: 0/1/0)
- api: 1 finding
- err: clean (seeds ran: 70/5/6/5)
- serde: clean (seeds ran: 0/0/0/9)
- obs: clean (seeds ran: 0/0/0/31)
- docs: 2 findings
- perf: clean (seeds ran: 6/3/2)
- conc: 1 finding
- async: 1 finding
- unsafe: N/A (seeds: 0/0/0/1 - seeds 1-3 all zero; single seed-4 hit is a doc-comment mention; manifest forbids unsafe_code)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: 1 finding