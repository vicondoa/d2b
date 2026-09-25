# d2b-p1 - d2b - part 1/3
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 6520 (excl. src/generated/**) | modules: context, activation, zone_support_bundle, share, guest, zone, host_generation, runtime, main
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: src/context.rs, src/activation.rs, src/zone_support_bundle.rs, src/share.rs, src/guest.rs, src/zone.rs, src/host_generation.rs, src/runtime.rs, src/main.rs

## idiom
- clean: seeds ran: 0 index loops / 0 hand-written impls / 4 statement-accumulations; the 4 hits (context.rs:2708, context.rs:2727, context.rs:2738, activation.rs:246) are bounded reads and a byte-budget string fold where an iterator pipeline would obscure the early-exit condition; no derive-replaceable impls and no index loops exist.

## own
- clean: seeds ran: 77 clones / 121 to_owned-to_vec-to_string / 1 Rc-RefCell-Arc-Mutex / 0 Cow; every clone inspected is explainable (owned constructor parameters such as CliZoneConnector::new and ProcessAttachTarget::shell_session, post-move reuse of resource_ref and guest_ref, serde_json::from_value ownership, clap arg fields); the single RefCell is the cfg(test) TEST_STAGING_BASE thread_local (activation.rs:114-117), a test fixture.

## type
- d2b-p1#1 sev=low blast=leaf effort=M verdict=actionable - ZoneContext stores the validated Zone name as a bare String and re-validates it at every construction site instead of carrying the invariant in the already-imported ZoneId type - fix: store `ZoneId` in ZoneContext (field at context.rs:713), build `zone_ref()` and `zone_name()` from it, delete `validate_zone_name` (context.rs:2751) and the duplicated double validation in `discover` (context.rs:752 and context.rs:763), and replace the `expect` re-parses in `from_socket` (context.rs:801-803) with direct construction - [context.rs:713, context.rs:2751, context.rs:801]
  evidence: seeds `fn validate_\w+|fn check_\w+` = 5 hits, `is_\w+: bool|\w+_flag: bool` = 2, `(mode|kind|state): String` = 3; the other 4 validate hits and all bool/String hits are wire-mirror types (ManifestVm.is_net_vm, ManifestRuntime.kind, BridgeHealthFixture.state, DaemonErrorEnvelope.kind) or boundary admission checks on untrusted daemon JSON (validate_response, validate_share_spec, validate_share_type_filter, validate_operation), which the card exempts.

## api
- d2b-p1#2 sev=medium blast=leaf effort=S verdict=actionable - `pub mod host_generation` (lib.rs:25) is the crate's only public module and its three exported items have zero consumers anywhere in the workspace - fix: make it `mod host_generation` (private) until a caller exists, or wire `build_request` into the host-generation CLI flow that currently does not call it - [packages/d2b/src/host_generation.rs:7, packages/d2b/src/host_generation.rs:17, packages/d2b/src/host_generation.rs:36]
  evidence: seed `\bpub (fn|struct|enum|trait|type|const|mod) ` = 14 hits; census: `HostGenerationRequest|build_request` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel = 1 hit (the defining file itself); the error-code strings host-generation-target-invalid and host-generation-artifact-invalid appear in no docs/reference/error-codes.md or cli-contract.md entry, so no wire pin exists.
- d2b-p1#3 sev=low blast=leaf effort=S verdict=actionable - zone_support_bundle.rs declares 9 `pub` items (6 structs, 3 consts, build_bundle, render_ndjson) inside a private module, a pub-in-private surface no external caller can reach - fix: reduce to `pub(crate)` or plain items, keeping only what the in-file tests and `run` need - [zone_support_bundle.rs:19, zone_support_bundle.rs:28, zone_support_bundle.rs:99, zone_support_bundle.rs:323, zone_support_bundle.rs:395]
  evidence: seed `\bpub (fn|struct|enum|trait|type|const|mod) ` = 14 hits; census: `build_bundle|render_ndjson|ResourceStatusSnapshot` over packages/ and tests/ = 0 hits outside the defining file; the d2b integration test drives the CLI binary (tests/zone_support_bundle_contract.rs), not the library surface.

## err
- clean: seeds ran: 84 unwrap-expect / 13 let-underscore-or-ok / 10 panic-unreachable-todo-unimplemented / 2 error enums; the only non-test unwrap/expect sites are invariant panics with named reasons (context.rs:529 fixed-width prefix conversion after an explicit length check, context.rs:801-803 validated-name construction, runtime.rs:28 startup precondition) which the card exempts; every `let _ =` site is deliberate best-effort teardown (socket shutdown, stream cancel/close on signal paths, temp-file cleanup); the 3 unreachable! sites (share.rs:203, share.rs:233, guest.rs:277) sit on closed local constructions; HostGenerationRequestError is a closed two-variant enum with Display, fine for a CLI-internal type.

## serde
- clean: seeds ran: 16 derives / 18 serde attributes / 0 hand-written Deserialize / 52 serde_json calls; all types are wire mirrors with deliberate rename_all camelCase, deny_unknown_fields on operator-facing fixtures, and `default` on support-bundle projections; the flatten on ManifestDocument.entries (context.rs:137) is a deliberate schema mirror consuming unknown top-level keys; no try_from gap, no untagged enum, no hand-written admission gate.

## obs
- clean: seeds ran: 2 println-eprintln / 0 interpolated event macros / 0 instrument / 7 tracing-log; the 2 eprintln sites (activation.rs:234, activation.rs:258) are CLI user-facing pending-config notes, product output the card exempts; the 7 tracing/log hits are substring false positives (`surface_catalog::` contains "log::") in guest.rs and host_generation.rs; the crate emits no telemetry from this partition.

## docs
- d2b-p1#4 sev=low blast=leaf effort=S verdict=actionable - two Result-returning public functions lack the canonical `# Errors` section naming their failure conditions - fix: add `# Errors` to `build_request` (TargetInvalid vs ArtifactInvalid) at host_generation.rs:17 and to `render_ndjson` (serialization failure only) at zone_support_bundle.rs:395 - [host_generation.rs:17, zone_support_bundle.rs:395]
  evidence: seeds `^\s*pub (fn|struct|enum|trait|const|type)` = 14, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 106; all 14 public items carry first-sentence doc comments and all modules carry `//!` docs, so the only gap is the missing Errors sections on the 2 Result-returning pub fns.

## perf
- d2b-p1#5 sev=medium blast=leaf effort=M verdict=actionable - every received frame allocates and zeroes a fresh 1 MiB buffer and then copies the payload again, on the interactive shell path where the daemon answers each 50 ms poll round trip - fix: keep a reusable receive buffer (e.g. a Vec<u8> field on CliSocket reused across read_frame calls, or a thread-local scratch) so the zeroed 1 MiB allocation happens once, and return the truncated buffer instead of `frame[FRAME_PREFIX_BYTES..].to_vec()` - [context.rs:570, context.rs:538]
  evidence: static (unmeasured); seeds `format!\(` = 67, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 11, `\.to_string\(\)` = 12; the remaining format!/Vec::new hits are cold error paths, bounded stdin reads, and output rendering, which the card exempts.

## conc
- clean: seeds ran: 6 std-thread / 1 Mutex / 29 atomics / 1 thread_local; the atomics in CliAttachStream and call_options use paired Acquire/Release/AcqRel handoffs and a Relaxed counter, all correct for their shape; the Mutex is the cfg(test) MockClient recorder and the thread_local is the cfg(test) staging override; no manual Send/Sync impls exist.

## async
- clean: seeds ran: 67 async fn-await / 0 spawn / 1 tokio sync / 0 tokio main; the transport is readiness-driven throughout (AsyncFd, non-blocking seqpacket syscalls, bounded retry loops); the two tokio::sync::Mutex guards (round_trip_guard, stdin_offset) legitimately span awaits; the Drop-time block_on in CliAttachStream::drop is guarded by inside_runtime(); the select! loop documents its cancellation safety; all disallowed-method sites carry the sanctioned "CLI-only path" reason.

## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0; seed 4 alone (workspace `unsafe_code = "forbid"` inherited via `[lints] workspace = true`, Cargo.toml:8-9) does not make the lens applicable.

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0; no foreign-caller boundary exists in this partition.

## macro
- N/A: seeds `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0; no macro definitions in this partition.

## test
- clean: seeds ran: 32 test attributes / 119 assertions / 0 proptest-insta-rstest / 0 ignore; 32 tests across the six in-file test modules (context.rs, activation.rs, zone_support_bundle.rs, share.rs, guest.rs, zone.rs) all assert observable behavior (frame formats, bounded deadlines, redaction, envelope fields) with human-written expectations; no test is structurally unable to fail; the crate-level tests/ directory is outside this partition and was not audited here.

## Coverage
- idiom: clean (seeds ran: 0/0/4)
- own: clean (seeds ran: 77/121/1/0)
- type: 1 finding(s)
- api: 2 finding(s)
- err: clean (seeds ran: 84/13/10/2)
- serde: clean (seeds ran: 16/18/0/52)
- obs: clean (seeds ran: 2/0/0/7)
- docs: 1 finding(s)
- perf: 1 finding(s)
- conc: clean (seeds ran: 6/1/29/1)
- async: clean (seeds ran: 67/0/1/0)
- unsafe: N/A (seeds: 0/0/0 all zero; no unsafe blocks, fns, or SAFETY comments; workspace forbid alone does not apply)
- ffi: N/A (seeds: 0/0/0/0 all zero; no extern boundary)
- macro: N/A (seeds: 0/0/0/0 all zero; no macro definitions)
- test: clean (seeds ran: 32/119/0/0)