# d2b-broker-p6 - d2b-broker - part 6/7
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 10,986 (excl. src/generated/**) | modules: envelope, ops/exec_reconcile, ops/audit_op, ops/store_view_posture, ops/device_worker, ops/nm, ops/security_key, ops/store_view_farm, ops/usbip_firewall
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 6/7 of d2b-broker per U1 section f: src/envelope/**, src/ops/exec_reconcile.rs, src/ops/audit_op.rs, src/ops/store_view_posture.rs, src/ops/device_worker.rs, src/ops/nm.rs, src/ops/security_key.rs, src/ops/store_view_farm.rs, src/ops/usbip_firewall.rs

## idiom
- d2b-broker-p6#1 sev=low blast=leaf effort=S verdict=actionable - the "path must be absolute" check (a `to_str()` + `starts_with('/')` + `InvalidInput` refusal) is hand-copied into seven SystemReconcileExecutor methods, so a wording or error-shape change must touch all seven. - fix: extract a private `fn require_absolute(path: &Path) -> Result<(), ReconcileExecError>` helper and call it from apply_nft_script, write_atomic_file, write_atomic_file_with_ownership, write_path_value, read_path_value, ip_route, run_usbip, run_ssh_keygen. - [src/ops/exec_reconcile.rs:404, src/ops/exec_reconcile.rs:505, src/ops/exec_reconcile.rs:551, src/ops/exec_reconcile.rs:602, src/ops/exec_reconcile.rs:622, src/ops/exec_reconcile.rs:642, src/ops/exec_reconcile.rs:697, src/ops/exec_reconcile.rs:811]
  evidence: idiom seeds over src/ops/exec_reconcile.rs = 1/0/0 (static read of the executor methods 392-926).
- d2b-broker-p6#2 sev=low blast=leaf effort=S verdict=actionable - write_atomic_file and write_atomic_file_with_ownership duplicate the parent-check + basename-extract + open_dir_path_safe preamble, differing only in the optional (u32, u32) ownership argument. - fix: fold into one `fn write_atomic_file(path: &Path, content: &[u8], owner: Option<(u32, u32)>) -> Result<(), ReconcileExecError>` and fold the `_with_ownership` twin into it (keeping a thin wrapper only if a caller outside the executor needs it) - [src/ops/exec_reconcile.rs:500-541, src/ops/exec_reconcile.rs:542-591]
  evidence: idiom seeds over src/ops/exec_reconcile.rs = 1/0/0 (static read of the two file-writing helpers).
- d2b-broker-p6#3 sev=low blast=leaf effort=M verdict=actionable - build_farm_via_namespace and build_store_view_via_namespace duplicate the same spawn-process + write-config + read-stdout + split-lines scaffold (about 100 lines each), drifting in error messages and success parsing. - fix: unify behind one private `async fn run_store_helper(verb: StoreViewHelperVerb, request: impl Serialize, success: impl FnOnce(&[u8]) -> ...) -> Result<(), StoreViewFarmError>` with a two-variant `StoreViewHelperVerb` enum, or extract the shared scaffold into a thin helper. - [src/ops/store_view_farm.rs:97-190, src/ops/store_view_farm.rs:228-300]
  evidence: idiom seeds over src/ops/store_view_farm.rs =  0/0/0 (static read of the two helper fns).
- d2b-broker-p6#4 sev=medium blast=leaf effort=M verdict=actionable - row_owner_ref, device_guest_owner,and tpm_devices_of_guest hand-roll the same "walk the bundles resources array" loop three times with slightly different match predicates, so bundle-shape drift (new field, renamed key) silently desyncs them. - fix: extract a single `fn find_resource_row<'a>(bundle: &'a Value, pred: impl FnMut(&'a Value) -> bool) -> Option<&'a Value>` and drive all three (and callers of row_owner_ref at src/ops/device_worker.rs:158) from it; keep the three predicates as call-site closures. - [src/ops/device_worker.rs:312-335, src/ops/device_worker.rs:339-369, src/ops/device_worker.rs:371-434]
  evidence: idiom seeds over src/ops/device_worker.rs =  0/0/1 (static read of the three bundle-walk fns.
.
- d2b-broker-p6#5 sev=low blast=leaf effort=M verdict=actionable - TrustedContextStore duplicates each worker-handshake entrypoint as a sync twin (`open`/`open_async`, `publish`/`publish_async`) whose sync copies spawn a `block_on`-free worker handshake that only in-crate `#[cfg(test)]` callers exercise;; the justification comment begins "The crate is nearly all async" and does not cover the sync twins' ongoing cost. - fix: gate the sync twins `#[cfg(test)]` (and gate `TrustedContextStore::Drop`'s sync persist path if unused outside tests), or unify over a private `fn with_worker<R>(blocking: bool, f: impl FnOnce...` seam if a production sync caller is restored. - [src/envelope/mod.rs:424-464, src/envelope/mod.rs:466-493, src/envelope/mod.rs:540-565, src/envelope/mod.rs:567-593]
  evidence: idiom seeds over src/envelope/mod.rs =  4/3/3 (static read of the four entrypoints 424-593.

## own
- d2b-broker-p6#6 sev=low blast=leaf effort=S verdict=actionable - in `context_worker_loop` the Bootstrap reply clones the entire persisted state (`let _ = reply.send(Ok(state.state.clone()))`) just to unblock open()/open_async(), which discard the reply value (`?`), so each broker open copies the whole `PersistedTrustedContext` for nothing. - fix: shrink `ContextCommand::Bootstrap`'s oneshot reply to `Sender<Result<(), TrustedContextStoreError>>` and send `Ok(())` without touching `state`;; delete the `state.state.clone()` site (keep the `let _ =` on the send alone). - [src/envelope/mod.rs:671]
  evidence: own seed `\.clone\(\)` over src/envelope/mod.rs = 24 hits; site 671 matched.

## type
- clean: deterministic-validators and stringly-state seeds checked;; the two `fn validate_*` sites (src/ops/nm.rs:151, src/ops/security_key.rs:82) are boundary cross-checks against existing config/authority state, not per-callsite re-validation;; the 10 `(mode|kind|state): String` occurrences are all audit-record or embedded-contract wire fields (state-posture-contract.json) pinned by the JSON drift gate, so no illegal-state combination is representable at the lens's bar.

.

## api

- d2b-broker-p6#7 sev=medium blast=leaf effort=L verdict=policy-confirmed - `lib.rs` declares `pub mod ops` (src/lib.rs:45) with `pub mod` arms for all 27 executor/helper modules in src/ops/mod.rs:20-94, so every item in them becomes part of the crate's public surface - while the recorded design rationale (src/lib.rs:8-11) is "public API of internal modules that downstream callers may use", and a census finds no downstream crate consumer.. - fix: if a deliberate public-surface decision is desired, keep `pub mod` and record the census in a doc comment or dossier;; otherwise narrow the `pub mod` arms to `pub(crate)` for modules with no out-of-crate consumer (ops/exec_reconcile, ops/audit_op, ops/store_view_posture, ops/store_view_farm, ops/device_worker, ops/security_key, ops/usbip_firewall, ops/nm)and re-export only test-consumed items (`OperationFields` for tests/security_key_broker.rs) under a `#[cfg(any(test, feature = "fake-backends")))]`-style gate. - [src/lib.rs:45, src/ops/mod.rs:20-94]
  evidence: census: `d2b_broker::ops` over packages/*/src, packages/*/tests, tests/, docs/reference/, labs/, nixos-modules/ = 10 hits (8 code imports in d2b-broker/tests/{bridge_lifecycle.rs:3, persistent_tap_lifecycle.rs:3, pidfd_handoff_scm_rights.rs:24,:25,:85, pidfd_real_spawner.rs:17, security_key_broker.rs:9};2 doc-comment mentions in d2b-host/src/{modules.rs:19, devices.rs:6}).



## err
- d2b-broker-p6#8 sev=low blast=leaf effort=M verdict=actionable - usbip_unbind_error_is_transient classifies retryable-vs-fatal usbip failures by case-folded substring matching over `stderr`/`error` text (42-Condition-Not-Satisfied, "program does not support"..., "no matching transport"), so a locale or usbip-version message change silently flips the retry decision and the broker's eventual verdict. - fix: parse the failure once at the stderr boundary into a typed `UsbipUnbindFailure { kind: UsbipUnbindFailureKind, transient: bool, detail: String }` (or a documented constant allowlist),and drive the retry loop (and final error reporting) off the typed kind instead of re-scanning strings. - [src/ops/exec_reconcile.rs:1238-1265]
  evidence: err seeds over src/ops/exec_reconcile.rs =  56/16/4/1 (static read of usbip_unbind_error_is_transient and its retry call sites 990-1070.
.
- d2b-broker-p6#9 sev=low blast=leaf effort=S verdict=actionable - guest_socket_directory returns `Result<&'static str,...>` with two plain-static-code errors (a "not root-owned" refusal, "no guest" refusal),while the sibling launch-scope pinner uses a typed `DeviceWorkerScopeError` enum - so an internal closed-error str forces callers (live_handlers.rs:2428) to stringly-match an error. - fix: give guest_socket_directory a small `GuestSocketError` enum (or reuse DeviceWorkerScopeError's callers-action split with a `GuestSocket` variant.)and return that instead of a `&'static str`. - [src/ops/device_worker.rs:262-284, src/ops/device_worker.rs:149, src/ops/live_handlers.rs:2428]
  evidence: err seed4 over src/ops/device_worker.rs =  1 (DeviceWorkerScopeError enum exists; guest_socket_directory uses the bare `&'static str` instead; static read of lines 262-284.

## serde
- clean: derive-counts, serde-attribute seeds, hand `Deserialize` impls,and serde_json calls checked;; wire-shaped structs in the lane carry `rename_all`, `deny_unknown_fields` (audit identity records, OpAuditRecord via parse_fields!), `skip_serializing_if`/`default` for optional fields, and legacy-compat tests pin optionality meaning;; the untagged `OperationFields` decomposition is deliberate (the JSON drift gate reads back fields per variant),and no missing-validation site warrants a `try_from` row..

## obs
- clean: println!/eprintln!/dbg! seeds zero;; tracing events (9 hits) all use named fields (`usbip_subcommand = %subcommand`, `path = %path.display()`, `error = %error`) with no msg-interpolation formatting;; no secrets in fields (audit paths are hashed/redacted at the wrapper boundaries by design).

## docs
- d2b-broker-p6#10 sev=low blast=leaf effort=S verdict=actionable - the two `pub async fn` store-view farm entrypoints carry the same design-journal sentence "Async form used by the async exec_reconcile and store_sync paths; the sync form was removed with its last sync caller" with a typo (missing space after `paths.`), stating history ("was removed") instead of a contract. - fix: trim to a one-line contract ("Async counterpart used by the async exec_reconcile/store_sync callers.") at both sites, and add `# Errors`-style failure notes where the error enum is non-obvious. - [src/ops/store_view_farm.rs:66-72, src/ops/store_view_farm.rs:191-197]
  evidence: docs seeds over src/ops/store_view_farm.rs =  0/0/5 (the two pub async fns are within the `-> Result<` hitset; static read of both docs.
.
- d2b-broker-p6#11 sev=low blast=leaf effort=S verdict=actionable - BrokerEnvelope::call, call_with_fds,and call_nested_with_fds return `Result<_, EnvelopeRefusal>` with no `# Errors` section, so failure modes (14 closed refusal-code consts, e.g. subscriber-only, scope refusals, budget refusals) must be chased around the file to be known. - fix: add an `# Errors` block to each of the three pub methods naming the closed `EnvelopeRefusal` vocabulary and pointing at the refusal constants. - [src/envelope/mod.rs:1118, src/envelope/mod.rs:1136, src/envelope/mod.rs:1187]
  evidence: docs seeds over src/envelope/mod.rs =  67/0/18 (canonical section hits zero amid 18 public `-> Result<` items; static read of the call trio.
.
- d2b-broker-p6#12 sev=low blast=leaf effort=S verdict=actionable - apply_with_reload/remove_with_reload doc prose narrates design history ("The dispatcher now lands on ops::nm even though the live path is still a thin wrapper... future coexistence/reload-verification work"), which rots and reads as rendered journal prose on a public item. - fix: rewrite the doc as a plain two-sentence contract (what it does, when reload verification kicks in),and move the rationale to the module doc if it must be preserved. - [src/ops/nm.rs:293-301, src/ops/nm.rs:303-308]
  evidence: docs seeds over src/ops/nm.rs =  12/0/9 (static read of the two wrapper docs 288-308.

## perf
- d2b-broker-p6#13 sev=low blast=leaf effort=S verdict=actionable - contract_store_view_levels re-parses the embedded `include_str!` JSON contract (STATE_POSTURE_CONTRACT) on every call (down per-VM posture passes; each row also re-parses the contract via contract_store_view_level, so the same ~600-line document is parsed many times per sync pass. - fix: pre-parse once into a `static CONTRACT: LazyLock<ContractFile>` (or `OnceLock`|and resolve per-row levels/profiles from the cached parse, deleting per-call `ContractFile::parse` sites. - [src/ops/store_view_posture.rs:194-271, src/ops/store_view_posture.rs:110-120, src/ops/store_view_posture.rs:310-352]
  evidence: static (unmeasured); hot-enough path only with many VMs; read of contract_store_view_levels 194-271 and its row sink 310-352).

## conc
- d2b-broker-p6#14 sev=low blast=leaf effort=S verdict=actionable - the invocation-id counter uses `Ordering::AcqRel` (`self.invocations.fetch_add(1, Ordering::AcqRel)`), but no reader of the counter or its derived id synchronizes on it - the returned old value is consumed only by the calling thread/audit record, so Relaxed is the weakest correct ordering. - fix: use `Ordering::Relaxed` at src/envelope/mod.rs:1146 (and at the test double's `observed.fetch_add` at src/envelope/mod.rs:2144). - [src/envelope/mod.rs:1146, src/envelope/mod.rs:2144]
  evidence: conc seeds over src/envelope/mod.rs =  5/7/10/0 (atomic hits include the AcqRel counter; static read of the counter's only use 1140-1160.

## async
- clean: async seeds (319 async/await, 3 tokio::spawn writer/stderr-drain tasks, 0 tokio::sync, 39 tokio main/test+block_on) checked;; the trusted-context store's worker uses the sanctioned bounded-worker pattern (async-gate-allow markers "dedicated bounded worker per plan R4" at src/envelope/mod.rs:699,725,798),the sync store twins (`open`/`epoch`/`holds_zone`/`publish`and Drop)run off the R4 worker thread by design,and no production path blocks an executor worker;; no guard is held across `.await`, no cancellation-safety hazard found (handler task abort at budget expiry is deliberate, KTD4; stdin-writer `tokio::spawn` at store_view_farm.rs:135,:247 is dropped after write, fine.

## unsafe
- clean: unsafe seeds over the lane =  0/0/6/0;; the six `from_raw` hits are all safe std/nix constructors (`io::Error::from_raw_os_error`, `Pid::from_raw`, `Gid::from_raw`),no `unsafe` block, unsafe fn, or unsafe impl exists in these files.



## ffi
- N/A: all four ffi seeds zero in these files; no FFI boundary lives in the lane - the crate's extern surface sits in src/sys.rs etc. (out of this part's scope)..



## macro
- clean: macro_rules! hits = 2 (src/ops/audit_op.rs:475 parse_fields!, src/ops/audit_op.rs:981 roundtrip_test!) - both are the legitimate "generate an impl per type from a small list" use (the parse_fields! macro generates serde impls for 40+ OperationFields variants with a single local pattern;,roundtrip_test! is test-only); no proc-macro, no `$crate`, no hygiene escape hatch..

## test
- clean: 109 #[test]/#[tokio::test] + 331 assert seeds, zero proptest/insta/rstest, zero #[ignore];; suite assertions target behavior (refusal codes, legacy-compat parses, fd-leg round-trips, causality-back timeouts),tests are deterministic (tempdir scratch roots, derived/seed paths,,no network, injected time),and no test that cannot fail was found in the lane's tests/**.



## Coverage
- idiom: 5 finding(s)
- own:  1 finding(s)
- type: clean (seeds ran:  2/0/10)
- api:  1 finding(s)
- err:  2 finding(s)
- serde: clean (seeds ran: 24/34/0/39)
- obs: clean (seeds ran:  0/0/0/9)
- docs:  3 finding(s)
- perf:  1 finding(s)
- conc:  1 finding(s)
- async: clean (seeds ran:  319/3/0/39)
- unsafe: clean (seeds ran:  0/0/6/0)
- ffi: N/A (seeds:  0/0/0/0 all zero; no FFI surface in the lane - the crate's extern boundary lives in src/sys.rs (out of this part's scope))
- macro: clean (seeds ran:  2/0/0/0)
- test: clean (seeds ran:  109/331/0/0)