# d2b-p3 - d2b - part 3/3
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 6525 (excl. src/generated/**) | modules: exec_client.rs, dispatch.rs, debug.rs, zone_doctor.rs, exec.rs, endpoint.rs, provider.rs, terminal_client.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: d2b-p3 = src/exec_client.rs, src/dispatch.rs, src/debug.rs, src/zone_doctor.rs, src/exec.rs, src/endpoint.rs, src/provider.rs, src/terminal_client.rs per U1 (f); the test lens additionally reads tests/**

## idiom
- d2b-p3#1 sev=low blast=leaf effort=S verdict=actionable - `summarize` hand-builds a zeroed `DoctorSummary` although the type derives `Default` - fix: `let mut summary = DoctorSummary::default();` - [packages/d2b/src/zone_doctor.rs:598-601]
  evidence: idiom seed 2 (`impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`) = 1 hit; `DoctorSummary` derives `Default` at zone_doctor.rs:122-123
- d2b-p3#2 sev=medium blast=leaf effort=M verdict=actionable - `all_known_subcommands` hand-maintains a 22-entry command-name list of which 13 (launch, realm list/inspect/enter/run, up, down, restart, boot, build, switch, test, rollback, generations, usb, console) are retired v2 verbs the ModernCli parser rejects, so `d2b auth status` reports them as known-but-denied commands - fix: derive the list from `ModernCli::command().get_subcommands()` minus `PROJECTION_COMMANDS` the way `BUILTIN_COMMANDS` does, or drop the retired entries; update the pinned expectation in tests/auth_status_contract.rs - [packages/d2b/src/dispatch.rs:688-706, packages/d2b/src/dispatch.rs:1169-1175]
  evidence: idiom seeds 1/2/3 = 1/1/6; census: the `ModernCommand` variants dispatched at dispatch.rs:794-929 contain none of launch/realm/up/down/restart/boot/build/switch/test/rollback/generations/usb/console, and the parser test `modern_parser_has_no_v2_alias_or_realm_dispatch` asserts those parse as errors; tests/auth_status_contract.rs pins the current allowed list, so the fix updates that test

## own
- d2b-p3#3 sev=low blast=leaf effort=S verdict=actionable - redundant clones of paging values that are dead after the call: `cursor.clone()` before `cursor` is overwritten by `next_cursor`, `page_token.clone()` before reassignment from `nextCursor`, and `reference.to_owned()` on a fresh `format!` String - fix: move `cursor` and `page_token` into the calls (reassign afterwards) and move `reference` into the struct literal - [packages/d2b/src/dispatch.rs:548, packages/d2b/src/debug.rs:472, packages/d2b/src/debug.rs:418]
  evidence: own seeds 1/2/3/4 = 19/109/3/0; each cited value is unused after the call (loop bodies reassign from the response); `AuditExportCursor` is a non-Copy struct (d2b-contracts/src/audit_wire.rs:8-15)
- d2b-p3#4 sev=low blast=leaf effort=S verdict=actionable - `modern_run` clones the entire argv (`raw_args.clone()`) for `try_parse_from` although `raw_args` is consumed by value from the only caller and never used again - fix: `ModernCli::try_parse_from(raw_args)` - [packages/d2b/src/dispatch.rs:977]
  evidence: own seed 1 = 19 hits; census: `dispatch::modern_run(raw_args)` at packages/d2b/src/lib.rs:236 passes ownership and no later use of `raw_args` exists in `modern_run`
- d2b-p3#5 sev=low blast=leaf effort=S verdict=actionable - `host_error_envelope` takes seven `&str` parameters and `.to_owned()`s each into the envelope, while callers pass `&format!(...)` results, allocating twice per field - fix: take `impl Into<String>` parameters so `format!` results move in directly - [packages/d2b/src/dispatch.rs:296-313, packages/d2b/src/dispatch.rs:347, packages/d2b/src/dispatch.rs:359, packages/d2b/src/dispatch.rs:371-377]
  evidence: own seed 2 = 109 hits; callers at dispatch.rs:346-378 (and host.rs:144,367) pass `&format!(...)` into the `&str` parameters

## type
- d2b-p3#6 sev=low blast=leaf effort=S verdict=actionable - closed CLI vocabularies carried as `String` and validated at every call site: `ExecKillArgs.signal` checked by `matches!` in `kill`, and `endpoint_class: Option<String>` checked by `validate_endpoint_class` in `list` and `watch` - fix: clap `ValueEnum` on the args fields so an invalid value is a parse error and the runtime checks disappear - [packages/d2b/src/exec.rs:90, packages/d2b/src/exec.rs:345, packages/d2b/src/endpoint.rs:34, packages/d2b/src/endpoint.rs:42, packages/d2b/src/endpoint.rs:203-216]
  evidence: type seeds 1/2/3 = 6/0/3; both vocabularies are closed five-value sets validated only at CLI entry; the wire value stays a string so no contract change

## api
- clean: seeds 72/0/0 run; every `pub` item in the part sits inside a private module (`mod exec_client;` etc., lib.rs:13-36), so the items are unreachable crate-external surface; the d2b lib's only outside consumer is xtask via `d2b::cli_command()` (census: 6 hits in packages/xtask/src/main.rs); no `Arc`/`Rc`/`Box`/`RefCell` in any public signature (`FdStateGuard`'s `Box<dyn HostTtyOps>` is a private field)

## err
- clean: seeds 33/10/5/0 run; all 33 `unwrap`/`expect` and all 5 `panic!` sit in `#[cfg(test)]`; the 10 `let _ =` sites are deliberate best-effort cleanup or discarding an `Ok` value (`round_trip(&close_op(...))?`, `fcntl_setfl`, `writeln!`, `error.print()`); `ExecClientError` is a documented struct (not an enum) carrying the redaction-safe wire `kind` slug, and `exit_for_kind` owns the exit-code mapping with a tested table

## serde
- clean: seeds 13/11/0/16 run; `HostErrorEnvelope` and `AuditResponseFrame` use `rename_all = "camelCase"` plus `deny_unknown_fields`; zone_doctor projections use type-level `#[serde(default)]`; no hand-written `Deserialize` impls; wire decode failures map to typed `ExecClientError` instead of stringified messages

## obs
- N/A: seeds 0/0/0/0 all zero (the 5 `tracing::|log::` grep hits are `surface_catalog::` substring false positives); d2b has no tracing/log dependency (packages/d2b/Cargo.toml), and CLI output goes through the `print_stdout`/`print_stderr` product-output helpers, not telemetry

## docs
- d2b-p3#7 sev=low blast=leaf effort=S verdict=actionable - six `pub fn` response expecters (`expect_start`, `expect_detached_create/list/logs/status/kill`) carry no doc comment in an otherwise fully documented module - fix: one-line docs stating the expected `ExecOpResponse` variant and the protocol error on mismatch - [packages/d2b/src/exec_client.rs:497, packages/d2b/src/exec_client.rs:507, packages/d2b/src/exec_client.rs:519, packages/d2b/src/exec_client.rs:531, packages/d2b/src/exec_client.rs:543, packages/d2b/src/exec_client.rs:555]
  evidence: docs seeds 1/2/3 = 72/0/66; the six fns are the only undocumented `pub` items in the module (bin crate, so this is a proposal, never the `missing_docs` lint)

## perf
- clean: seeds 54/16/109 run; all `format!` sites are error paths, one-shot CLI rendering, or test fixtures (cold per the card); the FSM's per-op `session.to_owned()` is one small String per socket round trip, not a hot-loop allocation; `pending_stdin` and the capture buffers grow by push with natural capacity reuse

## conc
- clean: seeds 1/1/0/0 run; one dedicated sigwait thread (`d2b-exec-sig`) is the right channel model for signal forwarding; the single `Arc<Mutex<VecDeque<ExecSignal>>>` has genuine two owners (sigwait thread + FSM) with a briefly held guard; the `tokio::sync::Notify` waiter documents its permit semantics; no atomics and no unsafe `Send`/`Sync` impls

## async
- clean: seeds 7/0/2/0 run; `audit_via_socket` is a bounded-budget async fn awaiting only socket send/recv; `InstalledSignals::waiter` uses the documented Notify permit pattern; `block_on` appears only at the CLI entry (`try_audit_via_socket`, dispatch.rs:514-517), a sanctioned process entry point; no guard is held across an `.await`

## unsafe
- N/A: seeds 0/0/3/0 all zero (the 3 `from_raw` hits are `io::Error::from_raw_os_error`, std functions, not unsafe blocks); no `unsafe` block/fn/impl and no `unsafe_code` attribute in the part; the crate inherits `unsafe_code = "forbid"` via `[lints] workspace = true` (packages/d2b/Cargo.toml:8-9)

## ffi
- N/A: seeds 0/0/0/0 all zero

## macro
- N/A: seeds 0/0/0/0 all zero

## test
- d2b-p3#8 sev=medium blast=leaf effort=S verdict=actionable - the `d2b exec wait` guest-exit-code passthrough (`guestExitCode`/`exitCode` lookup, 0-255 filter, `unwrap_or(0)`) has no unit or integration test, so a regression in the CLI exit-code contract would pass silently - fix: extract the extraction into a testable helper or add a mock-daemon integration test asserting the passthrough and the out-of-range fallback - [packages/d2b/src/exec.rs:227-239]
  evidence: test seeds over src+tests = 145/571/0/0; census: `exec.*wait|guestExitCode` over packages/d2b/tests = 0 hits; the exec.rs unit tests cover only attach
- d2b-p3#9 sev=low blast=leaf effort=S verdict=actionable - `validate_env` (KEY=VALUE shape, key length and charset bounds) has no test, unlike the sibling `validate_exec_ref` behavior that the attach tests cover - fix: table-driven unit test with human-written expected outcomes (valid, empty key, over-64 key, non-alnum key, missing `=`) - [packages/d2b/src/exec.rs:383-397]
  evidence: test seeds over src+tests = 145/571/0/0; census: `validate_env` appears only at exec.rs:125 (one call site, no test)

## Coverage
- idiom: 2 finding(s)
- own: 3 finding(s)
- type: 1 finding(s)
- api: clean (seeds ran: 72/0/0)
- err: clean (seeds ran: 33/10/5/0)
- serde: clean (seeds ran: 13/11/0/16)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency)
- docs: 1 finding(s)
- perf: clean (seeds ran: 54/16/109)
- conc: clean (seeds ran: 1/1/0/0)
- async: clean (seeds ran: 7/0/2/0)
- unsafe: N/A (seeds: 0/0/3/0 all zero; `from_raw` hits are `from_raw_os_error` false positives)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: 2 finding(s)