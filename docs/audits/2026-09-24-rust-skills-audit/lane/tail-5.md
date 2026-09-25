# tail-5 - tail lane
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 3127 (excl. src/generated/**) | modules: d2b-provider-telemetry-service, d2b-provider-test-controller, d2b-provider-transport-unix, d2b-provider-wayland-session, d2b-provider-zone
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crates

## d2b-provider-telemetry-service

### idiom
- clean: seeds ran 0/1/0 - the one hand-written impl is `Default for TelemetryServiceDriverFactory` delegating to `new()` (driver.rs:90-94), the idiomatic shape; no index loops, no statement-style accumulation.

### own
- clean: seeds ran 13/2/0/0 - every clone is explainable: `key.clone()` into the driver's own key (driver.rs:159), `spec.base().clone()` into the owned envelope (driver.rs:151), `endpoint_ref.clone()` into `present_endpoints` (driver.rs:299), plus test-fixture clones; no Rc/RefCell/Arc<Mutex>/Cow.

### type
- tail-5#1 sev=low blast=leaf effort=S verdict=actionable - `TelemetryServiceStatus` carries stringly-typed state: `phase: &'static str` (three spellings via `PHASE_*` consts) and `projection: serde_json::Value` built by hand with `json!` at three sites, so the `{serviceRole, serviceReadiness}` pair is constructed and indexed by string - fix: add a `TelemetryServicePhase` enum with `as_str()` for the three spellings and a two-field `TelemetryServiceProjection` struct that serializes to the same contract-pinned shape (`SERVICE_STATUS_ALLOWED` spellings at d2b-contracts-provider/src/v3/semantic_services/telemetry.rs:55), replacing the `json!` literals at driver.rs:274-290 and 309-315 - [packages/d2b-provider-telemetry-service/src/driver.rs:119-125, packages/d2b-provider-telemetry-service/src/driver.rs:309-315]
  evidence: type seeds s1=2 (the trait-required `validate` and a test fn - not findings), s2=0, s3=0; public-surface read of the exported status struct

### api
- clean: seeds ran 16/1/1 - the single `Arc<dyn SpecDecoder>` in a public signature (driver.rs:147) is the house decoder contract required by `typed_spec_decoder`; `pub use` re-export arms in lib.rs:34-36 are the house single-surface pattern; every exported item is deliberate (driver, factory, descriptor, decoder, status, error).

### err
- tail-5#2 sev=medium blast=leaf effort=S verdict=actionable - `reconcile_service` replaces the manager's `ResourceError` with the stable `Reconcile` kind via `Err(_) => return Err(...)`, dropping the source, so the actor sees only the wire code and the underlying store failure is invisible - fix: log the source before converting (the crate has no tracing dependency today) or carry it as a `#[source]` field on `TelemetryServiceDriverError` - [packages/d2b-provider-telemetry-service/src/driver.rs:304]
  evidence: err seeds s1=13 (all inside `#[cfg(test)] mod tests`, lines 601-879), s2=2 (both `let _ = self.envelope(...)?` - propagated, not swallowed), s3=4 (test panics), s4=1
- tail-5#3 sev=low blast=leaf effort=S verdict=actionable - `ingest_endpoint_refs` silently drops unparseable declared refs (`ResourceRef::parse(value).ok()` inside `filter_map`), so a typo'd `ingestEndpointRefs` entry is indistinguishable from an absent list and the row requeues on the 5s resync forever with no signal - fix: log a warning naming the dropped value, or fail the reconcile with `InvalidResource` - [packages/d2b-provider-telemetry-service/src/driver.rs:386]
  evidence: err s2=2; the `.ok()` swallow is outside the seed's `\.ok\(\);` shape (no trailing semicolon) - read-based

### serde
- clean: seeds ran 0/0/0/3 - the crate crosses no wire with derives; serde_json use is the preserved envelope decode (`from_slice::<ResourceSpec>`, `to_canonical_bytes` round-trip, driver.rs:141-151), and `validate` is the decode admission gate; the canonical-bytes round-trip in `value()` is preserved old-reconciler behavior (driver.rs:141-144, documented).

### obs
- N/A: seeds 0/0/0/0 all zero; Cargo.toml carries no tracing/log dependency (the crate reports through its typed error codes only)

### docs
- clean: seeds ran 16/0/14 - `#![deny(missing_docs)]` (lib.rs:8) and every pub item carries a contract doc; the Result-returning items are `ResourceDriver` trait impls whose failure contract lives in the trait; no canonical-section gaps on inherent pub items.

### perf
- clean: seeds ran 3/7/0 - all `format!` hits are test-fixture log strings; the `Vec::new()` sites are cold paths or empty-case-common collections (`watched`, empty `present_endpoints`); no hot-loop allocation.

### conc
- clean: seeds ran 0/4/0/0 - all four `Mutex<` hits are `tokio::sync::Mutex` in the `#[cfg(test)]` recording fakes; no threads, atomics, or manual Send/Sync in the crate.

### async
- clean: seeds ran 73/0/1/9 - the 73 await hits are the driver verbs (validate/recover/reconcile/delete/watch_once) over the runtime's async `ResourceContext`; no spawn, no spawn_blocking, no blocking call in an async context, no guard held across await; the 9 `#[tokio::test]` sites are tests; the one `tokio::sync::Mutex` is test-only.

### unsafe
- N/A: seeds 0/0/0 all zero; manifest `unsafe_code = "forbid"` (Cargo.toml [lints.rust])

### ffi
- N/A: seeds 0/0/0/0 all zero

### macro
- N/A: seeds 0/0/0/0 all zero

### test
- clean: seeds ran 13/39/0/0 - 13 tests (9 driver-level over a recording manager endpoint + requeue recorder, 4 registration-boundary) assert behavior and error variants (`matches!` on `ProviderDirectoryError::DuplicateType`/`RequiredBeforeOpen`, `FailureClass::Retryable`), cover all four reconcile branches (degraded/projection/pending/fail-closed), and use deterministic fakes; no `#[ignore]`, no property tooling needed at this size.

### Coverage
- idiom: clean (seeds ran: 0/1/0)
- own: clean (seeds ran: 13/2/0/0)
- type: 1 finding(s)
- api: clean (seeds ran: 16/1/1)
- err: 2 finding(s)
- serde: clean (seeds ran: 0/0/0/3)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency in Cargo.toml)
- docs: clean (seeds ran: 16/0/14)
- perf: clean (seeds ran: 3/7/0)
- conc: clean (seeds ran: 0/4/0/0)
- async: clean (seeds ran: 73/0/1/9)
- unsafe: N/A (seeds: 0/0/0 all zero; manifest forbid)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 13/39/0/0)

## d2b-provider-test-controller

### idiom
- clean: seeds ran 0/0/0 - bin-only crate with plain sequential code; no index loops, no hand-written impls, no accumulation statements.

### own
- clean: seeds ran 0/1/0/0 - the single `.to_vec()` (main.rs:194) copies the bootstrap protocol marker into the owned packet payload, the one ownership-transfer boundary; no clones, no shared-state types.

### type
- clean: seeds ran 0/0/0 - `SessionDisposition` is a proper two-variant enum; no boolean/string state; the `Result<(), ()>` retry signal is deliberate for the fixture loop.

### api
- N/A: seeds 0/0/0 all zero; the crate is bin-only ([[bin]] Cargo.toml:24-26) with no lib target and no pub items

### err
- tail-5#4 sev=low blast=leaf effort=S verdict=actionable - `send_bootstrap` and `controller_transport` drop every failure reason with `map_err(|_| ())` (AncillaryCapacity, credit scopes, packet build, send burst, transport build), and the caller logs only the generic retry line, while sibling sites log `reason = %e` - fix: log the reason at each drop site with `warn!(reason = %e, ...)` before converting to `()` - [packages/d2b-provider-test-controller/src/main.rs:186-187, packages/d2b-provider-test-controller/src/main.rs:196-199, packages/d2b-provider-test-controller/src/main.rs:221-230]
  evidence: err seeds s1=9 (all inside `#[cfg(test)] mod tests`), s2=0, s3=1 (test-only panic), s4=0

### serde
- N/A: seeds 0/0/0/0 all zero (no wire crossing in the bin)

### obs
- tail-5#5 sev=medium blast=leaf effort=S verdict=actionable - the bin emits `tracing` events with structured `reason = %e` fields but never installs a subscriber (main() at main.rs:33-47; Cargo.toml has `tracing` but no tracing-subscriber), so every debug/warn/error event is dropped and the only operator-visible diagnostics are the unstructured `eprintln!` retry lines at main.rs:68/95/99/111/125 - one failure class reported through two channels, one of which is dead - fix: install a subscriber once at process start (e.g. `tracing_subscriber::fmt::init()`), or convert the tracing sites to eprintln - [packages/d2b-provider-test-controller/src/main.rs:33-47, packages/d2b-provider-test-controller/src/main.rs:68]
  evidence: obs seeds s1=5 (eprintln), s2=4, s3=0, s4=1; Cargo.toml dependency read shows no subscriber crate
- tail-5#6 sev=low blast=leaf effort=S verdict=actionable - message-only warn events drop their context: the keepalive error is discarded via `.is_err()` and logged as a bare message, and the unexpected named stream's id is unnamed - fix: bind the error (`warn!(reason = %e, ...)`) and name the stream (`warn!(stream = ?stream, ...)`) - [packages/d2b-provider-test-controller/src/main.rs:164, packages/d2b-provider-test-controller/src/main.rs:173-174]
  evidence: obs s2=4 (message-only `warn!`/`error!`/`debug!` sites; the other two are startup messages without a field to attach)

### docs
- N/A: seeds 0/0/4 - seed 1 (pub items) is zero; bin-only crate, and the skill never adds missing_docs to a binary

### perf
- clean: seeds ran 0/0/0 - no format!, no grow-by-push collections, no copies; the retry sleeps are the only pacing.

### conc
- N/A: seeds 0/0/0/0 all zero (no threads, locks, atomics, or thread_local in the crate)

### async
- clean: seeds ran 14/0/0/0 - the 14 await hits are the session loop, bootstrap send, and retry sleeps, all on the current-thread runtime built at process entry (`block_on` at main.rs:45 is the sanctioned entry-point pattern); no spawn, no blocking call in an async context, no guard across await.

### unsafe
- N/A: seeds 0/0/0 all zero; `#![forbid(unsafe_code)]` (main.rs:3) and manifest forbid

### ffi
- N/A: seeds 0/0/0/0 all zero

### macro
- N/A: seeds 0/0/0/0 all zero

### test
- clean: seeds ran 3/8/0/0 - three meaningful tests: a `should_reconnect` disposition table, a behavioral handshake-terminality test over a real socketpair with `select!`/`timeout` proving one-shot bootstrap delivery, and a fail-closed spawn of the bin without fd10; error variants asserted, deterministic, no `#[ignore]`.

### Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: clean (seeds ran: 0/1/0/0)
- type: clean (seeds ran: 0/0/0)
- api: N/A (seeds: 0/0/0 all zero; bin-only crate, no lib target)
- err: 1 finding(s)
- serde: N/A (seeds: 0/0/0/0 all zero)
- obs: 2 finding(s)
- docs: N/A (seeds: 0/0/4; seed 1 zero - no pub items in a bin-only crate)
- perf: clean (seeds ran: 0/0/0)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: clean (seeds ran: 14/0/0/0)
- unsafe: N/A (seeds: 0/0/0 all zero; forbid)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 3/8/0/0)

## d2b-provider-transport-unix

### idiom
- clean: seeds ran 1/1/0 - the single index loop is the bounded 8-attempt handle-collision retry with early return (portal.rs:345), where a plain `for` is right; the hand-written `Default for TransportPortal` delegates to `new()` (portal.rs:147-150), the idiomatic shape.

### own
- clean: seeds ran 0/0/0/0 - no clones, no to_owned, no shared-state types; ownership is moved end to end (`OwnedFd` transfers, `into_parts`, `into_transport_fd`).

### type
- clean: seeds ran 2/0/0 - the two `validate_*` fns are the boundary admission checks (`validate_route_class`, `validate_and_prepare`), exactly where parse-once validation belongs; `attachments_enabled: bool` is a single flag whose illegal combination is rejected at the same boundary, below the skill's stopping rule.

### api
- clean: seeds ran 47/0/3 - deliberate closed surface: opaque redacted `TransportHandle`, private-field `OpenedTransport`/`TransportDescriptor` with accessors, `pub use` named re-export arms (lib.rs:17-23), no Arc/Rc/Box/RefCell in signatures; the zero in-tree callers are the documented declared-provider class (provider-crate-policy ratchet, refusal ledger), not a surface defect.

### err
- clean: seeds ran 1/0/0/2 - the single `expect("finalized order is populated")` (portal.rs:141) sits behind a `len() > MAX_OPEN_TRANSPORTS` check the compiler cannot see and names the invariant; both error enums are closed stable-code surfaces with `From` mapping between them; no swallowed errors.

### serde
- N/A: seeds 0/0/0/0 all zero (the crate crosses no wire; descriptors are kernel-observed, not serialized)

### obs
- tail-5#7 sev=low blast=leaf effort=S verdict=actionable - four message-only `tracing::warn!` events drop the underlying errno (`map_err(|_|)` then warn with only the `provider` field): peer-credential bind failure, monitor-fd duplication, observation poll, and entropy-source failures - fix: capture the errno as `reason = %e` like the admission-rejection site at portal.rs:209-212 already does - [packages/d2b-provider-transport-unix/src/portal.rs:217-220, packages/d2b-provider-transport-unix/src/portal.rs:244-247, packages/d2b-provider-transport-unix/src/portal.rs:301-304, packages/d2b-provider-transport-unix/src/portal.rs:348-351]
  evidence: obs seeds s1=0, s2=0 (the `tracing::warn!(` form does not match the interpolated-message seed), s3=0, s4=9; read-based

### docs
- tail-5#8 sev=low blast=leaf effort=S verdict=actionable - the three inherent pub methods returning `Result` (`open`, `close`, `observe`) lack `# Errors` sections naming which conditions produce which `PortalError` variant, though the failure conditions are recoverable from the enum docs - fix: add `# Errors` sections to the three doc comments - [packages/d2b-provider-transport-unix/src/portal.rs:197-201, packages/d2b-provider-transport-unix/src/portal.rs:266, packages/d2b-provider-transport-unix/src/portal.rs:286]
  evidence: docs seeds s1=41, s2=0, s3=7; `#![deny(missing_docs)]` (lib.rs:3) is satisfied but the canonical-section rule is not

### perf
- clean: seeds ran 0/4/0 - the `Vec::new`/`HashMap::new`/`HashSet::new`/`VecDeque::new` hits are the empty portal's initial state (portal.rs:60-66) and a test fixture; no format! in the crate; handle generation is bounded at 8 attempts.

### conc
- tail-5#9 sev=low blast=leaf effort=S verdict=actionable - `tokio::sync::Mutex` (portal.rs:18, 155) in a crate with zero async code - every use is `try_lock()` on a synchronous path, so the tokio `sync` feature dependency exists solely for this one lock - fix: use `std::sync::Mutex` (the crate's own `try_lock`-only pattern never awaits) - [packages/d2b-provider-transport-unix/src/portal.rs:18, packages/d2b-provider-transport-unix/src/portal.rs:155]
  evidence: conc seeds s1=0, s2=1, s3=0, s4=0; async seeds s1=0, s2=0, s3=1 (this same Mutex import), s4=0

### async
- clean: seeds ran 0/0/1/0 - the crate has no async fn, no await, no spawn; the single `tokio::sync::Mutex` hit is judged under conc (tail-5#9); nothing here blocks an executor because there is no executor.

### unsafe
- N/A: seeds 0/0/0 all zero; manifest `unsafe_code = "forbid"` (Cargo.toml [lints.rust])

### ffi
- N/A: seeds 0/0/0/0 all zero (rustix syscall wrappers are not a foreign-caller boundary)

### macro
- N/A: seeds 0/0/0/0 all zero

### test
- clean: seeds ran 7/19/0/0 - six integration tests over real socketpairs assert error variants (`PortalError::PeerCredentials`/`SocketKindMismatch`/`AttachmentPolicyConflict`/`HandleTableFull`/`UnknownHandle`), kernel-bound peer credentials, fd-substitution refusal, idempotent close, foreign-handle refusal, disconnect observation, and full-table recovery; one unit test covers handle-reissue; deterministic, no `#[ignore]`.

### Coverage
- idiom: clean (seeds ran: 1/1/0)
- own: clean (seeds ran: 0/0/0/0)
- type: clean (seeds ran: 2/0/0)
- api: clean (seeds ran: 47/0/3)
- err: clean (seeds ran: 1/0/0/2)
- serde: N/A (seeds: 0/0/0/0 all zero)
- obs: 1 finding(s)
- docs: 1 finding(s)
- perf: clean (seeds ran: 0/4/0)
- conc: 1 finding(s)
- async: clean (seeds ran: 0/0/1/0)
- unsafe: N/A (seeds: 0/0/0 all zero; forbid)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 7/19/0/0)

## d2b-provider-wayland-session

### idiom
- clean: seeds ran 0/1/0 - the hand-written `Default for WaylandSession` (wayland_session.rs:96-98) builds the `Arc<dyn DisplayChildSource>` the derive cannot, delegating through `new()`; `desired_children` already uses the iterator pipeline (`intents.iter().map(owned_child_ensure).collect()`).

### own
- clean: seeds ran 4/0/0/0 - the four clones (wayland_session.rs:116-119) copy the spec's domain refs into the owned dependencies Vec, the required ownership transfer; `Arc<dyn DisplayChildSource>` is genuine shared ownership (see api).

### type
- clean: seeds ran 0/0/0 - `InteractionKind`/`InteractionType` typestate comes from the shared wayland-policy engine; no boolean/string state in this crate; `DisplayChildSource` is a one-required-method trait.

### api
- clean: seeds ran 12/2/1 - the two `Arc<dyn DisplayChildSource>` signature hits (wayland_session.rs:84, 97) are genuine shared ownership: `WaylandSession` derives `Clone` and clones share the source, with `Default` supplying `SessionChildSource`; `pub use` named re-export arms (lib.rs:11-20) are the house single-surface pattern; `WaylandSessionDriver`/`WaylandSessionFactory` aliases follow the family convention.

### err
- tail-5#10 sev=low blast=leaf effort=S verdict=actionable - `SessionChildSource::display_children` maps any `WorkerEffectError` from the display crate's child derivation to `InteractionEffectError::InvalidResource`, dropping the cause, and the crate has no tracing, so the derivation failure detail is invisible at the boundary - fix: log the source before mapping (add a tracing dependency) or preserve the specific variant - [packages/d2b-provider-wayland-session/src/wayland_session.rs:73]
  evidence: err seeds s1=0, s2=0, s3=0, s4=0; read-based (the `map_err(|_| ...)` at wayland_session.rs:73 drops `WorkerEffectError` from `display_owned_child_intents`, session_children.rs:42-46)

### serde
- N/A: seeds 0/0/0/0 all zero (spec decoding is delegated to the family's `spec_decoder` in wayland-policy; this crate defines no wire types)

### obs
- N/A: seeds 0/0/0/0 all zero; Cargo.toml carries no tracing/log dependency (failures travel as typed error codes only)

### docs
- clean: seeds ran 12/0/4 - `#![deny(missing_docs)]` (lib.rs:7) and every pub item carries a contract doc; the Result-returning items are `InteractionType` trait impls whose failure contract lives in the trait.

### perf
- clean: seeds ran 0/0/0 - no format!, no grow-by-push collections, no copies; the crate is a thin declaration layer over the shared engine.

### conc
- N/A: seeds 0/0/0/0 all zero

### async
- N/A: seeds 0/0/0/0 all zero (no async code in this crate; the engine's async verbs live in wayland-policy)

### unsafe
- N/A: seeds 0/0/0 all zero; manifest `unsafe_code = "forbid"` (Cargo.toml [lints.rust])

### ffi
- N/A: seeds 0/0/0/0 all zero

### macro
- N/A: seeds 0/0/0/0 all zero

### test
- clean: seeds ran 6/20/0/0 - six integration tests assert the declaration surface, registry duplicate refusal (`matches!` on `ProviderDirectoryError::DuplicateType`), the four-dependency read order, foreign-row refusal (error variant), the two child rows' materialized spec/metadata content, and a refusing child source; deterministic, no `#[ignore]`.

### Coverage
- idiom: clean (seeds ran: 0/1/0)
- own: clean (seeds ran: 4/0/0/0)
- type: clean (seeds ran: 0/0/0)
- api: clean (seeds ran: 12/2/1)
- err: 1 finding(s)
- serde: N/A (seeds: 0/0/0/0 all zero)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency in Cargo.toml)
- docs: clean (seeds ran: 12/0/4)
- perf: clean (seeds ran: 0/0/0)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero)
- unsafe: N/A (seeds: 0/0/0 all zero; forbid)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 6/20/0/0)

## d2b-provider-zone

### idiom
- clean: seeds ran 0/0/0 - no index loops, no hand-written impls (all derives), no accumulation statements; the emitter's `match`/`Option::replace` flow is expression-shaped.

### own
- clean: seeds ran 2/0/0/0 - the two `last_reconciled_at.clone()` hits (zone_status.rs:19, 131) are the first-borrow-then-move pattern in `emit_handler_status`/`emit`, the required copy for the two handler records; no shared-state types.

### type
- clean: seeds ran 0/0/0 - `ZoneStatusInput` is a builder with private fields, `ZoneRuntimeMetadata` is a flat counter struct, `generation_cleanup_pending: bool` is a single flag below the stopping rule; `ZoneStatusProjectionError` is a one-variant enum carrying the stable wire code, not a flag.

### api
- tail-5#11 sev=medium blast=leaf effort=M verdict=actionable - `pub mod zone_status;` plus `pub use zone_status::*;` (lib.rs:9-10) exposes every zone_status item at two paths (crate root and module path), violating the house single-surface pattern (named re-export arms with a private module, cf. telemetry-service lib.rs:34-36 and wayland-session lib.rs:11-20) - fix: make the module private and re-export the four items by name (`SystemCoreStatusEmitter`, `ZoneRuntimeMetadata`, `ZoneStatusInput`, `ZoneStatusProjectionError`), updating the module-path call sites - [packages/d2b-provider-zone/src/lib.rs:9-10, packages/d2b-provider-zone/src/zone_status.rs:43]
  evidence: api seeds s1=11, s2=0, s3=2; census: `d2b_provider_zone::zone_status` over packages/ = 3 hits (d2bd/src/resource_runtime.rs:63-65, tests/zone_status.rs:3-4)

### err
- clean: seeds ran 0/0/0/1 - the single error enum is a closed stable-code surface (`zone-status-projection-invalid`); no unwrap/expect/panic outside tests; the `map_err(|_| Contract)` at zone_status.rs:139 converts a caller-constructed input violation, where the stable code is the contract.

### serde
- N/A: seeds 0/0/0/0 all zero (no wire types defined in this crate; status projection consumes contract types)

### obs
- N/A: seeds 0/0/0/0 all zero; Cargo.toml carries no tracing/log dependency

### docs
- tail-5#12 sev=low blast=leaf effort=S verdict=actionable - the inherent pub `SystemCoreStatusEmitter::emit` returns `Result` (zone_status.rs:113-114) without an `# Errors` section naming the contract-rejection condition - fix: add an `# Errors` section stating that duplicate system-core handler records or a rejected `ZoneStatusResource` yield `ZoneStatusProjectionError::Contract` - [packages/d2b-provider-zone/src/zone_status.rs:110-114]
  evidence: docs seeds s1=10, s2=0, s3=1; `#![deny(missing_docs)]` (lib.rs:7) is satisfied but the canonical-section rule is not

### perf
- clean: seeds ran 0/0/0 - no format!, no grow-by-push collections, no copies; `Vec::with_capacity(input_handlers.len() + 2)` (zone_status.rs:120) sizes the only allocation.

### conc
- N/A: seeds 0/0/0/0 all zero

### async
- N/A: seeds 0/0/0/0 all zero (the emitter is a synchronous projection; async verbs live in the runtime)

### unsafe
- N/A: seeds 0/0/0 all zero; manifest `unsafe_code = "forbid"` (Cargo.toml [lints.rust])

### ffi
- N/A: seeds 0/0/0/0 all zero

### macro
- N/A: seeds 0/0/0/0 all zero

### test
- clean: seeds ran 5/12/0/0 - four emitter tests (exact mandatory system-core pair, malformed input cannot publish Ready, duplicate handler records rejected, metadata/timestamp projection) plus the policy-required registration shim calling the shared `assert_metadata_registration`; behavior and error assertions, deterministic, no `#[ignore]`.

### Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: clean (seeds ran: 2/0/0/0)
- type: clean (seeds ran: 0/0/0)
- api: 1 finding(s)
- err: clean (seeds ran: 0/0/0/1)
- serde: N/A (seeds: 0/0/0/0 all zero)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency in Cargo.toml)
- docs: 1 finding(s)
- perf: clean (seeds ran: 0/0/0)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero)
- unsafe: N/A (seeds: 0/0/0 all zero; forbid)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 5/12/0/0)