# d2b-provider-process-systemd - d2b-provider-process-systemd
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 3816 (excl. src/generated/**, incl. tests/**) | modules: whole crate (src: audit, controller, drain, effects_service, error, launch, lib, lifecycle, metrics, operations, sandbox; tests: boundaries, conformance, controller, execution_parents, lifecycle)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate

## idiom
- d2b-provider-process-systemd#1 sev=low blast=leaf effort=S verdict=actionable - hand-written `impl Default` on the unit structs `SystemdEffectsService` and `SystemdEffectsServiceFactory` where `#[derive(Default)]` generates the identical impl - fix: replace both `impl Default { fn default() -> Self { Self::new() } }` blocks with `#[derive(Default)]` on the structs - [packages/d2b-provider-process-systemd/src/effects_service.rs:98, packages/d2b-provider-process-systemd/src/effects_service.rs:218]
  evidence: seed `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 3 hits; the third (`SystemdProviderConfig`, src/lifecycle.rs:20) is a legitimate hand-written Default preserving nonzero bounded defaults (a field-wise derive would yield zeros)
- clean: seeds ran: 0/3/0 - no index loops (`for \w+ in 0\.\.` = 0), no statement-style accumulation (`let mut \w+ = (String|Vec)::new\(\)` = 0); the only hand-written impls are the two unit-struct Defaults above plus the invariant-preserving config Default

## own
- clean: seeds ran: 14/29/0/0 - every `.clone()` is explainable: move-closure captures into `kernel_seat::run` (src/operations.rs:649-653), owned `ProcessStatusReport`/`UnitIdentity` fields (src/lib.rs:238-243, src/operations.rs:596-597), the payload clone into `ValidatedPayload` (src/effects_service.rs:184), and cfg(test) fixtures; `.to_owned()/.to_string()` sites are uid/path formatting, closure captures, and error-detail strings; no Rc/RefCell/Arc<Mutex>/Arc<RwLock>/Cow

## type
- d2b-provider-process-systemd#2 sev=low blast=leaf effort=S verdict=actionable - `RestartPolicy.restart_on_failure: bool` is invariant state: the only constructor sets it to `true` and nothing ever mutates it, so the field and its guard encode a state the type cannot otherwise represent - fix: drop the field and the `if !self.restart_on_failure` check in `should_restart`, or add a `RestartPolicy::never()` constructor if the never-restart class is real - [packages/d2b-provider-process-systemd/src/lifecycle.rs:78, packages/d2b-provider-process-systemd/src/lifecycle.rs:100]
  evidence: seed `is_\w+: bool|\w+_flag: bool` = 0; full-file read found the invariant field (single constructor `on_failure` at lifecycle.rs:87 sets it true; no other assignment)
- d2b-provider-process-systemd#3 sev=low blast=leaf effort=S verdict=actionable - `metrics::validate_labels` accepts stringly-typed `(String, String)` label pairs checked against the runtime `LABEL_KEYS` allowlist, so a misspelled key is a runtime rejection instead of a type error - fix: introduce `enum MetricLabelKey { Operation, Outcome, Domain }` with an `as_str()` accessor and take the key side typed - [packages/d2b-provider-process-systemd/src/metrics.rs:7, packages/d2b-provider-process-systemd/src/metrics.rs:4]
  evidence: seed `fn validate_\w+|fn check_\w+` = 3 hits (`validate_request` is a boundary admission gate cross-checking untrusted wire fields against the trusted bundle - not a parse-once candidate; `validate_launch_ticket` is a two-line provider-binding check); the label-key case is the stringly-typed one
- clean: seeds ran: 3/0/1 - the one `(mode|kind|state): String` hit is the external systemd `ActiveState` property read (src/operations.rs:565), a wire-boundary value, not crate state

## api
- d2b-provider-process-systemd#4 sev=low blast=leaf effort=S verdict=actionable - `SystemdProviderConfig`, `RestartPolicy`, `SystemdConfigError`, and `EphemeralProcessController` are each reachable at two paths: `pub mod lifecycle` (src/lib.rs:28) plus the root re-export `pub use lifecycle::{...}` (src/lib.rs:33), violating the one-path-per-item surface rule - fix: make `lifecycle` private (`mod lifecycle;`) and keep the root re-export as the single surface; no external caller imports through the module path (tests use the crate root) - [packages/d2b-provider-process-systemd/src/lib.rs:28, packages/d2b-provider-process-systemd/src/lib.rs:33]
  evidence: seed `^\s*pub use ` = 1 hit; seed `\bpub (fn|struct|enum|trait|type|const|mod) ` = 77 hits; the four re-exported items are the only two-path items (other modules are single-path)
- d2b-provider-process-systemd#5 sev=low blast=leaf effort=S verdict=actionable - `SystemdProviderConfig::no_persistent_unit()` is an always-true method with no production caller; the invariant it states already lives in the README security posture and the dossier - fix: delete the method and its test assertion (tests/lifecycle.rs:12), or replace it with a documented `const` if the surface is contract - [packages/d2b-provider-process-systemd/src/lifecycle.rs:55, packages/d2b-provider-process-systemd/tests/lifecycle.rs:12]
  evidence: census: `no_persistent_unit` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel, *.bzl = 2 hits (definition + its own test)
- d2b-provider-process-systemd#6 sev=low blast=leaf effort=S verdict=policy-confirmed - the controller/provider/lifecycle/drain/launch/sandbox/audit/metrics/error modules have zero production consumers: the daemon composes only `effects_service` + `operations` (the U15 forward seam), so the declared controller surface is unwired in the tree - fix: none until daemon composition lands; record the drift - [packages/d2b-provider-process-systemd/src/lib.rs:22, packages/d2b-provider-process-systemd/README.md:28]
  evidence: census: `SystemdProcessController|SystemdProcessProvider|SystemdReconcileAction|SystemdReconcileResult|EphemeralProcessController|RestartPolicy|SystemdProviderConfig|DrainProof|DrainStage|DrainError` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel, *.bzl = all hits in-crate (src + tests + README); d2bd consumes only `PROCESS_SYSTEMD_EFFECTS_SERVICE` + `SystemdEffectsServiceFactory` (d2bd/src/resource_plane_v3.rs:2282-2288, d2bd/src/forward_rendezvous.rs:2063-2065); policy: prior audit kept the dossier-named modules (docs/explanation/over-engineering-audit-record.md:262, row 4) and the dossier required layout names them (docs/specs/providers/ADR-046-provider-system-systemd.md:1348-1357); README declares the controller as the shipped library type (README.md:28-31)

## err
- d2b-provider-process-systemd#7 sev=low blast=leaf effort=S verdict=actionable - `SystemdProviderError` (src/error.rs) is a closed error catalogue with zero consumers while the live handlers refuse through the parallel `&'static str` code constants in src/operations.rs:51-107 - two refusal vocabularies in one crate - fix: delete the unused enum, or route the handler refusals through it (its codes are not pinned in docs/reference/error-codes.md, so no wire contract binds them) - [packages/d2b-provider-process-systemd/src/error.rs:5, packages/d2b-provider-process-systemd/src/operations.rs:51]
  evidence: census: `SystemdProviderError` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel, *.bzl = 4 hits, all in src/error.rs (enum, impl, Display, Error); seed `enum \w*Error` = 3 hits (SystemdProviderError, SystemdConfigError, DrainError)
- clean: seeds ran: 24/0/0 - every `.unwrap()/.expect()` is in `#[cfg(test)]` or on frozen literal construction (`BoundedToken::parse(PROVIDER_NAME)` and the profile build in `SystemdProcessProvider::new`, src/lib.rs:68-80; the `LazyLock` operation-table parses, src/operations.rs:161-181); no swallowed Results, no panic macros

## serde
- clean: seeds ran: 2/2/0/6 - Serialize-only audit projection (`SystemdAuditOperation` kebab-case, `SystemdProcessAudit` camelCase) with no raw unit name/PID/path fields; no hand-written Deserialize; the `serde_json::to_value/from_value` conversions at the operation boundary map failures to the closed refusal codes (UNIT_INVALID_REQUEST/UNIT_QUERY_FAILED) instead of stringified messages

## obs
- d2b-provider-process-systemd#8 sev=low blast=leaf effort=S verdict=actionable - the `debug!` event on the cancelled-ticket path evaluates `ticket.process_ref().to_canonical_string()` eagerly, allocating the canonical string even when debug is disabled - fix: pass a reference and let the macro format lazily (`resource = %ticket.process_ref()` if Display exists, else `?ticket.process_ref()`), reserving the eager `to_canonical_string()` for the warn/error paths - [packages/d2b-provider-process-systemd/src/lib.rs:139, packages/d2b-provider-process-systemd/src/lib.rs:141]
  evidence: seed `(info|debug|warn|error|trace)!\("` = 0 (every event uses named fields); full-file read found the eager field expression on the debug! site
- clean: seeds ran: 0/0/0/4 - no println/eprintln in the library; all tracing events carry named fields (provider, resource, identity, error, timeout_sec); no spans, which is consistent with one-shot handler operations

## docs
- d2b-provider-process-systemd#9 sev=medium blast=leaf effort=M verdict=actionable - public `Result`-returning items lack `# Errors` sections naming their failure conditions: `SystemdProviderConfig::new` (OutOfRange bounds), `drain::validate` (two refusal variants), `SystemdProcessController::reconcile` (DeadlineExceeded), `validate_launch_ticket`, `SystemdSandboxCompiler::compile` - fix: add `# Errors` sections to each, stating which inputs produce which failure - [packages/d2b-provider-process-systemd/src/lifecycle.rs:33, packages/d2b-provider-process-systemd/src/drain.rs:30, packages/d2b-provider-process-systemd/src/controller.rs:66, packages/d2b-provider-process-systemd/src/launch.rs:8, packages/d2b-provider-process-systemd/src/sandbox.rs:14]
  evidence: seed `/// # (Examples|Errors|Panics|Safety)` = 0 hits; seed `-> Result<` = 31 hits; seed `^\s*pub (fn|struct|enum|trait|const|type)` = 67 hits, all documented (the crate opts into `#![deny(missing_docs)]` at src/lib.rs:16)
- clean: seeds ran: 67/0/31 - every public item carries a doc comment with a one-line first sentence and every module has a `//!` doc; the gap is the canonical-section depth, not presence

## perf
- d2b-provider-process-systemd#10 sev=low blast=leaf effort=S verdict=actionable - `unit_name` builds the hex suffix with `format!` inside a 16-iteration loop (16 small String allocations) plus a final `format!`, on every unit operation that names a unit - fix: write the bytes into the preallocated `String::with_capacity(52)` with `write!` per byte, or format once into a fixed buffer - [packages/d2b-provider-process-systemd/src/operations.rs:417, packages/d2b-provider-process-systemd/src/operations.rs:421]
  evidence: seed `format!\(` = 20 hits, of which 18 are cold error/detail paths and one is the loop site; static (unmeasured)
- clean: seeds ran: 20/13/10 - `Vec::new()` sites are empty fixtures and the deliberately empty `auxiliary` argument to StartTransientUnit; `.to_string()` sites are uid/path and error-detail strings on cold paths

## conc
- N/A: seeds: 0/0/0/0 all zero - no threads, locks, atomics, or channels; the only shared state is `tokio::sync::Semaphore` (async-side, judged under async)

## async
- clean: seeds ran: 53/0/0/3 - no blocking work on the executor (the sync `/proc/sys/kernel/random/boot_id` read in `validate_request` carries the sanctioned per-site allow `#[allow(clippy::disallowed_methods, reason = "synchronous path")]` at src/operations.rs:208); zbus and `/proc/<pid>/stat` reads are async (tokio::fs); the kernel leg runs through `kernel_seat::run` with 'static captures; the semaphore permit held across `.await` in `reconcile` is the bounded-slot design (try_acquire_owned, never blocking); the timeout-drop of a mid-launch future is recoverable through the designed adoption path; the `tokio::runtime::Handle::try_current()` gate in reconcile is deliberate for the crate's single-poll test driver

## unsafe
- N/A: seeds: 0/0/0 all zero - no unsafe blocks/fns/impls, no SAFETY comments, no transmute/from_raw/MaybeUninit/zeroed; manifest sets `unsafe_code = "forbid"` (packages/d2b-provider-process-systemd/Cargo.toml, [lints.rust])

## ffi
- N/A: seeds: 0/0/0/0 all zero - no extern "C", no_mangle, catch_unwind, repr(C)/repr(transparent), or CStr/CString/c_char anywhere in the crate (zbus D-Bus calls are Rust-side, not FFI)

## macro
- N/A: seeds: 0/0/0/0 all zero - no macro_rules!, proc-macro, $crate, or spanned-error machinery; the crate defines no macros

## test
- clean: seeds ran: 43/102/0/0 - 43 tests (12 in src, 31 in tests/) assert observable behavior with human-written expectations and error-variant matching (`ProcessConformanceError::*`, `EffectServiceError::Declined`), never Display strings; deterministic (no network, explicit current-thread runtimes, 0-second timeouts for the timeout tests); the three runtime-driving tests carry the sanctioned `#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]`; no `#[ignore]`, no property/snapshot tooling (not required for this surface); the guest-binding gate test reads the real kernel boot id, which is stable within a boot

## Coverage
- idiom: 1 finding(s)
- own: clean (seeds ran: 14/29/0/0 - every clone explainable; no Rc/RefCell/Arc<Mutex>/Arc<RwLock>/Cow)
- type: 2 finding(s)
- api: 3 finding(s)
- err: 1 finding(s)
- serde: clean (seeds ran: 2/2/0/6 - Serialize-only redacted audit projection; wire conversions map to closed refusal codes)
- obs: 1 finding(s)
- docs: 1 finding(s)
- perf: 1 finding(s)
- conc: N/A (seeds: 0/0/0/0 all zero; no threads/locks/atomics/channels; the only shared state is tokio::sync::Semaphore, async-side)
- async: clean (seeds ran: 53/0/0/3 - no blocking on executor, sanctioned per-site allow cited, bounded-slot permit design, adoption-recoverable timeout)
- unsafe: N/A (seeds: 0/0/0 all zero; unsafe_code = "forbid" in Cargo.toml [lints.rust])
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 43/102/0/0 - behavior-based, error-variant assertions, deterministic, no ignored tests)