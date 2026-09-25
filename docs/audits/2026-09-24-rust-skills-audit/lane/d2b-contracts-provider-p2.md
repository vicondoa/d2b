# d2b-contracts-provider-p2 - d2b-contracts-provider - part 2/2
Baseline: 6ebdd4cec | LOC audited: 7395 (excl. src/generated/**) | modules: v3/semantic_services (mod, audio, child_resources, security_key, telemetry, usb), v3/credential_controller, v3/telemetry_policy, v3/telemetry_frame
Lenses: idiom own type api err serde obs docs perf conc async unsafe ffi macro test | Partitions: src/v3/semantic_services/**, src/v3/credential_controller.rs, src/v3/telemetry_policy.rs, src/v3/telemetry_frame.rs

## idiom
- d2b-contracts-provider-p2#1 sev=low blast=leaf effort=S verdict=actionable - the two `children.push(BindingChildIntent {...})` arms in `explicit_binding_children_with_user` are identical 15-field literals differing only in `producer_ref: None` versus `Some(producer_ref)`, forced apart by a `let ... else { ...; continue; }` - fix: bind `let producer_ref: Option<ResourceRef> = producer_ref.transpose()?;` before the push and emit one literal with `producer_ref,`, deleting the else-continue arm - [packages/d2b-contracts-provider/src/v3/semantic_services/child_resources.rs:573, packages/d2b-contracts-provider/src/v3/semantic_services/child_resources.rs:595]
  evidence: seed `let mut \w+ = (String|Vec)::new\(\)` = 1 hit (test-support, mod.rs:1276); the duplication is a read finding at child_resources.rs:574-616; seeds 1 and 2 = 1 and 0 hits
- clean: seeds ran: 1/0/1 - no index loops outside one test fixture (telemetry_frame.rs:555), no hand-written Default/From/PartialEq/Debug/Clone/Hash impls matched (the redacting `Debug` impls are `impl core::fmt::Debug` and are the deliberate redaction pattern), one statement-style accumulation in cfg(test) support code

## own
- d2b-contracts-provider-p2#2 sev=low blast=leaf effort=S verdict=actionable - duplicate-detection set in `validate_descriptor` clones every label key (`seen.insert(label.key.clone())`) where a borrowed `BTreeSet<&str>` suffices - fix: declare `let mut seen: BTreeSet<&str> = BTreeSet::new();` and insert `&label.key` - [packages/d2b-contracts-provider/src/v3/telemetry_policy.rs:497, packages/d2b-contracts-provider/src/v3/telemetry_policy.rs:500]
  evidence: seed `\.clone\(\)` = 30 hits, of which this is one of two non-test, non-construction clones; the other clones are multi-owner construction copies (frame field values, child intents, single-flight set insert) that pass the one-sentence test
- d2b-contracts-provider-p2#3 sev=low blast=leaf effort=S verdict=actionable - `allowed_telemetry_value` allocates a fresh String just to test zone validity (`validate_zone(value.to_owned()).is_ok()`) although `validate_zone` only reads the value - fix: give the zone grammar a `&str`-based check (for example `fn is_valid_zone(value: &str) -> bool` used here, keeping the owning `validate_zone` for the three construction call sites that need the validated String back) - [packages/d2b-contracts-provider/src/v3/credential_controller.rs:1591, packages/d2b-contracts-provider/src/v3/credential_controller.rs:1549]
  evidence: seed `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 25 hits; this is the only hit where the owned value is immediately discarded (`.is_ok()`), the rest are genuine owned returns or test fixtures
- clean: seeds ran: 30/25/0/0 - no Rc/RefCell/Arc<Mutex>/Arc<RwLock> and no Cow in the partition; every remaining clone is a multi-owner copy (child intents share binding/provider refs, telemetry frame fields appear in three label sets, single-flight set insert) or a `to_canonical_string` owned return

## type
- d2b-contracts-provider-p2#4 sev=medium blast=family effort=M verdict=actionable - `BindingChildRequest::process` and `process_for_user` accept `kind: BindingChildKind` including `Endpoint`, so an Endpoint carrying process fields is constructible and must be rejected at runtime (`InvalidProducer`, child_resources.rs:502-510), and the sibling check `producer_role.is_some() && kind != Endpoint` (child_resources.rs:499) is unreachable because only `endpoint()` sets `producer_role` and it hardcodes `Endpoint` - fix: take a restricted `ProcessChildKind { Process, EphemeralProcess }` in the two process constructors (all seven in-tree call sites pass `BindingChildKind::Process`), then delete both runtime checks - [packages/d2b-contracts-provider/src/v3/semantic_services/child_resources.rs:92, packages/d2b-contracts-provider/src/v3/semantic_services/child_resources.rs:499, packages/d2b-contracts-provider/src/v3/semantic_services/child_resources.rs:502]
  evidence: seed `fn validate_\w+|fn check_\w+` = 20 hits, all boundary validators on wire input or constructor invariants; census: `BindingChildRequest::process` over packages/nixos-modules/tests/docs/reference/labs = 7 hits in 4 crates (audio-pipewire, device-security-key, device-usbip, observability-otel), all passing `BindingChildKind::Process`; the illegal Endpoint-with-process-fields combination is exercised only by the runtime check, not by any caller
- clean: seeds ran: 20/0/0 - no boolean flag soup (`is_\w+: bool` = 0), no stringly-typed state (`(mode|kind|state): String` = 0); the remaining `validate_*` functions are parse-once admission checks on wire input, which is the skill's sanctioned boundary placement

## api
- clean: seeds ran: 231/0/1 - the exported surface is the deliberate wide contract vocabulary of a contract crate (U1 (c) api false positive applies), no Arc/Rc/Box/RefCell appears in any public signature, and the single `pub use` arm (telemetry_policy.rs:9) re-exports generated catalog constants as the house single-surface pattern; `SemanticPairDeclaration`, `NonEmpty`, and `SemanticBackingDeclaration` are correctly `pub(crate)`

## err
- d2b-contracts-provider-p2#5 sev=low blast=leaf effort=S verdict=actionable - `CredentialControllerError::AlreadyRunning` renders the wire label "credential-queue-pressure", which names a different concept (the lease-ceiling outcome `CredentialControllerOutcome::QueuePressure`) than the variant's documented meaning ("the same Credential is already being handled") - fix: emit "credential-already-running" from the Display arm, or rename the variant to match the code - [packages/d2b-contracts-provider/src/v3/credential_controller.rs:90]
  evidence: seed `enum \w*Error` = 6 error enums read; census: `credential-queue-pressure` over packages/nixos-modules/tests/docs/reference/labs = 1 hit (the definition itself), so the label is not pinned by docs/reference/error-codes.md or any consumer
- d2b-contracts-provider-p2#6 sev=low blast=leaf effort=S verdict=actionable - `CredentialObservabilityError` Display strings are prose sentences ("credential audit record is invalid", "credential telemetry frame is invalid"), breaking the kebab-code diagnostic convention every sibling error type in this crate follows (`CredentialControllerError`, `MetricPolicyError`, `TelemetryFrameError`, `SemanticContractError`, `BindingChildError`) - fix: render "credential-audit-record-invalid" and "credential-telemetry-frame-invalid" - [packages/d2b-contracts-provider/src/v3/credential_controller.rs:1508, packages/d2b-contracts-provider/src/v3/credential_controller.rs:1509]
  evidence: seed `enum \w*Error` = 6 enums; census: both prose strings over packages/nixos-modules/tests/docs/reference/labs = 1 hit each (the definitions), no consumer or doc pins them
- d2b-contracts-provider-p2#7 sev=low blast=leaf effort=S verdict=actionable - `CredentialSingleFlight` maps a poisoned mutex to `InvalidInput` (a caller-input error) and its guard `Drop` silently skips the removal on poison, which would leave a stale UID and a permanent `AlreadyRunning`; the skill names recovery via `into_inner()` for exactly this shape - fix: recover with `self.running.lock().unwrap_or_else(|poisoned| poisoned.into_inner())` in both `lock()` and `Drop`, keeping the documented synchronous-path boundary - [packages/d2b-contracts-provider/src/v3/credential_controller.rs:834, packages/d2b-contracts-provider/src/v3/credential_controller.rs:846]
  evidence: seed `\.unwrap\(\)|\.expect\(` = 40 hits, all in tests or on literally-built values (write! to String, canonical constants) per the U1 false-positive list; the poison mapping is a read finding at the two lock sites, both carrying the sanctioned `#[allow(clippy::disallowed_methods, reason = "synchronous path")]`
- clean: seeds ran: 40/1/1/6 - no panic!/unreachable!/todo!/unimplemented! outside one test helper (telemetry_policy.rs:990), the single `let _ =` hit is a compile_fail doctest, and every unwrap/expect outside tests is the U1 false-positive class (write! to String, literally-built catalog constants); error taxonomy is otherwise split by caller action with closed discriminants

## serde
- d2b-contracts-provider-p2#8 sev=low blast=leaf effort=S verdict=actionable - `parse_raw_frame` maps every serde failure to `Malformed`, so a top-level unknown field (rejected by `deny_unknown_fields` on `TelemetryFrame`) reports `Malformed` while the same unknown key nested inside `value` reports `UnknownField` from validation - the variant exists but is unreachable for the shape that names it - fix: `map_err(|error| match error.classify() { serde_json::error::Category::UnknownField => TelemetryFrameError::UnknownField, _ => TelemetryFrameError::Malformed })` (serde_json 1.0.151 in Cargo.lock provides `classify`) - [packages/d2b-contracts-provider/src/v3/telemetry_frame.rs:76, packages/d2b-contracts-provider/src/v3/telemetry_frame.rs:114]
  evidence: seed `derive\([^)]*(De)?[Ss]erialize` = 2 hits, seed `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` = 3 hits, seed `serde_json::from_|serde_json::to_` = 4 hits; the UnknownField-vs-Malformed asymmetry is a read finding across parse (line 76) and validate (lines 114-121)
- clean: `rename_all = "lowercase"` on `TelemetrySignal` and per-type `deny_unknown_fields` on `TelemetryFrame` follow the card's naming and decision guidance; the hand-written `Deserialize` for `SemanticProjectionProtocolVersion` is a live admission gate (grammar parse), the sanctioned pattern; no `flatten`, no `try_from`, no untagged

## obs
- clean: seeds ran: 0/0/0/0 - no println/eprintln, no interpolated log macros, no tracing or log usage anywhere in the partition; the telemetry frame and policy modules are the redaction and closed-domain policy data themselves, and `CredentialTelemetryFrame` builds structured field lists rather than log lines, so there is nothing to instrument

## docs
- d2b-contracts-provider-p2#9 sev=medium blast=family effort=M verdict=actionable - none of the 59 Result-returning items in the partition carries an `# Errors` section, so callers cannot learn from the docs which closed-discriminant error each condition produces (for example when `CredentialControllerCall::authorize` yields `DeadlineExceeded` versus `OperationDenied`, or which `validate_*` failure maps to which `MetricPolicyError` variant) - fix: add `# Errors` sections naming the variant per condition to the public Result APIs, starting with the constructor and validate families - [packages/d2b-contracts-provider/src/v3/credential_controller.rs:155, packages/d2b-contracts-provider/src/v3/telemetry_policy.rs:478, packages/d2b-contracts-provider/src/v3/telemetry_frame.rs:72, packages/d2b-contracts-provider/src/v3/semantic_services/mod.rs:93]
  evidence: seed `/// # (Examples|Errors|Panics|Safety)` = 0 hits against seed `-> Result<` = 59 hits; every public item carries a one-line doc (no undocumented pub items found), so this is the missing canonical-section class, not missing docs
- clean: every pub item in the partition has a doc comment with a load-bearing first sentence; module docs (`//!`) exist on all six modules; the redacting Debug/Display impls are documented policy

## perf
- clean: seeds ran: 11/6/25 - every `format!` site is a cold path (child-name construction, audit wire record rendering, error labels, test fixtures), every `Vec::new()` is an empty-case-common or cfg(test) site, and the `to_string`/`to_owned` sites are owned returns or the two `own` findings above; no hot loop, no attacker-keyed hashing, no grow-by-push in a measured path; static (unmeasured)

## conc
- clean: seeds ran: 0/1/0/0 - the only synchronization is `CredentialSingleFlight`'s `Mutex<BTreeSet<ResourceUid>>`, a documented synchronous-path boundary with the sanctioned `#[allow(clippy::disallowed_methods, reason = "synchronous path")]` per U1 (d) 4, held only across single insert/remove operations; no threads, no atomics, no thread_local, no unsafe Send/Sync claims

## async
- N/A (seeds: 0/0/0/0 all zero; no `async fn`, no `.await`, no tokio usage anywhere in the partition - the controller contract is synchronous by design, documented at credential_controller.rs:800-806)

## unsafe
- N/A (seeds: 0/0/0/0 all zero; no unsafe blocks, fns, impls, or SAFETY comments in the partition, and the crate inherits `unsafe_code = "forbid"` through `[lints] workspace = true`)

## ffi
- N/A (seeds: 0/0/0/0 all zero; no extern "C", no no_mangle, no repr(C)/repr(transparent), no CStr/CString in the partition)

## macro
- N/A (seeds: 0/0/0/0 all zero; no macro_rules!, no proc-macro or syn/quote usage, no $crate, no to_compile_error in the partition)

## test
- d2b-contracts-provider-p2#10 sev=medium blast=leaf effort=M verdict=actionable - several public contract behaviors have no test: `observe_credential` (both the degraded and the InspectMetadata branches), the rotation-retry-exhausted branch of `reconcile_credential` (`CredentialRetryState::exhausted` feeding `RotationFailed`/`Failed`), `CredentialLeaseAggregate::from_active_expiries`, `CredentialControllerHealth::derive`, and `CredentialAuditRecord::controller_event` - fix: add table-driven unit tests asserting the outcome/disposition variant per input row, mirroring the existing `rotation_policy_matrix_is_closed` shape - [packages/d2b-contracts-provider/src/v3/credential_controller.rs:691, packages/d2b-contracts-provider/src/v3/credential_controller.rs:1346, packages/d2b-contracts-provider/src/v3/credential_controller.rs:499, packages/d2b-contracts-provider/src/v3/credential_controller.rs:1084]
  evidence: seed `#\[test\]|#\[tokio::test\]` = 40 tests read across the partition; the named functions appear zero times inside `#[cfg(test)]` bodies (census over the crate's tests: `observe_credential` 0 test hits, `from_active_expiries` 0, `CredentialControllerHealth::derive` 0, `controller_event` 0, `CredentialRetryState` 0), while `reconcile_credential` and `revoke_credential` are exercised
- clean: seeds ran: 40/150/0/0 - no proptest/insta/rstest, no `#[ignore]`; the existing tests are behavior-asserting with negative controls (fingerprint re-derivation, provider-neutrality probes, redaction idempotence) and table-driven cases with messages; no test found that cannot fail

## Coverage
- idiom: 1 finding
- own: 2 findings
- type: 1 finding
- api: clean (seeds ran: 231/0/1; deliberate contract vocabulary, no internals in signatures, one house-pattern pub use)
- err: 3 findings
- serde: 1 finding
- obs: clean (seeds ran: 0/0/0/0; no logging surface in the partition)
- docs: 1 finding
- perf: clean (seeds ran: 11/6/25; all sites cold or test)
- conc: clean (seeds ran: 0/1/0/0; one documented synchronous-path Mutex with sanctioned allow)
- async: N/A (seeds: 0/0/0/0 all zero; synchronous contract by design)
- unsafe: N/A (seeds: 0/0/0/0 all zero; unsafe_code forbid inherited)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: 1 finding