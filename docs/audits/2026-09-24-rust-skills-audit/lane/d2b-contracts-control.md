# d2b-contracts-control - d2b-contracts-control
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 5019 (excl. src/generated/**) | modules: whole crate (cli_output, proxy_readiness, public_wire, terminal_wire, unsafe_local_wire)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none (single-part lane)

## idiom
- clean: seeds `for \w+ in 0\.\.` / `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` / `let mut \w+ = (String|Vec)::new\(\)` ran at 0/0/0; manual check confirms every hand-written `Debug` impl (TerminalWriteStdin, ExecStartArgs, NamedProcessStreamRequest, ScopeIdentity, ShellName, ...) is a deliberate secret-redaction impl per the idiom card's repo false positives, and all other traits are derived.

## own
- clean: seeds `\.clone\(\)` / `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` / `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` / `Cow<` ran at 94 hits total, all explainable: `StatusServicesOutputV3::from_v2` clones String fields into an owned map (cli_output.rs:282-290), `.to_owned()` in the `ShellName` JsonSchema impl builds schema strings (public_wire.rs:1319-1333), and the rest are `#[cfg(test)]` fixtures; no Rc/RefCell/Arc/Cow anywhere.

## type
- d2b-contracts-control#1 sev=medium blast=wide effort=L verdict=needs-contract - `AuditResponse` pairs `complete: bool` with `next_cursor: Option<AuditExportCursor>`, encoding 4 states of which 2 are illegal, guarded only by the runtime `validate_audit_page` at deserialize - fix: replace the pair with an enum (`Complete` / `More(AuditExportCursor)`) so the illegal combos are unrepresentable, deleting `validate_audit_page` - [public_wire.rs:2166, public_wire.rs:2203]
  evidence: seed `(mode|kind|state): String` + `fn validate_\w+|fn check_\w+` = 22 hits; the complete/next_cursor invariant is the one Option-pair smell in the crate; wire change so needs-contract.
- d2b-contracts-control#2 sev=low blast=wide effort=L verdict=needs-contract - status DTOs carry stringly-typed state (`mode`, `state`, `kind`, `status` as `String`) mirroring daemon-side vocabularies instead of closed enums - fix: convert the closed vocabularies (realm mode, gateway state, qemu runner/registry state, read-model kind) to kebab-case enums on both daemon and wire sides - [cli_output.rs:99, cli_output.rs:123, public_wire.rs:2634, public_wire.rs:2672, public_wire.rs:2156]
  evidence: seed `(mode|kind|state): String` = 22 hits across cli_output.rs and public_wire.rs; shapes are pinned by docs/reference/cli-output schemas and daemon-api.md, so needs-contract.
- d2b-contracts-control#3 sev=low blast=wide effort=M verdict=needs-contract - `MutationFlags` models dry-run/apply/json as three booleans while the doc comment itself records that "the daemon rejects requests that set neither `dry_run` nor `apply`", i.e. an illegal state the type still represents - fix: encode the mode as an enum variant (e.g. `MutationMode::{DryRun, Apply}` plus a separate json flag) and delete the daemon-side runtime rejection - [public_wire.rs:316, public_wire.rs:311]
  evidence: seed `is_\w+: bool|\w+_flag: bool` = 22 hits; the neither-set rejection is documented at public_wire.rs:310-313; wire change so needs-contract.

## api
- d2b-contracts-control#4 sev=low blast=leaf effort=S verdict=actionable - `StatusServicesOutputV3` and its `from_v2` conversion shim are exported but have zero callers in the workspace; the doc comment says "Used so callers... can be migrated incrementally" but no migration landed - fix: delete `StatusServicesOutputV3` and `from_v2` (or wire the intended caller) - [cli_output.rs:241, cli_output.rs:276]
  evidence: census `StatusServicesOutputV3|from_v2` over packages/, nixos-modules/, tests/, docs/reference/, labs = 1 hit (the definition itself); not in the generated v2 wire-protocol.json (xtask WireProtocolSchema imports only AuditOutputV2/AuthStatusOutputV2/ListOutputV2/OpInspectOutputV1/StatusOutputV2/UsbProbeOutputV1, xtask/src/main.rs:19-22).
- d2b-contracts-control#5 sev=low blast=leaf effort=S verdict=actionable - `pub use d2b_contracts::audio::LevelPercent;` in cli_output.rs re-exports a type neither this module nor any external caller uses (public_wire.rs imports LevelPercent from d2b_contracts directly) - fix: delete the re-export - [cli_output.rs:6]
  evidence: census `cli_output::LevelPercent` over packages/, nixos-modules/, tests/, docs/reference/, labs = 0 hits; in-crate use is only the re-export line itself.
- d2b-contracts-control#6 sev=low blast=wide effort=S verdict=needs-contract - `AuditEntry` (public_wire.rs:2677) is exported but referenced by no wire type in the crate - `AuditResponse` uses `AuditExportEntry` from d2b_contracts - and survives only as a historical schema artifact - fix: remove the struct after confirming docs/reference/schemas/v1/wire-protocol.json:127 no longer needs the definition - [public_wire.rs:2677, docs/reference/schemas/v1/wire-protocol.json:127]
  evidence: census `AuditEntry` over packages/ = definition plus an unrelated distinct type in d2b-broker/src/audit.rs:124; the only doc pin is the v1 wire-protocol.json definition, so needs-contract.
- d2b-contracts-control#7 sev=low blast=leaf effort=S verdict=actionable - `HelperSnapshot::validate` and `HelperLaunchRequest::validate_bounds` are `pub` but every caller is an in-crate `Deserialize` impl; external consumers call `validate_unsafe_local_resource_identity` directly instead - fix: make both methods private (or `pub(crate)`) - [unsafe_local_wire.rs:105, unsafe_local_wire.rs:171]
  evidence: census `\.validate_bounds\(|snapshot\.validate\(` over packages/ = in-crate calls only (unsafe_local_wire.rs:136, unsafe_local_wire.rs:206); external `validate()` hits are other crates' distinct types.

## err
- d2b-contracts-control#8 sev=low blast=leaf effort=S verdict=actionable - `ShellNameError` is a public error type with no `Display` or `std::error::Error` impl, so callers cannot format it or chain it with `?` - fix: add `Display` + `std::error::Error` impls (additive; the type is documented as an empty struct in daemon-api.md:653) - [public_wire.rs:1297]
  evidence: seed `enum \w*Error` = 92 hits; `ShellNameError` is the only error type in the crate without Display/Error; all `\.unwrap\(\)|\.expect\(` hits (92) sit in `#[cfg(test)]` mods or tests/ and `panic!` hits are test assertions, so panic policy is otherwise clean.
- clean: seeds `\.unwrap\(\)|\.expect\(` / `let _ = |\.ok\(\);` / `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` / `enum \w*Error` ran at 92 hits; every panic site is in `#[cfg(test)]` or tests/, wire error vocabularies (NamedProcessStreamErrorKind, AudioErrorKind, HelperFailureCode) are closed kebab-case enums, and the sole `let _ =` is a test line.

## serde
- d2b-contracts-control#9 sev=medium blast=leaf effort=M verdict=actionable - three hand-written `Deserialize` impls plus private `*Wire` shadow structs (HelperSnapshot, HelperLaunchRequest, AuditResponse) re-implement exactly what `#[serde(try_from = "...")]` generates: deserialize raw, validate, map failure to a deserialization error - fix: derive `Deserialize` via `#[serde(try_from = "HelperSnapshotWire")]` (and the two siblings), keeping `deny_unknown_fields` on the wire structs and deleting the manual impls - [unsafe_local_wire.rs:118, unsafe_local_wire.rs:176, public_wire.rs:2228]
  evidence: seed `impl .*Deserialize.*for` = 5 hits (the three Wire-struct pairs plus the two single-field newtypes ShellName/RealmAccentColor, whose custom error strings are fine to keep); same admission semantics, no wire change; the refusal-ledger class covers qemu guest/provider shapes only, not these sites.
- clean: seeds `derive\([^)]*(De)?[Ss]erialize` / `serde\()...)` / `impl .*Deserialize.*for` / `serde_json::from_|serde_json::to_` ran at 660 hits; rename_all/deny_unknown_fields/skip_serializing_if discipline is consistent, `#[serde(other)]` Unknown fallbacks on probe-state enums are the right forward-compat choice, and untagged enums (StatusOutputV2, ApiReadyStatusV1) are output-only.

## obs
- N/A: seeds `\bprintln!\(|\beprintln!\(` / `(info|debug|warn|error|trace)!\("` / `\.instrument\(|#\[instrument` / `tracing::|log::` all 0 hits and the manifest (packages/d2b-contracts-control/Cargo.toml) declares no tracing/log dependency; pure DTO crate with no telemetry surface.

## docs
- d2b-contracts-control#10 sev=medium blast=leaf effort=M verdict=actionable - cli_output.rs exports 20+ CLI-output DTOs (ListOutputV2, ListItemOutputV2, UsbProbeOutputV1, RealmListOutputV1, RealmInspectOutputV1, OpInspect*, RealmPolicyOutputV1, StatusOutputV2, StatusInventoryOutputV2, ApiReady*, StatusVmOutputV2, LivePoolIntegrityOutputV1, StatusServicesOutputV2, RunnerParityOutputV2, StatusBridgeCheckOutputV2, Audit*OutputV2, Auth*OutputV2) with no doc comments; only StatusServicesOutputV3 and two fields document anything - fix: add one-line doc comments naming the wire shape each DTO renders - [cli_output.rs:10, cli_output.rs:14, cli_output.rs:49, cli_output.rs:130, cli_output.rs:168, cli_output.rs:222]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 244 hits; zero `/// # (Examples|Errors|Panics|Safety)` sections anywhere in the crate.
- d2b-contracts-control#11 sev=medium blast=leaf effort=M verdict=actionable - public_wire.rs request/response structs and fields are undocumented where the wire semantics are non-obvious (ListRequest, StatusRequest, AuditRequest, AuditSelector, ListEntry, VmStatus, PublicVmServices, BridgeCheck, VmLifecycle, RuntimeSummary, VmAutostartPosture, QemuMedia*, ShellName, ShellNameError, WorkloadListArgs, UsbipProbeEntry field meanings), and `-> Result<` items (ShellName::new, RealmAccentColor::new) carry no `# Errors` section - fix: add doc comments with `# Errors` on the Result-returning constructors - [public_wire.rs:278, public_wire.rs:293, public_wire.rs:2451, public_wire.rs:2495, public_wire.rs:2588, public_wire.rs:2633, public_wire.rs:1279, public_wire.rs:1282]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 244 hits and seed `-> Result<` = 57 hits; no canonical doc sections exist in the crate.
- d2b-contracts-control#12 sev=medium blast=leaf effort=M verdict=actionable - unsafe_local_wire.rs exposes undocumented pub constants with unexplained magic values (MAX_HELPER_QUEUE_DEPTH=128, MAX_HELPER_SNAPSHOT_SCOPES=1024, MAX_COMPLETED_OPERATIONS_PER_UID=1024, MAX_COMPLETED_OPERATION_AGE_SECS=24*60*60, UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION), undocumented pub fns (unsafe_local_helper_protocol_supported, validate_unsafe_local_resource_identity, HelperSnapshot::validate, HelperLaunchRequest::validate_bounds), and undocumented wire types (HelperHello, HelperHelloAccepted, HelperHeartbeat, HelperScopeKind, HelperScopeState, HelperScopeSnapshot, HelperSnapshot, HelperOperationResult, HelperOperationRejected, DaemonToUnsafeLocalHelper, UnsafeLocalHelperToDaemon, UnsafeLocalHelperWireSchema) - fix: document each constant with the why (queue/snapshot/age bounds the daemon enforces) and one line per wire type - [unsafe_local_wire.rs:15, unsafe_local_wire.rs:21, unsafe_local_wire.rs:24, unsafe_local_wire.rs:26, unsafe_local_wire.rs:250, unsafe_local_wire.rs:32, unsafe_local_wire.rs:308]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 244 hits; the constants are consumed by d2bd-runtime and d2b-unsafe-local-helper (census over packages/), so their bounds are cross-crate contracts.
- d2b-contracts-control#13 sev=low blast=leaf effort=S verdict=actionable - terminal_wire.rs's seven DTOs (TerminalStream, TerminalSize, TerminalWriteStdin, TerminalReadOutput, TerminalResize, TerminalWriteStdinResult, TerminalReadOutputChunk) have no item docs; only the module-level `//!` explains them - fix: add one-line docs per type (the redacted-Debug note belongs on the session-bearing types) - [terminal_wire.rs:12, terminal_wire.rs:19, terminal_wire.rs:26, terminal_wire.rs:105]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 244 hits; terminal_wire.rs is the only module whose types are entirely undocumented.

## perf
- clean: seeds `format!\(` / `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` / `\.to_string\(\)` ran at 57 hits, every one in `#[cfg(test)]` redaction assertions or cold paths (BTreeMap::new in the from_v2 conversion shim, to_owned in the JsonSchema impl); the crate is DTO definitions with no hot loop, so all sites are `static (unmeasured)` and non-issues.

## conc
- N/A: seeds `std::thread::|thread::spawn|thread::scope` / `\bMutex<|\bRwLock<` / `Atomic\w+|Ordering::` / `thread_local!|unsafe impl (Send|Sync) for` all 0 hits; pure data-definition crate with no threads, locks, or atomics.

## async
- N/A: seeds `async fn|async move|\.await` / `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` / `tokio::sync::(Mutex|RwLock|Notify)` / `#\[tokio::(main|test)\]|Runtime::block_on` all 0 hits and the manifest has no tokio dependency.

## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` / `// SAFETY:` / `transmute|from_raw|MaybeUninit|mem::zeroed` all 0 hits; the manifest inherits `[workspace.lints]` (`unsafe_code = "forbid"`, Cargo.toml root) and seed 4 alone does not make the lens applicable.

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` / `catch_unwind` / `repr\(C\)|repr\(transparent\)` / `CStr|CString|c_char` all 0 hits (the only textual matches are `cStr` inside the identifier `ExecStream`, a case-insensitive false positive); no FFI surface in this crate.

## macro
- N/A: seeds `macro_rules!` / `proc_macro|syn::|quote!` / `\$crate` / `to_compile_error|new_spanned` all 0 hits; no macros defined or used beyond std derives.

## test
- d2b-contracts-control#14 sev=medium blast=leaf effort=M verdict=actionable - the `WorkloadOp`/`WorkloadOpResponse` wire family (feature-negotiated v3 operations, dispatched by d2bd/src/composition.rs:7666) has no round-trip or shape test in this crate, unlike every sibling family (exec, console, audio, shell, named streams, audit all have wire-shape tests) - fix: add a round-trip + tag/rename pin test for WorkloadOp::List/Status/LauncherExec and WorkloadOpResponse, mirroring `audio_public_wire_json_shape_is_stable` - [public_wire.rs:167, public_wire.rs:175]
  evidence: seed `#\[test\]|#\[tokio::test\]` = 35 tests and `assert_eq!\(|assert_ne!\(|assert!\(` = 134 asserts in src+tests; none reference WorkloadOp (census over the crate's tests), so the contract behavior has no test.
- clean: seeds `#\[test\]|#\[tokio::test\]` / `assert_eq!\(|assert_ne!\(|assert!\(` / `proptest!|insta::assert|rstest` / `#\[ignore\]` ran at 35 tests / 134 asserts / 0 / 0; the suite is table-driven with failure messages (shell_name_enforces_adr_shape), pins wire shapes deliberately, and asserts fail-closed behavior (unknown fields, invalid audit pages, redaction sentinels); no ignored or tautological tests found.

## Coverage
- idiom: clean (seeds ran: 0/0/0; hand-written Debug impls are deliberate redaction)
- own: clean (seeds ran: 94 hits; all clones/to_owned explainable wire-building or test fixtures)
- type: 3 finding(s)
- api: 4 finding(s)
- err: 1 finding(s)
- serde: 1 finding(s)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency in manifest)
- docs: 4 finding(s)
- perf: clean (seeds ran: 57 hits; all cold/test sites)
- conc: N/A (seeds: 0/0/0/0 all zero; no threads/locks/atomics)
- async: N/A (seeds: 0/0/0/0 all zero; no async fn or tokio dependency)
- unsafe: N/A (seeds: 0/0/0 all zero for seeds 1-3; manifest inherits workspace `unsafe_code = "forbid"`)
- ffi: N/A (seeds: 0/0/0/0 all zero; no FFI surface)
- macro: N/A (seeds: 0/0/0/0 all zero; no macros defined)
- test: 1 finding(s)