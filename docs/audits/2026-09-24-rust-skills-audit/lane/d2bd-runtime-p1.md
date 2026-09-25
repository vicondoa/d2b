# d2bd-runtime-p1 - d2bd-runtime - part 1/4
Baseline: 6ebdd4cec | LOC audited: 10715 (excl. src/generated/**) | modules: supervisor (dag, pidfd_table, readiness_liveness, state), typed_error, autostart, component_session_vsock, daemon_config, resource_api, zone_authority, shell_backend, broker_transport, public_read_model, vm_start_support, json_io
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 1/4: src/supervisor/**, src/typed_error.rs, src/autostart.rs, src/component_session_vsock.rs, src/daemon_config.rs, src/resource_api.rs, src/zone_authority.rs, src/shell_backend.rs, src/broker_transport.rs, src/public_read_model.rs, src/vm_start_support.rs, src/json_io.rs

## idiom
- d2bd-runtime-p1#1 sev=low blast=leaf effort=S verdict=actionable - build_autostart_plan accumulates two Vecs with side-effect loops then extends a third, where an iterator pipeline partition would express the split - fix: replace the two push loops in build_autostart_plan with a collector pair: `let (net_entries, workload_entries): (Vec<_>, Vec<_>) = resolver.manifest.vms.iter().map(|(name, vm)| { ... }).partition(|e| e.is_net_vm);` then sort each half - [autostart.rs:228-245]
  evidence: seed3 `let mut \w+ = (String|Vec)::new\(\)` = 10 hits; hit sites 228-229 are the statement-style split being judged (other hits are test fixtures or map-key builders)
- clean: seeds 1 `for \w+ in 0\.\.` = 4 (all fixed-count test loops in pidfd_table.rs:990-1015,1412-1415 - deliberate retry bounds), seed2 `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 5 (hand-written Default impls for AutostartConfig, ArtifactPaths, DaemonConfig, NodeBudget preserve fixed-path/invariant defaults a derive would break - justified), seed3 = 10 (one suspect site above, all other hits are owned map keys or test fixtures).

## own
- d2bd-runtime-p1#2 sev=low blast=leaf effort=S verdict=actionable - DagExecutor::run_split clones `state` into api_ready then matches the same value by move, when matching `&state` would keep it - fix: `match &state { .. }`, bind `ApiReadyState::Error { reason }` by reference in the format! call, and set `api_ready = Some(state)` after the match - [dag.rs:423-424]
  evidence: seed1 `\.clone\(\)` = ~75 hits; this clone is the only avoidable one (api_ready then match by move; the enclosing value is not used afterwards in the match arms other than the cloned copy)
- clean: seed2 `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = ~110 hits (map keys, Display/remediation strings, test fixtures - all explainable one-liners), seed3 `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 3 (zone_authority.rs:181,198,220 - the deliberate U17 shared coordinator handle, documented at lines 177-180), seed4 `Cow<` = 0

## type
- d2bd-runtime-p1#3 sev=low blast=leaf effort=S verdict=actionable - ComponentSessionTransportFailure carries `kind: String` in four Io variants, where std::io::ErrorKind would carry the same class as a typed value - fix: replace the `kind: String` fields of PeerCredentialIo/ConnectIo/WriteIo/AckIo with `io::ErrorKind` and map at call sites (component_session_vsock.rs:150,198,381,410), dropping the `.to_string()` round-trips - [component_session_vsock.rs:32-36]
  evidence: seed3 `(mode|kind|state): String` = 6 hits; 4 of them are these Io variants (the other 2 - typed_error.rs:615 RuntimeCapabilityUnsupported payload and ::700 ErrorEnvelope.kind - wire/deliberate contract shapes)
- clean: seeds 1 `fn validate_\w+|fn check_\w+` = ~7 (component_session_vsock path-policy checks and broker_transport::validate_instance - boundary validators on wire/path input, parse-at-boundary is the right place for them), seed2 `is_\w+: bool|\w+_flag: bool` = 3 (VmAutostartEntry.is_net_vm/autostart flags - each independently meaningful, no combined invariant)

## api
- d2bd-runtime-p1#4 sev=medium blast=family effort=M verdict=actionable - EstablishedShell exposes `pub backend: Arc<dyn ShellBackend>`, pushing an Arc + trait-object + the whole backend trait into the public field surface, when callers only need the three trait methods - fix: make the field private, add `handle_op`/`close_attachment`/`cancel_attachment` delegating methods on EstablishedShell, and update the d2bd/src/composition.rs call sites (13403,13460,13517,13625,13677)) - [shell_backend.rs:52-53]
  evidence: seed2 `pub .*\b(Arc|Rc|Box|RefCell)<` = 3 hits; census: `EstablishedShell` over packages/nixos-modules/tests/docs/reference/labs = 9 hits (5 cross-crate field reads in d2bd/src/composition.rs - the field is genuinely consumed, so the fix is delegation, not deletion)
- d2bd-runtime-p1#5 sev=low blast=leaf effort=S verdict=actionable - CachedPublicFrame is pub with pub fields (including a serde_json::Value dependency field)but only used inside public_read_model; the struct is dead public surface - fix: make CachedPublicFrame (and its fields) module-private or pub(crate, keep the ArcSwapOption slots private - [public_read_model.rs:51-53]
  evidence: seed1 `^\s*pub (fn|struct|enum|trait|const|mod) ` = ~110 hits; census: `CachedPublicFrame` over packages/nixos-modules/tests/docs/reference/labs = 5 hits, all inside public_read_model.rs (lines 52,60,61,115,139) - no consumer outside the module
- clean: seed2 `pub .*\b(Arc|Rc|Box|RefCell)<` = 3 hits (zone_authority.rs:181 new_coordinator Arc<Mutex<ZoneCoordinator>> - documented U17 shared handle with awaitable entry points; shell_backend.rs:53 - flagged above), seed3 `^\s*pub use ` = 1 (supervisor/pidfd.rs:3 re-export of the pidfd_table surface - house single-surface pattern)

## err
- d2bd-runtime-p1#6 sev=high blast=leaf effort=S verdict=actionable - default_audit_join_context panics with `.expect("canonical broker zone digest")` on a wire-supplied digest - a malformed request from the broker client crashes the daemon instead of returning a refusal - fix: propagate the parse failure (e.g. `CanonicalAuditDigest::parse(zone_id).ok()?;` or map into TypedError::WireInvalidFrame/InternalConfig),and only attend None when digest missing route review-pass - [broker_transport.rs:63,65]
  evidence: seed1 `\.unwrap\(\)|\.expect\(` = ~60 hits; production hits are only these 2 (both with wire-derived values via request.authoritative_audit_join()); every other hit sits in #[cfg(test)] modules or asserts a construction invariant (dag.rs:344,406, state.rs:403,411, pidfd_table.rs:496, typed_error.rs:1266)
- clean: seed2 `let _ = |\.ok\(\);` = ~30 (reply.send best-efforts in autostart.rs:394, test fixture joins, OnceLock::set one-shot setters, parent-dir sync best-effort - deliberate per site); seed3 `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = ~12 (all in #[cfg(test)] modules - test assertion style); seed4 `enum \w*Error` = ~10 (TypedError + four closed kind enums + ResourceRuntimeError, ZoneAuthorityError, ModeBoundBrokerError, DagError, PidfdTableError, ProcStatError, SnapshotStoreError - taxonomy split by caller action with wire_kind()/code()/label() accessors, no string-matching callers)

## serde
- clean: seeds 1 `derive\([^)]*(De)?[Ss]erialize` = ~25, seed2 `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` = ~30, seed3 `impl .*Deserialize.*for` = 1, seed4 `serde_json::from_|serde_json::to_` = ~20; config/snapshot types carry rename_all + deny_unknown_fields + per-field serde(default) with default_* fns consistent with hand-written Default; the only hand-written deserializer (ApiReadyState in dag.rs:89-108)is a deliberate wire admission gate over an untagged Helper enum for the `"yes"|"pending"|"timeout"|{"error": str}` shapes, round-trip tested at dag.rs:1058-1113 - not flagged

## obs
- clean: seeds 1 `\bprintln!\(|\beprintln!\(` = 0, seed2 `(info|debug|warn|error|trace)!\("` = 0, seed3 `\.instrument\(|#\[instrument` = 0, seed4 `tracing::|log::` = ~25; every tracing event carries named fields (kind, path, detail, busid, vm, role, error, generation),the TypedError::log_raw_detail boundary logs the chain once with the unredacted detail deliberately kept out of the public envelope, and no secret/identifier-only fields beyond the ADR 0010/0028 redaction gate scope were found

## docs
- d2bd-runtime-p1#7 sev=medium blast=leaf effort=S verdict=actionable - Three public helpers in json_io.rs carry no doc comments, though their semantics are non-obvious (absolute-vs-relative bundle path resolution, manifest-must-be-object) - fix: add one-line-then-detail doc comments (`# Errors` for the Result fns)on resolve_bundle_artifact_path and load_manifest - [json_io.rs:10,41]
  evidence: seed1 `^\s*pub (fn|struct|enum|trait|const|type) ` = ~110 hits; spot-check of the file (65 LOC) found 2 undocumented pub items
- d2bd-runtime-p1#8 sev=medium blast=leaf effort=S verdict=actionable - Public fns in vm_start_support.rs lack docs while siblings are documented; role->mode mapping, tracked_role_id, and store-view-intent resolution are contract-relevant for the d2bd composition - fix: add one-line-first-sentence docs (+ `# Errors` for the Result fn)on vm_start_node_mode, tracked_role_id, resolve_store_view_intent_for_guest - [vm_start_support.rs:14,44,89]
  evidence: seed1 = ~110 hits; full-file read (186 LOC) found 3 undocumented pub items (neighboring items have docs - inconsistent coverage)
- d2bd-runtime-p1#9 sev=medium blast=leaf effort=S verdict=actionable - ShellTerminalOp, ShellTerminalResponse,and EstablishedShell (a cross-crate contract type) carry no doc comments - fix: add doc comments naming each op/response variant's wire twin and the EstablishedShell lifetime/ownership contract - [shell_backend.rs:14,21,52]
  evidence: seed1 = ~110 hits; item-list read of shell_backend.rs found 3 undocumented pub items (EstablishedShell is consumed by d2bd/src/composition.rs:13703)
- d2bd-runtime-p1#10 sev=medium blast=leaf effort=S verdict=actionable - Five broker_transport helpers (audit-join extraction, deadline arithmetic, kind extraction, two launcher redaction renderers)carry no docs, and two of them format operator-facing remediation strings - fix: add one-line-first-sentence docs naming input contracts and output shapes, with `# Panics` on default_audit_join_context identified - [broker_transport.rs:60,69,116,128,187]
  evidence: seed1 = ~110 hits; targeted raw reads of broker_transport.rs found 5 undocumented pub fns (the file's other fns carry /// docs (e.g. dispatch_broker_request_to_socket, ModeBoundBrokerAdapter))
- clean: seed2 `/// # (Examples|Errors|Panics|Safety)` = 0, seed3 `-> Result<` = ~55; Result-returning items mostly carry #-style contract prose in prose form; no doctests exist in this lane (acceptable: no pure example-worthy boundary items in the lane scope)

## perf
- d2bd-runtime-p1#11 sev=low blast=family effort=M verdict=actionable - load_list/load_status clone the entire cached serde_json::Value frame per call (`then(|| cached.value.clone())`), making every public status/list poll allocate a full copy of the read-model frame - fix: return `Option<Arc<CachedPublicFrame>>` (or `&Value` tied to the Arc swap guard)from load_if_fresh and let the wire renderer borrow the Value; update the d2bd composition call sites - [public_read_model.rs:117-118]
  evidence: static (unmeasured) - no benchmark exists for the public-read path; seed1 `format!\(` = ~70 (all in error strings, remediation rendering, and test fixtures - cold paths), seed2 `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = ~15 (empty-case-common collection builds and test fixtures), seed3 `\.to_string\(\)` = ~110 (Display/error/remediation strings - the artifact is the text)

## conc
- clean: seeds 1 `std::thread::|thread::spawn|thread::scope` = ~9 (autostart run_phase dedicated bounded worker threads per plan R4 (documented at autostart.rs:352-356)and test threads in pidfd_table/component_session_vsock), seed2 `\bMutex<|\bRwLock<` = ~9 (PidfdTable RwLock<BTreeMap>+mutation_lock Mutex serializing register/snapshot sequences, BrokerReapLog Mutex, InMemorySnapshotStore Mutex (test-only), FakeStarter/FakeRunner Mutexes (cfg(test))), seed3 `Atomic\w+|Ordering::` = ~25 (SNAPSHOT_TMP_COUNTER/next-id Relaxed counters, PidfdTable generation AcqRel/Acquire pairs, PublicStatusReadModel AcqRel/Acquire CAS publish loop - weakest correct orderings for the handoff each guards), seed4 `thread_local!|unsafe impl (Send|Sync) for` = 0; no manual Send/Sync claims, no shared-state-among-threads mis-model found

## async
- clean: seeds 1 `async fn|async move|\.await` = ~90, seed2 `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = ~12 (JoinSet spawns in autostart run_phase and dag executor tests; `spawn_blocking` = 0 - dedicated threads per R4 replace it), seed3 `tokio::sync::(Mutex|RwLock|Notify)` = ~3 (zone_authority coordinator Mutex - guard held only across synchronous calls, documented U17), seed4 `#\[tokio::(main|test)\]|Runtime::block_on` = ~16 (tokio::test marks; block_on sites in shell_backend.rs:112,224,241 carry `#[allow(clippy::disallowed_methods, reason = "synchronous path")]` - sanctioned blocking-census entries); no guard held across an await, no blocking call on an executor worker,and no cancellation-unsafe irreversible step found

## unsafe
- clean: seeds 1 `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, seed2 `// SAFETY:` = 0, seed3 `transmute|from_raw|MaybeUninit|mem::zeroed` = 4 (all `rustix::process::Pid::from_raw` / `rustix::process::Signal::from_raw` safe constructors in pidfd_table.rs:401,744,808and state.rs:321 doc prose - no actual unsafe blocks/UB hazards), seed4 `unsafe_code` = 2 (documentation: state.rs:322and the daemon workspace lint inherit - no allow); the lane contains no unsafe code, so no SAFETY comments are owed

## ffi
- N/A: seeds 1-4 all zero; no FFI surface exists in the assigned files (libc::c_int signal numbers are syscall-adjacent but never cross a foreign caller)

## macro
- N/A: seeds 1 `macro_rules!` = 0, seed2 `proc_macro|syn::|quote!` = 0, seed3 `\$crate` = 0, seed4 `to_compile_error|new_spanned` = 0 all zero; no macro definitions or proc-macro machinery in the lane

## test
- clean: seeds 1 `#\[test\]|#\[tokio::test\]` = ~120 (unit tests per module + 2 boundary tests in tests/runtime_boundary.rs), seed2 `assert_eq!\(|assert_ne!\(|assert!\(` = ~320 (behavioral assertions with per-case messages, error-variant matches not Display strings), seed3 `proptest!|insta::assert|rstest` = 0 (no property/snapshot tooling; hand-built case tables with failure messages cover the parser/classifier edges adequately for the closed input classes), seed4 `#\[ignore\]` = 0 (no ignored tests); tests are deterministic (fixed `/proc/stat` fixtures, injected fakes, no network, tempdir-scoped state),and the boundary test suite locks the provider-implementation-free contract

## Coverage
- idiom: 1 finding(s)
- own:  1 finding(s)
- type:  1 finding(s)
- api:  2 finding(s)
- err:  1 finding(s)
- serde: clean (seeds ran: ~25/~30/1/~20; all shapes deliberate; one hand-written admission gate with round-trip test)
- obs: clean (seeds ran: 0/0/0/~25; named-field events only, no println, no interpolated message-only logs, no secret fields)
- docs:  4 finding(s)
- perf:  1 finding(s)
- conc: clean (seeds ran: ~9/~9/~25/0; worker-thread model and lock/atomic orderings match the workload shapes)
- async: clean (seeds ran: ~90/~12/~3/~16; dedicated R4 workers, no awaits-under-lock, no executor blocking)
- unsafe: clean (seeds ran: 0/0/4/2; only safe rustix::process::Pid::from_raw constructors; no unsafe blocks to justify)
- ffi: N/A (seeds: 0/0/0/0 all zero; no FFI surface)
- macro: N/A (seeds: 0/0/0/0 all zero; no macros)
- test: clean (seeds ran: ~120/~320/0/0; deterministic behavior-focused unit+boundary suite, error variants asserted, no ignored/property tests needed for the closed input classes)