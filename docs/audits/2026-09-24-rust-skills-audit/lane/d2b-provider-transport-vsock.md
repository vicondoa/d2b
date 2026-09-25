# d2b-provider-transport-vsock - d2b-provider-transport-vsock
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 4128 (excl. src/generated/**) | modules: whole crate
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none (whole crate)

## idiom
- d2b-provider-transport-vsock#1 sev=low blast=leaf effort=S verdict=actionable - `ReadySession::disconnect` takes `mut self`, assigns `SessionState::Disconnected` to a by-value copy that is immediately dropped, and then returns the assigned constant  -  the mutation is dead code and the method contract is fully expressed by returning the constant. - fix: `pub fn disconnect(self) -> SessionState { SessionState::Disconnected }`, dropping `mut` and the state assignment - [packages/d2b-provider-transport-vsock/src/auth.rs:221-224]
  evidence: idiom seeds ran:0/3/0; site found by full-file read of auth.rs.

## own
- clean: all 12 hits (seeds ran:5/0/7/0) are explainable:the five `.clone()` sites share `BridgeControl`/`ReadySession`/`GuestIdentity` values where the clone is the ownership transfer the spanned bridge task or session object needs; the seven `Arc<Mutex<...>>` fields back genuinely multi-owner service state (active/completed maps, per-entry history/phase/exit, subscriber list) shared between the service and its spawned bridge tasks, matching the U1 shared-state false-positive note.



## type
- clean: seeds all zero (0/0/0) but lens is applicable (the crate declares many structs/enums): no `validate_`/`check_` fns, boolean flag soup, or stringly state;`VsockTransportSettings`'s validate-at-callsite shape (pub fields + later `validate()`) is reported under serde as a wire-boundary validation gap rather than here.



## api
- clean: seeds 106/0/10; the pub surface is the deliberate single-path `lib.rs` re-export pattern(10 arms), every exported item documented under `#![deny(missing_docs)]`; no `Arc`/`Rc`/`Box`/`RefCell` or dependency types appear in public signatures, and the three effect-port traits keep small required surfaces with associated types for the per-implementer stream/handle types.



## err
- clean: seeds 8/1/0/7; the 6 relay.rs `.expect("reservation")`/`.expect("listener")` sites are invariant assertions after an explicit `Some)...)` assignment in the same function (accepted by the skill's "expect names the invariant" rule),the 2 service.rs unwraps live under `#[cfg(test)]` mod tests, and the single `let _ =` is a deliberately ignored best-effort watch send in `BridgeControl::stop`.



## serde
- d2b-provider-transport-vsock#4 sev=medium blast=wide effort=M verdict=needs-contract - `VsockTransportSettings` deserializes untrusted wire JSON with no boundary validation: invalid `guest_ref`/`connect_timeout_seconds` land as ordinary values and are only rejected by later explicit `validate()` calls (in `new()` and `ZoneLinkSpec::validate`), and the all-public fields let any caller build an invalid settings value silently; a parse-once `try_from` type would reject once at the wire. - fix: `#[serde(try_from = "VsockTransportSettingsWire")]` with a private raw wire shape +`TryFrom` validation, plus private fields and accessors; wire field names/schema stay unchanged - [packages/d2b-provider-transport-vsock/src/settings.rs:16-27, packages/d2b-provider-transport-vsock/src/settings.rs:42-50, packages/d2b-provider-transport-vsock/tests/schema.rs:13-18]
  evidence: serde seeds ran:2/4/0/0; census: `VsockTransportSettings` over packages/, tests/, nixos-modules/, labs/, docs/reference/ = 17 hits, all within this crate + its tests; wire shape pinned by the committed schema at docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json (referenced from settings.rs:56).

## obs
- d2b-provider-transport-vsock#2 sev=low blast=leaf effort=S verdict=actionable - the 9 transport-open rejection warn events log `endpoint`/`binding` fields whose `Display` impls are constants ("opaque-endpoint"/"opaque-binding"), so the named fields cannot correlate any event to a specific transport  -  an operator debugging repeated opens sees identical values every time. - fix: drop the `endpoint`/`binding` fields from the open-reject warn events, or give `OpaqueEndpointId`/`OpaqueBindingId` a real `Display` over `self.0` while keeping `Debug` redacted - [packages/d2b-provider-transport-vsock/src/service.rs:392, packages/d2b-provider-transport-vsock/src/service.rs:466, packages/d2b-provider-transport-vsock/src/service.rs:65-67, packages/d2b-provider-transport-vsock/src/service.rs:99-101]
  evidence: obs seeds ran:0/0/0/13; static read of the 9 warn-field sites.
- d2b-provider-transport-vsock#3 sev=low blast=leaf effort=M verdict=actionable - bridge-drop,and bridge-copy-failure debug events carry no transport identity (no handle, endpoint, or binding field), so with up to `MAX_ACTIVE_TRANSPORTS` concurrent transports an operator cannot tell which one dropped an event or failed a copy  -  thread a handle/endpoint identity into the open path's spawned bridge task and through `emit_event`. - fix: capture `endpoint_id`/`binding_id` into the `tokio::spawn` block in `open_transport` and pass them to `emit_event`, adding named fields to the three drop events and the `run_bridge` copy-failure site - [packages/d2b-provider-transport-vsock/src/service.rs:735, packages/d2b-provider-transport-vsock/src/service.rs:766, packages/d2b-provider-transport-vsock/src/service.rs:851, packages/d2b-provider-transport-vsock/src/bridge.rs:186]
  evidence: obs seeds ran:0/0/0/13; static read of the emit_event/drop sites.



## docs
- d2b-provider-transport-vsock#5 sev=low blast=leaf effort=M verdict=actionable - 38 public `Result`-returning items carry no `# Errors` doc sections, so callers must infer from doc prose which condition yields which failure variant  -  the crate's `#![deny(missing_docs)]` (lib.rs:3) secures only first sentences, not the canonical contract sections. - fix: add `# Errors` bullet lists naming the failure variant per condition to the public Result-returning items (e.g. `GuestIdentity::new`, `SessionAuthority::authenticate`, `VsockTransportSettings::new`, `ZoneLinkSpec::validate`, `open_transport`, `NativeGuestRelay::start`) - [packages/d2b-provider-transport-vsock/src/auth.rs:59, packages/d2b-provider-transport-vsock/src/auth.rs:257, packages/d2b-provider-transport-vsock/src/settings.rs:31, packages/d2b-provider-transport-vsock/src/service.rs:383, packages/d2b-provider-transport-vsock/src/relay.rs:201]
  evidence: docs seeds ran:106/0/38; the canonical-sections seed (`/// # (Examples|Errors|Panics|Safety)`) returned zero hits.



## perf
- clean: seeds ran:0/3/0; the 3 collection-init hits (two `HashMap::new()` for the active/completed tables, one `Mutex::new(Vec::new())` for subscribers) are genuinely dynamic bounded maps or an empty-case-common vector, and no `format!` or `to_string()` appears in src  -  no allocation hot path to flag (static, unmeasured).

## conc
- clean: seeds ran: 0/7/17/0; the atomics match the skill's model: `BridgeStats` counters use `Relaxed` (pure counters), `next_handle.fetch_add(Relaxed` (uniqueness-only handle generation), the `done` `AtomicBool` uses the paired Release-store/Acquire-load handoff with re-check after arming `Notify`,and the seven `Arc<Mutex<...>>` fields are tokio async mutexes genuinely shared between service and per-transport bridge tasks.



## async
- clean: seeds ran:111/1/0/0; no lock guard crosses an `.await`, every effect open/close and named-stream open are wrapped in `timeout`(with the tokio-clock deadline rationale documented in open_transport),the spawned bridge task uses only async I/O (`copy_bidirectional`, AsyncWriteExt/AsyncReadExt),and the tokio `Mutex`/`Notify`/`watch` selection matches the workload  -  no blocking call on an executor worker found.



## unsafe
- N/A (seeds:0/0/0 all zero; no unsafe blocks/fns/impls or `// SAFETY:` comments; seed4 (`unsafe_code`=2) is only the `#![forbid(unsafe_code)]` at lib.rs:4 plus the crate manifest's mirror table, which do not make the lens applicable.



## ffi
- N/A (seeds:0/0/0/0 all zero; no `extern "C"`/`no_mangle`, `catch_unwind`, `repr(C)`/`repr(transparent)`, or `CStr`/`CString`/`c_char` surface exists in the crate).



## macro
- N/A (seeds:0/0/0/0 all zero; no `macro_rules!`, proc-macro trees (`proc_macro`/`syn::`/`quote!`/`$crate`/`to_compile_error`/`new_spanned`) exist in the crate).



## test
- d2b-provider-transport-vsock#6 sev=low blast=leaf effort=S verdict=actionable - tests/observe.rs asserts `ServicePhase::Ready == ServicePhase::Ready`, a self-comparison that cannot fail and adds nothing to a test already asserting the observation fields  -  dead assertion weight with no regression value. - fix: delete line 15 (the test still asserts `observation.phase == TransportPhase::Released`), or replace with a real cross-variant assertion (e.g. `assert_ne!(ServicePhase::Ready, ServicePhase::Serving)` - [packages/d2b-provider-transport-vsock/tests/observe.rs:14-15]
  evidence: test seeds ran:38/84/0/0; the tautology found by full-file read of tests/observe.rs; rest of the suite asserts error variants, uses table-driven virtual-clock deadlines (`drive_until_settled`), redaction canaries, and bounded eviction behavior  -  no `#[ignore]`, flaky, or implementation-restating tests found.



## Coverage
- idiom: 1 finding (seeds:0/3/0)
- own: clean (seeds ran:5/0/7/0)
- type: clean (seeds ran:0/0/0)
- api: clean (seeds ran:106/0/10)
- err: clean (seeds ran:8/1/0/7)
- serde:1 finding (seeds:2/4/0/0)
- obs:2 findings (seeds:0/0/0/13)
- docs:1 finding (seeds:106/0/38)
- perf: clean (seeds ran:0/3/0)
- conc: clean (seeds ran:0/7/17/0)
- async: clean (seeds ran:111/1/0/0)
- unsafe: N/A (seeds:0/0/0 all zero; no unsafe blocks/fns/impls or SAFETY comments)
- ffi: N/A (seeds:0/0/0/0 all zero; no extern/"C", catch_unwind, repr(C)/transparent, or CStr surface)
- macro: N/A (seeds:0/0/0/0 all zero; no macro_rules! or proc-macro machinery)
- test:1 finding (seeds:38/84/0/0)