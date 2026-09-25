# d2b-provider-device-usbip - d2b-provider-device-usbip
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 7015 (excl. src/generated/**; src 5863 + tests 1143 + integration 9) | modules: whole crate
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate (1 part)

## idiom
- d2b-provider-device-usbip#1 sev=low blast=leaf effort=S verdict=actionable - `declared_dependency_refs` accumulates into `let mut refs = Vec::new()` and pushes in match arms where each arm returns a fixed small list - fix: return the match arms as owned `Vec` literals (or `.into_iter().flatten().collect()`) so the shape is an expression - [driver.rs:267-281]
  evidence: seed 3 `let mut \w+ = (String|Vec)::new\(\)` = 1 hit (driver.rs:267); seeds 1-2 (`for \w+ in 0\.\.`, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`) = 0
- clean: all three seeds ran (0/0/1); the crate's hand-written `Debug` impls redact (busid.rs:25, process.rs:47, firewall.rs:39) and are the deliberate redaction pattern, not derive candidates

## own
- d2b-provider-device-usbip#2 sev=low blast=leaf effort=S verdict=actionable - lease admission in `KernelUsbipDispatcher` clones each 16-byte lease three times per reservation (`ledger.insert)..., lease.clone())`, `self.x = Some(lease.clone())`, `Ok(lease.clone())`) - fix: move the lease into the field and clone from the field for the map and the return (two clones), or return `self.x.as_ref().unwrap().clone()` after the insert - [broker.rs:194-208, broker.rs:218-232, broker.rs:313-321, broker.rs:333-341]
  evidence: seed 1 `\.clone\(\)` = 104 hits crate-wide (matrix); the triple-clone shape at broker.rs:197+205+207 (and the relay/slot/proxy twins); all other clones in the crate are explainable (owned struct fields, report snapshots, test doubles)
- clean: seeds ran (104/14/0/0 per matrix); `Arc<tokio::sync::Mutex<AuthorityLedger>>` sharing is genuine (one ledger per zone, handed to every dispatcher; caller d2bd/src/shared_provider_effects.rs:267); no `Rc`/`RefCell`/`Cow`

## type
- clean: seeds ran (4/1/0): `validate_zone`/`validate_provider_class`/`validate_admission`/`validate_network` are boundary admission gates on wire strings (recorded refused class, U1 (d) 6), `watched_configuration_is_dependency: bool` is a pinned cutover contract field (controller.rs:28); `BusId`/`PhysicalUsbBackingToken`/leases are already parsed/opaque newtypes; no Option-pair or stringly-state smells

## api
- d2b-provider-device-usbip#3 sev=medium blast=leaf effort=S verdict=actionable - `pub mod state_machine` (lib.rs:24) plus the root `pub use state_machine::{...}` (lib.rs:61-65) exposes every state-machine item at two public paths, and no external caller uses the module path - fix: make the module private (`mod state_machine`) since lib.rs already re-exports its whole surface - [lib.rs:24, lib.rs:61-65]
  evidence: seed 3 `^\s*pub use ` = 9 arms; census `device_usbip::state_machine` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 0 hits outside lib.rs (docs/reference/usbip-state-machine.md:110 imports from the root)
- d2b-provider-device-usbip#4 sev=medium blast=leaf effort=S verdict=actionable - `new_authority_ledger` returns `Arc<tokio::sync::Mutex<AuthorityLedger>>`, leaking the concrete lock type into the public signature and making any lock change a breaking change for the caller - fix: introduce an opaque `AuthorityLedgerHandle` newtype wrapping the `Arc<tokio::sync::Mutex<...>>` (or a `pub type` alias) so the handle is the API - [broker.rs:131-133]
  evidence: seed 2 `pub .*\b(Arc|Rc|Box|RefCell)<` = 4 hits (broker.rs:131, facets.rs:50+65, test_support.rs:101); the facets `Arc<dyn Trait>` fields are justified shared daemon-supplied facets (built at d2bd/src/resource_plane_v3.rs:2009-2015); census `new_authority_ledger` = 1 caller (d2bd/src/shared_provider_effects.rs:267); the Arc sharing itself is genuine, only the lock-type leak is the finding
- clean: 313 pub items (matrix) are the deliberate declared-provider surface; `pub use` re-export arms in lib.rs are the house single-surface pattern; the other `pub mod`s (broker, core_adapter, effects_service, facets, reconcile_state, vocabulary) each have external module-path consumers (d2bd/src/composition.rs:9557, resource_plane_v3.rs:1874, shared_provider_effects.rs:60)

## err
- d2b-provider-device-usbip#5 sev=medium blast=leaf effort=M verdict=actionable - `UsbipStepExecutor` returns `Result<(), String>` from every step method, forcing implementers and callers to string-match reasons where the crate's own taxonomy is otherwise typed enums with `code()` accessors - fix: introduce a closed per-step error enum (or reuse `UsbipPlanError` tagged with the step) and map it in `execute_usbip_plan` - [state_machine.rs:378-386]
  evidence: seed 4 `enum \w*Error` = 8 typed enums, all with stable `code()` accessors; the trait is the crate's only `Result<(), String>` surface; census `UsbipStepExecutor` = 1 impl (state_machine.rs:510, test fixture only) and docs/reference/usbip-state-machine.md:126 records "no production adapter currently implements this trait"
- clean: seed 1 `.unwrap\(\)|\.expect\(` = 12 hits, all inside `#[cfg(test)]` or the literal-constant `expect` at lifecycle.rs:56 (recorded false-positive class); seed 2 `let _ = |\.ok\(\);` = 0; seed 3 `panic!` = 2, both in tests; error enums carry no caller-controlled identity

## serde
- clean: seeds ran (8/12/1/3): 8 derives with `rename_all`/`deny_unknown_fields`/`tag` conventions, 1 hand-written `Deserialize` (`UsbipReconcileCorrelationId`, reconcile_state.rs:293) plus the `deserialize_with` VM-shape gate (reconcile_state.rs:37) - both are live admission gates in the recorded refused class (U1 (d) 6, do not re-flag); `serde_json` calls are error-mapped boundary conversions (driver.rs:305-316); optionality meanings (`default` + `skip_serializing_if` + `Option`) are used deliberately on `UsbipEventSource.vm` (reconcile_state.rs:220-224)

## obs
- clean: seeds ran (0/0/0/45): zero `println!`/`eprintln!`; zero interpolated-first-argument events - every tracing site uses named fields with a trailing message (e.g. lifecycle.rs:251-256); no secret in fields (resource refs are the crate's canonical identities, and wrong_zone_and_redaction.rs:77-98 pins identity-free Debug/metric labels); no spans needed since the crate has no async orchestration of its own

## docs
- d2b-provider-device-usbip#6 sev=medium blast=leaf effort=M verdict=actionable - `#![allow(missing_docs)]` in reconcile_state.rs:6 and state_machine.rs:61 contradicts the crate's `#![deny(missing_docs)]` (lib.rs:9), leaving root-re-exported pub items without doc contracts (`UsbipPolicyFailure::telemetry_label`, `UsbipEventSource::vm`/`component`, `UsbipReconcileCorrelationId::new`, `UsbipClaimSource::is_explicit`, `UsbipExecutionReport::is_ok`, `UsbipBusidPlan::stop_order`) - fix: document the pub items and drop the two module-level allows - [reconcile_state.rs:6, state_machine.rs:61, lib.rs:9]
  evidence: docs seed 2 `/// # (Examples|Errors|Panics|Safety)` = 0; lib.rs:9 `#![deny(missing_docs)]` vs the two module allows; the affected items are re-exported at the crate root (lib.rs:61-65)
- d2b-provider-device-usbip#7 sev=medium blast=leaf effort=M verdict=actionable - Result-returning public items have no `# Errors` section anywhere in the crate, so callers cannot learn the closed failure sets from the docs - fix: add `# Errors` sections naming the variants to `UsbipArbitrator::new`, `BusId::parse`, `UsbipBindingContext::new`, `UsbipBindingController::new`, `build_usbip_plan`, `execute_usbip_plan`, `admit_bind_bus_class`, `binding_child_resources` - [arbitration.rs:76, busid.rs:12, broker.rs:61, controller.rs:210, state_machine.rs:254, state_machine.rs:429, vocabulary.rs:70, lifecycle.rs:43]
  evidence: seed 3 `-> Result<` = 111 hits (matrix docs total 424 = 313 pub items + 111 Result returns); seed 2 canonical sections = 0 hits
- clean: first sentences are one-line contract statements throughout; magic values carry the why (USBIP_DEVICE_MAJOR vocabulary.rs:33, USBIP_REPAIR_INTERVAL_SECS controller.rs:14)

## perf
- clean: seeds ran (4/5/12): `format!` only in plan-error paths (state_machine.rs:287,298,345) and a test (741) - cold per the card; `Vec::new()` at empty-case-common or fixture sites (arbitration.rs:90, lifecycle.rs:904, state_machine.rs:489+495) plus driver.rs:267 (covered by idiom#1); `to_string`/`to_owned` at wire-rendering and owned-projection boundaries; no hot loops; all findings static (unmeasured)

## conc
- d2b-provider-device-usbip#8 sev=low blast=leaf effort=S verdict=policy-confirmed - `test_support.rs` recorders use `parking_lot::Mutex` (fields at 25/27/29/70/72, `.lock()` at 35/46-47/58-59/82/93) with no per-site allow, but parking_lot is banned outright with the single R4 bounded-worker exception - fix: replace with `tokio::sync::Mutex` (the clippy.toml-named replacement) or add the sanctioned `cfg(test) helper` per-site allow - [test_support.rs:25, test_support.rs:35, Cargo.toml:30]
  evidence: seed 2 `\bMutex<` = 9 hits (broker.rs:131-157 tokio::sync::Mutex x4, test_support.rs parking_lot::Mutex x5); zero `#[allow(clippy::disallowed_methods)]` sites in the crate; policy: clippy.toml:40 "parking_lot is banned outright (plan KD3)"; the site is not on the recorded exception list (U1 (d) 2)
- clean: `AtomicU64` + `Ordering::Relaxed` token counter (broker.rs:123) is the weakest-correct ordering for a nobody-synchronizes-on counter; the shared `tokio::sync::Mutex` ledger is used via `try_lock` in synchronous dispatcher methods, never held across an await; no threads, no `thread_local!`, no manual `Send`/`Sync`

## async
- clean: seeds ran (18/0/4/0): all `async fn`s are thin delegations to the daemon-supplied facets (`UsbipRuntime`/`UsbipBrokerDispatch`) with no spawn/select/join/block_on; the ledger mutex is never held across `.await` (try_lock in sync methods); `#[async_trait]` is the pragmatic object-safe choice; no runtime is started inside the library

## unsafe
- N/A: seeds 0/0/0 all zero (`\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern`, `// SAFETY:`, `transmute|from_raw|MaybeUninit|mem::zeroed`); manifest `unsafe_code = "forbid"` (Cargo.toml [lints.rust]) and no `unsafe_code` text in src

## ffi
- N/A: seeds 0/0/0/0 all zero (`extern "C"|no_mangle|unsafe\(link_section`, `catch_unwind`, `repr\(C\)|repr\(transparent\)`, `CStr|CString|c_char`); the crate crosses no foreign boundary

## macro
- N/A: seeds 0/0/0/0 all zero (`macro_rules!`, `proc_macro|syn::|quote!`, `\$crate`, `to_compile_error|new_spanned`); no macros defined or used beyond std

## test
- d2b-provider-device-usbip#9 sev=low blast=leaf effort=S verdict=actionable - conformance.rs:63-66 asserts `AttachmentCommand::Attach(AttachmentActivation::Declared)` equals an identical literal, an assertion that cannot fail - fix: delete the tautological assert_eq (the same test's other asserts already pin the enum shape) - [tests/conformance.rs:63-66]
  evidence: seed 2 `assert_eq!\(|assert_ne!\(|assert!\(` = 229 hits (matrix); the two compared expressions are the same literal construction
- d2b-provider-device-usbip#10 sev=medium blast=leaf effort=S verdict=actionable - `UsbipArbitrator` branches are untested: only the exclusive second-claim conflict is covered, while the constructor ceiling/ArbitrationViolation validation, `MaxClaimsExceeded`, idempotent re-claim by the same holder, and `release` have no test - fix: add a table-driven unit test over the ceiling, arbitration mode, re-claim, and release paths - [tests/arbitration_conflict.rs:8-19, src/arbitration.rs:81-84, src/arbitration.rs:113-115, src/arbitration.rs:117-121]
  evidence: seed 1 `#\[test\]|#\[tokio::test\]` = 40 hits across src+tests; arbitration_conflict.rs:8 is the only arbiter test and exercises one of the four claim branches
- d2b-provider-device-usbip#11 sev=medium blast=leaf effort=S verdict=actionable - the crate's declared wire types (`UsbipEventSource`, `UsbipReconcileAttemptContext`, `UsbipPublicDegradedReason`, `UsbipClaimSource`) have no serde round-trip test with a real-shaped payload, so kebab-case/camelCase wire drift would pass - fix: add a round-trip test deserializing a hand-written payload for each serde type and re-serializing - [src/reconcile_state.rs:51-308, src/state_machine.rs:98-100]
  evidence: serde seeds = 25 hits in src but tests/ contains zero `serde_json`/`from_value`/`to_value`/`to_vec` hits; the daemon consumes the vocabulary via `to_public_reason` (d2bd/src/composition.rs:9556-9798), so the wire shapes are live contract
- clean: the suite is behavior-focused (call-order arrays, phase transitions, error variants not Display strings, redaction canaries at wrong_zone_and_redaction.rs:77-98); table-driven loops carry failure messages (state_machine.rs:730-754, vocabulary.rs:88-96); no `#[ignore]`, no network/time dependence; `integration/attach_detach_lifecycle.rs` is a declaration-only policy-required scaffold (recorded class, U1 (d) 5-6, not flagged)

## Coverage
- idiom: 1 finding(s)
- own: 1 finding(s)
- type: clean (seeds ran: 4/1/0)
- api: 2 finding(s)
- err: 1 finding(s)
- serde: clean (seeds ran: 8/12/1/3)
- obs: clean (seeds ran: 0/0/0/45)
- docs: 2 finding(s)
- perf: clean (seeds ran: 4/5/12)
- conc: 1 finding(s)
- async: clean (seeds ran: 18/0/4/0)
- unsafe: N/A (seeds: 0/0/0 all zero; manifest `unsafe_code = "forbid"`)
- ffi: N/A (seeds: 0/0/0/0 all zero; no foreign boundary)
- macro: N/A (seeds: 0/0/0/0 all zero; no macro definitions)
- test: 3 finding(s)