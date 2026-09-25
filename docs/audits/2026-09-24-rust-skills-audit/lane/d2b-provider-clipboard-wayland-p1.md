# d2b-provider-clipboard-wayland-p1 - d2b-provider-clipboard-wayland - part 1/2
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 6771 (excl. src/generated/**) | modules: bin/d2b-clipd.rs, fd.rs, runtime.rs, audit.rs, policy.rs, lib.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: src/bin/**, src/fd.rs, src/runtime.rs, src/audit.rs, src/policy.rs, src/lib.rs

## idiom
- d2b-provider-clipboard-wayland-p1#1 sev=medium blast=leaf effort=S verdict=actionable - d2b-clipd hand-rolls CLI flag parsing with a manual loop while every other binary in the repo uses clap derive - fix: replace parse_args with a clap::Parser derive on Args (clap is the house pattern in d2bd/src/main.rs, d2b/src/dispatch.rs, d2b-provider-display-wayland/src/bin/d2b-wayland-proxy.rs), which also fixes --help exiting 2 via Err - [src/bin/d2b-clipd.rs:3815, src/bin/d2b-clipd.rs:127]
  evidence: census: clap::{Parser,Args} over packages = 3+ binaries (d2bd/src/main.rs:3, d2b/src/dispatch.rs:21, d2b-provider-display-wayland/src/bin/d2b-wayland-proxy.rs:23); seed `let mut \w+ = (String|Vec)::new\(\)` 12 hits, all legitimate byte/bounded loops
- d2b-provider-clipboard-wayland-p1#2 sev=low blast=leaf effort=S verdict=actionable - should_suppress_published_selection_echo takes two unused parameters and delegates to an identity wrapper that returns its argument unchanged - fix: return selection.suppress_selection_echo directly, drop the _window and _bridge_selection parameters and the two arguments at the call site, delete should_suppress_published_selection_echo_state - [src/bin/d2b-clipd.rs:3776, src/bin/d2b-clipd.rs:3787, src/bin/d2b-clipd.rs:2113]
  evidence: static: wrapper body is `suppress_selection_echo` returned verbatim; census: both functions in-file only (def 3776/3787, call 2113, test 4022) = 4 hits over packages/
- d2b-provider-clipboard-wayland-p1#3 sev=low blast=leaf effort=S verdict=actionable - install_bridge_listeners builds a Vec with a push loop where the body is a pure Result-producing map - fix: collect the iterator: bridge_peers.into_iter().map(|peer| { ... Ok(BridgeListener { ... }) }).collect::<Result<Vec<_>, String>>()? - [src/bin/d2b-clipd.rs:1131]
  evidence: seed `let mut \w+ = (String|Vec)::new\(\)` 12 hits; site matches the statement-accumulation shape the seed names
- d2b-provider-clipboard-wayland-p1#14 sev=medium blast=leaf effort=S verdict=actionable - preferred_mime_order hardcodes the four MIME strings that policy.rs ALLOWED_MIME_TYPES already owns, keeping the reported MIME-policy triplication alive (row S79, reported not consolidated) - fix: iterate d2b_provider_clipboard_wayland::ALLOWED_MIME_TYPES in preferred_mime_order instead of the literal list, so allowlist changes propagate to the preference order - [src/bin/d2b-clipd.rs:2812, src/policy.rs:12]
  evidence: census: the four MIME literals appear at policy.rs:12-17 (ALLOWED_MIME_TYPES), d2b-clipd.rs:2812-2816 (preferred_mime_order), and clipd_host policy (part 2 scope); row S79 at docs/explanation/over-engineering-audit-record.md:394 records the triplication as reported-not-consolidated, and the site still matches
- clean: seeds ran: `for \w+ in 0\.\.` 3 (bounded drain and tests), `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` 1 (Policy Default preserves constructor invariants, deliberate), `let mut \w+ = (String|Vec)::new\(\)` 12 (byte reads and bounded loops); no other hand-written impls, naming drift, or conversion smells found

## own
- clean: seeds ran: `\.clone\(\)` 75, `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` 165, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` 0, `Cow<` 0; every clone inspected is explainable (Arc clone inside FdPermitPool::acquire for drop-side release, mpsc Sender clones at thread spawn, candidate construction clones, history-retention clones at d2b-clipd.rs:757 and 1043); no borrow-checker-fighting clones or argument-position copies found

## type
- d2b-provider-clipboard-wayland-p1#4 sev=medium blast=leaf effort=S verdict=actionable - the config-sourced picker path bypasses the absolute-path validation that the --picker CLI flag enforces, because the check runs on args.picker before the merge with the config value - fix: validate the merged picker value (args.picker.clone().or(picker_from_config)) once after the merge, rejecting relative paths from either source - [src/bin/d2b-clipd.rs:140, src/bin/d2b-clipd.rs:142]
  evidence: seed `fn validate_\w+|fn check_\w+` 7 hits, all boundary gates (validate_fd_metadata, validate_bridge_peer); static: the `!p.is_absolute()` check at 142-146 tests only args.picker, while the config-sourced path flows into PickerCommand::program unvalidated at 197-200
- clean: seeds ran: `fn validate_\w+|fn check_\w+` 7 (all single-boundary admission gates on kernel or wire input, the correct place per the card), `is_\w+: bool|\w+_flag: bool` 0, `(mode|kind|state): String` 0; the five Policy booleans are independent toggles with no illegal combination, and FdMetadata is constructed only by inspect_fd, so no flag soup or validate-at-every-callsite found

## api
- clean: seeds ran: `\bpub (fn|struct|enum|trait|type|const|mod) ` 99, `pub .*\b(Arc|Rc|Box|RefCell)<` 0, `^\s*pub use ` 8 (lib.rs re-export arms are the house single-surface pattern); crate root denies missing_docs and forbids unsafe_code; no dependency types or internals in public signatures, no dual-path items, ClipdHost is the crate's own re-exported type

## err
- d2b-provider-clipboard-wayland-p1#5 sev=low blast=leaf effort=S verdict=actionable - spawn_niri_event_thread panics with .expect("niri thread spawn") on thread-spawn failure while the four sibling spawn sites log the error and continue - fix: return Result from spawn_niri_event_thread and log at the call site like the bridge-copy-read, paste-replay, host-copy-read, and published-write spawners - [src/bin/d2b-clipd.rs:3550, src/bin/d2b-clipd.rs:1754, src/bin/d2b-clipd.rs:2863]
  evidence: seed `\.unwrap\(\)|\.expect\(` 78 hits; 77 are inside #[cfg(test)] modules or startup preconditions (card false positives), the single non-test site is 3550; sibling spawn sites log::error! on failure
- d2b-provider-clipboard-wayland-p1#6 sev=low blast=leaf effort=M verdict=actionable - the binary propagates errors as Result<_, String> with format!-built messages at 13 signatures, where the skill names anyhow for binaries - fix: introduce anyhow at the binary top level (run and its helpers), keeping the lib error enums unchanged - [src/bin/d2b-clipd.rs:127, src/bin/d2b-clipd.rs:411, src/bin/d2b-clipd.rs:247]
  evidence: seed `enum \w*Error` 6 hits (lib enums ClipboardRuntimeError, FdSafetyError, FdReadError, ClipboardPolicyError are well-shaped); static: 13 `Result<..., String>` signatures in the binary; no caller string-matches these errors today, hence low
- clean: seeds ran: `\.unwrap\(\)|\.expect\(` 78 (77 test/startup, 1 finding above), `let _ = |\.ok\(\);` 32 (best-effort cleanups: cancel_picker, cancel_active, tx.send, remove_file; judged per site), `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` 4 (all in tests), `enum \w*Error` 6 (lib enums split by caller action, Display is the pinned wire code)

## serde
- d2b-provider-clipboard-wayland-p1#7 sev=medium blast=leaf effort=M verdict=actionable - the daemon config is parsed as serde_json::Value and read through hand-rolled .pointer() lookups with per-field error strings, instead of a typed Deserialize struct that validates at the boundary - fix: define a typed ClipdConfig with #[serde(deny_unknown_fields)] (picker.executable, runtime.bridgeEndpoints with try_from for WorkloadTarget::parse) and deserialize once in run(); the JSON shape is unchanged, so the Nix producer keeps working - [src/bin/d2b-clipd.rs:132, src/bin/d2b-clipd.rs:247]
  evidence: seed `serde_json::from_|serde_json::to_` 8 hits; census: config shape produced by nixos-modules nix/site.nix:69-136 (bridgeEndpoints, picker.executable) and pinned by tests at d2b-clipd.rs:4346; the pointer plumbing spans 70+ lines (247-316) that a derive replaces
- clean: seeds ran: `derive\([^)]*(De)?[Ss]erialize` 3, `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` 3, `impl .*Deserialize.*for` 0, `serde_json::from_|serde_json::to_` 8; BridgeFrame, BridgeAttribution, and ControlFrame are tagged enums with deny_unknown_fields and rename_all, and parse_bridge_frame validates identity and attribution after the parse (the correct admission-gate shape); no hand-written deserializers

## obs
- d2b-provider-clipboard-wayland-p1#8 sev=medium blast=leaf effort=M verdict=actionable - the binary logs through the log facade with interpolated message strings (62 sites) while the crate's lib uses tracing with named fields, giving one crate two facades and unqueryable events - fix: migrate d2b-clipd.rs to tracing (already a dependency, used by runtime.rs) with named fields, e.g. log::info!("d2b-clipd: ready (config={}, ...)") becomes tracing::info!(config = %args.config.display(), bridge_root = %args.bridge_root.display(), "d2b-clipd ready") - [src/bin/d2b-clipd.rs:206, src/bin/d2b-clipd.rs:1627, src/runtime.rs:100]
  evidence: seed `(info|debug|warn|error|trace)!\("` 40 hits (35 in the binary, 5 in runtime.rs); `tracing::|log::` 76 hits split 62 log:: in the binary vs 14 tracing:: in runtime.rs; `\.instrument\(|#\[instrument` 0, so no span carries the context the interpolated messages duplicate
- clean: seeds ran: `\bprintln!\(|\beprintln!\(` 2 (main's user-facing error path and check-config output, product output per the card), `(info|debug|warn|error|trace)!\("` 40 (runtime.rs events carry named fields, e.g. error = %e; the binary's 35 are the finding above), `\.instrument\(|#\[instrument` 0, `tracing::|log::` 76; no secret or clipboard payload is logged (module doc at d2b-clipd.rs:7 and redaction tests confirm), and AcceptDiagnostics::warn builds messages lazily in a closure

## docs
- d2b-provider-clipboard-wayland-p1#9 sev=medium blast=leaf effort=M verdict=actionable - Result-returning public items carry no # Errors sections anywhere in the crate despite deny(missing_docs), so failure contracts (ConcurrentLimitExceeded, InvalidBounds, AuditQueueFull, SessionUnauthenticated) are undocumented - fix: add # Errors sections naming the variants to the public Result-returning items, starting with FdPermitPool::acquire, Policy::new, ClipboardAuditQueue::push, and ClipboardRuntime::admit_route - [src/fd.rs:549, src/policy.rs:81, src/audit.rs:200, src/runtime.rs:97]
  evidence: seed `-> Result<` 69 hits (31 bin, 13 fd, 19 runtime, 4 audit, 2 policy); `/// # (Examples|Errors|Panics|Safety)` 0 hits; first sentences are otherwise one-line and contract-shaped
- d2b-provider-clipboard-wayland-p1#10 sev=medium blast=family effort=S verdict=actionable - ClipboardAuditEvent::to_wire derives wire labels from Debug impls (format!("{:?}", event_type) lowercased and size_bucket via {:?}) instead of stable as_str labels, so a variant rename silently changes the cross-crate audit record consumed by d2bd - fix: add as_str() to ClipboardEventType and SizeBucket returning the exact current renderings ("pasteauthorized", "Lt1K", ...) and use them in to_wire - [src/audit.rs:172, src/audit.rs:174]
  evidence: seed `format!\(` 77 hits; static: event_type and size_bucket render via Debug at audit.rs:174-178 while reason uses ClipboardReason::as_str; census: the wire record is consumed by d2bd/src/interaction_composition.rs:4701 and asserted in tests/provider_behavior.rs:88 and tests/redaction.rs:23
- clean: seeds ran: `^\s*pub (fn|struct|enum|trait|const|type)` 99 (all documented, missing_docs denied at lib.rs:3), `/// # (Examples|Errors|Panics|Safety)` 0 (finding above), `-> Result<` 69 (finding above); module docs present in all five modules; no doctests marked ignore

## perf
- d2b-provider-clipboard-wayland-p1#11 sev=low blast=leaf effort=M verdict=actionable - clipboard payload maps (up to MATERIALIZE_MAX_BYTES = 8 MiB) are cloned wholesale on the host-selection record, history materialization, and bridge-copy publish paths, copying every byte per paste - fix: hold payloads as Arc<BTreeMap<String, Vec<u8>>> (or Arc<[u8]> per MIME) in ClipboardHistoryEntry, BridgeSelectionState, and PublishedSelectionState so materialization and publish become refcount bumps; the history-retention clones at 757 and 1043 disappear - [src/bin/d2b-clipd.rs:757, src/bin/d2b-clipd.rs:1043, src/bin/d2b-clipd.rs:1833, src/bin/d2b-clipd.rs:1838, src/bin/d2b-clipd.rs:2797]
  evidence: static (unmeasured); seed `\.clone\(\)` 75 hits; the six full-map clones (757, 1043, 1833, 1838, 2797, 2803) copy the entire payload, bounded at 8 MiB per item by policy.rs:114
- clean: seeds ran: `format!\(` 77 (error paths, one-shot diagnostics, and wire rendering, all cold), `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` 49 (bounded reads and empty-case collections), `\.to_string\(\)` 14 (boundary copies and Display-free labels); no format! in a loop over payload data, no attacker-controlled hashing, no collection-choice problems found

## conc
- d2b-provider-clipboard-wayland-p1#12 sev=low blast=leaf effort=S verdict=actionable - the two permit counters use Acquire/AcqRel orderings where Relaxed is the weakest correct ordering, since neither counter publishes any data (the permit and descriptor ownership move by value) - fix: switch FdPermitPool.active and HELPER_THREADS to Ordering::Relaxed for load, CAS, and fetch_sub - [src/fd.rs:545, src/fd.rs:600, src/bin/d2b-clipd.rs:74, src/bin/d2b-clipd.rs:96]
  evidence: seed `Atomic\w+|Ordering::` 24 hits (fd.rs 10, bin 14); static: no paired handoff through either counter, so the acquire/release pairs synchronize nothing
- clean: seeds ran: `std::thread::|thread::spawn|thread::scope` 5 (named worker threads with mpsc handoff, the channel model the skill prefers), `\bMutex<|\bRwLock<` 2 (test-only serialization locks with documented reasons), `Atomic\w+|Ordering::` 24 (counters, finding above), `thread_local!|unsafe impl (Send|Sync) for` 0; no shared-state deadlock surface, no static mut

## async
- N/A: seeds `async fn|async move|\.await` 0, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` 0, `tokio::sync::(Mutex|RwLock|Notify)` 0, `#\[tokio::(main|test)\]|Runtime::block_on` 0 over the scope; the binary is deliberately synchronous (poll loop plus worker threads, documented at d2b-clipd.rs:120-122) and the lib has no async fn, so the lens criteria fail

## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` 0, `// SAFETY:` 0, `transmute|from_raw|MaybeUninit|mem::zeroed` 6 raw matches all false positives (from_raw_os_error at d2b-clipd.rs:2534 and FileType::from_raw_mode at fd.rs:242), `unsafe_code` 1 (the `#![forbid(unsafe_code)]` attribute at lib.rs:4, which per the card does not make the lens applicable); no unsafe blocks or unsafe_code allow manifests in the scope

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` 0, `catch_unwind` 0, `repr\(C\)|repr\(transparent\)` 0, `CStr|CString|c_char` 0 over the scope; the crate crosses no foreign boundary (rustix/nix syscall wrappers stay in their own crates)

## macro
- N/A: seeds `macro_rules!` 0, `proc_macro|syn::|quote!` 0, `\$crate` 0, `to_compile_error|new_spanned` 0 over the scope; the crate defines no macros

## test
- d2b-provider-clipboard-wayland-p1#13 sev=medium blast=leaf effort=S verdict=actionable - published_selection_echo_is_always_suppressed_once asserts the identity wrapper should_suppress_published_selection_echo_state, a forwarding pin that fails only if the wrapper's triviality changes - fix: delete the test together with the wrapper (idiom finding #2); the behavior it gestures at is already covered by bridge_selection_echo_suppression_persists_for_source_vm_or_unknown_focus - [src/bin/d2b-clipd.rs:4021, src/bin/d2b-clipd.rs:3787]
  evidence: seed `#\[test\]|#\[tokio::test\]` 61 hits; `#\[ignore\]` 0; the test body asserts `should_suppress_published_selection_echo_state(true)` and `(false)`, i.e. the wrapper's forwarding, not observable behavior
- clean: seeds ran: `#\[test\]|#\[tokio::test\]` 61, `assert_eq!\(|assert_ne!\(|assert!\(` 139, `proptest!|insta::assert|rstest` 0, `#\[ignore\]` 0; the sampled tests assert observable behavior (frame parsing, fd queue limits, echo suppression, timeout and size-exceeded errors, umask restoration) with human-written expectations; error-code Display assertions in fd.rs:681 pin the wire codes recorded in docs/specs/providers/ADR-046-provider-clipboard-wayland.md:1091-1095, so they are contract pins, not implementation pins; tests/ is outside this part's partition (part 2 covers the remaining src modules)

## Coverage
- idiom: 4 finding(s)
- own: clean (seeds ran: 75/165/0/0)
- type: 1 finding(s)
- api: clean (seeds ran: 99/0/8)
- err: 2 finding(s)
- serde: 1 finding(s)
- obs: 1 finding(s)
- docs: 2 finding(s)
- perf: 1 finding(s)
- conc: 1 finding(s)
- async: N/A (seeds: 0/0/0/0 all zero; no async fn, await, spawn, or tokio sync types in scope; binary is a sync poll loop by design)
- unsafe: N/A (seeds: 0/0/6-false-positive/1-forbid-attribute; no unsafe blocks, SAFETY comments, or unsafe_code allow manifests in scope)
- ffi: N/A (seeds: 0/0/0/0 all zero; no foreign boundary)
- macro: N/A (seeds: 0/0/0/0 all zero; no macros defined)
- test: 1 finding(s)