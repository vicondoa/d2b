# d2b-p2 - d2b - part 2/3
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 6600 (excl. src/generated/**) | modules: doctor.rs, zone_audit.rs, resource.rs, host_validate.rs, shell.rs, host.rs, lib.rs, complete.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: d2b-p2 per U1 (f): src/doctor.rs, src/zone_audit.rs, src/resource.rs, src/host_validate.rs, src/shell.rs, src/host.rs, src/lib.rs, src/complete.rs

## idiom
- d2b-p2#1 sev=low blast=leaf effort=S verdict=actionable - the pidfd-table inspection detail string is re-derived by an identical 3-line match at five check sites - fix: add `PidfdEntries::state_detail() -> String` next to `load_pidfd_entries` and call it from `check_otel_host_bridge_runner`, `check_usbipd_runners`, `check_seccomp_bpf_loaded`, `check_pre_ns_posture_with_reader`, `check_broker_reap_health` - [packages/d2b/src/doctor.rs:529, packages/d2b/src/doctor.rs:579, packages/d2b/src/doctor.rs:1230, packages/d2b/src/doctor.rs:1343, packages/d2b/src/doctor.rs:1449]
  evidence: seed 3 (`let mut \w+ = (String|Vec)::new\(\)`) = 3 hits, all deliberate text building; the repeated `match &entries.state { PidfdState::ParseError(d) => d.clone(), _ => "daemon state dir unreadable".to_owned() }` block read at 5 sites
- d2b-p2#2 sev=low blast=leaf effort=M verdict=actionable - `typed()` hand-rolls eight field-by-field typed-args to generic-args conversions with ~20 clones instead of `From` impls - fix: implement `From<TypedListArgs> for GenericListArgs` (and the other six pairs) consuming the typed args, and change `typed()`/`typed_noun()` to take `TypedResourceArgs` by value so the conversions stop cloning - [packages/d2b/src/resource.rs:525, packages/d2b/src/resource.rs:546, packages/d2b/src/resource.rs:555, packages/d2b/src/resource.rs:565, packages/d2b/src/resource.rs:592, packages/d2b/src/resource.rs:608, packages/d2b/src/resource.rs:617]
  evidence: own seed 1 (`\.clone\(\)`) = 52 hits, ~20 of them inside `typed()` at resource.rs:533-621; the generic structs (GenericListArgs etc.) are the same fields as the typed structs, so `From` applies
- d2b-p2#3 sev=low blast=leaf effort=S verdict=actionable - `valid_digest` and `valid_hash` in zone_audit.rs are byte-identical functions - fix: keep one (e.g. `valid_digest`) and delete the other, updating its three call sites - [packages/d2b/src/zone_audit.rs:805, packages/d2b/src/zone_audit.rs:838]
  evidence: reading both bodies: identical `strip_prefix("sha256:")` + 64-hex check; `valid_hash` is called at zone_audit.rs:376 and 410, `valid_digest` at 670, 806, 816, 822
- d2b-p2#4 sev=low blast=leaf effort=M verdict=actionable - the v1 record path in `validate_record` duplicates the ~22-line chain-verification tail of the nested `validate_v2_record` (hash extraction, expected-previous check, canonical json! build, digest compare) - fix: extract `verify_chain(object, fields_key, expected_previous) -> Result<String, RecordValidationError>` and call it from both the v1 path and `validate_v2_record` - [packages/d2b/src/zone_audit.rs:349, packages/d2b/src/zone_audit.rs:395]
  evidence: reading zone_audit.rs:349-429: the nested fn body and the outer v1 tail are identical line-for-line except the envelope/fields validation that precedes them
- d2b-p2#5 sev=low blast=leaf effort=S verdict=actionable - `validate_public_fields` and `validate_v2_fields` have identical bodies differing only in the per-field validator they call - fix: merge into one `validate_fields(class, fields, validate_field: fn(&str, &str, &Value) -> bool)` and pass `validate_public_field`/`validate_v2_field` - [packages/d2b/src/zone_audit.rs:591, packages/d2b/src/zone_audit.rs:607]
  evidence: reading both bodies: same expected-count, contains-key, posture-field, key-subset, and per-field iteration logic; only the validator reference differs

## own
- d2b-p2#6 sev=low blast=leaf effort=S verdict=actionable - three `.clone()` calls feed `json!` operands, which serde_json serializes by reference (`to_value(&expr)`), so the clones are dropped immediately - fix: pass `parsed.schema_version`, `issue_kinds`, and `parsed.issues` to `json!` without `.clone()` - [packages/d2b/src/doctor.rs:1062, packages/d2b/src/doctor.rs:1069, packages/d2b/src/doctor.rs:1070]
  evidence: seed 1 (`\.clone\(\)`) = 52 hits; the three sites sit inside one `json!({...})` literal at doctor.rs:1061-1071 where the macro borrows each operand, making each clone redundant
- clean: seeds 1/2/3 = 52/116/2 hits; the remaining clones are explainable (report rows owning their detail strings, typed-args to generic-request struct conversion at dispatch, `Option` unwrap_or_else ownership, test fixtures); the two `RefCell`/`Mutex` hits are the `#[cfg(test)]` stdout-capture statics in lib.rs:74-82

## type
- d2b-p2#7 sev=low blast=leaf effort=M verdict=actionable - the "exactly one of --dry-run / --apply" invariant lives as a bool pair in six clap arg structs and is hand-rechecked at four call sites with diverging exit codes - fix: introduce `enum MutationMode { DryRun, Apply }` with a single `MutationMode::from_flags(dry_run, apply) -> Result<MutationMode, CliFailure>` constructor and a shared missing-flag error, then use it in `require_mutation_flags`, `mutation`, `reconcile`, and `validate` - [packages/d2b/src/resource.rs:880, packages/d2b/src/host.rs:274, packages/d2b/src/host.rs:303, packages/d2b/src/host.rs:332]
  evidence: seed 2 (`is_\w+: bool|\w+_flag: bool`) = 2 hits (AuditStreamValidator state bools, judged fine); the dry_run/apply pairs were found by reading: DeviceUsbAttachArgs, DeviceUsbDetachArgs, DeviceSecurityKeyCancelArgs (resource.rs), HostMutationArgs, HostValidateArgs, HostReconcileArgs (host.rs)

## api
- d2b-p2#8 sev=low blast=leaf effort=S verdict=actionable - the d2b lib exports a wide pub surface while the only external consumer (xtask) uses just `d2b::cli_command()`; `pub mod host_generation` and `pub const EXIT_API_TIMEOUT` have zero consumers anywhere - fix: narrow `doctor`/`host_validate` pub items and `EXIT_API_TIMEOUT` to `pub(crate)`, make `host_generation` a private `mod`, keeping only `cli_command`/`run` public - [packages/d2b/src/lib.rs:25, packages/d2b/src/lib.rs:41, packages/d2b/src/doctor.rs:62, packages/d2b/src/host_validate.rs:55]
  evidence: census: `use d2b::` over packages/, nixos-modules/, tests/, labs/ = 5 hits, all `d2b::cli_command()` in packages/xtask/src/main.rs:841-922; `host_generation` over packages/d2b = 1 hit (the lib.rs:25 declaration itself); `EXIT_API_TIMEOUT` over packages/d2b = 1 hit (the lib.rs:41 declaration); `#![allow(dead_code)]` at lib.rs:1 hides the zero-consumer items from the compiler

## err
- d2b-p2#9 sev=medium blast=leaf effort=S verdict=actionable - `CliFailure` flattens the error class into the message (`format!("{error_class}: {message}")`), so callers recover the class by string-matching the message prefix - fix: add a structured `code: &'static str` field to `CliFailure` (lib.rs:44-52), populate it in `ZoneContext::failure` (context.rs:1181-1189), and match on it in `can_fallback_to_local_state` and `reconcile_deadline` instead of `message.split(':').next()` / `strip_prefix("ref-invalid: ")` - [packages/d2b/src/host.rs:200, packages/d2b/src/resource.rs:916, packages/d2b/src/lib.rs:44]
  evidence: seed 1 (`\.unwrap\(\)|\.expect\(`) = 51 hits, all in `#[cfg(test)]` or after an adjacent compiler-invisible check (zone_audit.rs:99); the string-match recovery was read at host.rs:200-204 and resource.rs:916-918
- d2b-p2#10 sev=medium blast=leaf effort=S verdict=needs-contract - `d2b host prepare`/`destroy` without flags exit 2 with kind `ref-invalid`, diverging from the documented `--apply-or-dry-run-required` exit-78 envelope; `host reconcile` exits 78 but with the wrong kind - fix: route `mutation()` and `reconcile()` through `missing_mutation_flag_envelope` (dispatch.rs:369-375) like `validate()` already does, or correct docs/reference/error-codes.md:156 - [packages/d2b/src/host.rs:274, packages/d2b/src/host.rs:303, packages/d2b/src/host.rs:332, docs/reference/error-codes.md:156]
  evidence: seed 3 (`\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(`) = 1 hit (test-only); error-codes.md:156 pins `--apply-or-dry-run-required` exit 78 for host prepare/destroy/install flags while host.rs:274-278 emits `ref-invalid` exit 2; `missing_mutation_flag_envelope` (dispatch.rs:369) already emits the documented shape and `validate()` uses it

## serde
- clean: seeds 1-2/3/4 = 42/0/26 hits; all Deserialize derives are loose forward-compatible shapes for daemon-persisted state files (`serde(default)` on every field, `#[allow(dead_code)]` on unused fields), DoctorStatus/WaveStatus serialize kebab-case for the CLI wire, and no hand-written Deserialize exists; `deny_unknown_fields` absence is deliberate for service-consumed messages

## obs
- N/A: seeds 1/2/3/4 = 0/0/0/19 all effectively zero (the 19 seed-4 hits are the `surface_catalog::` substring false positive, verified: 19 of 19 match `surface_catalog`); packages/d2b/Cargo.toml declares no tracing/log dependency, so the lens's N/A criteria hold

## docs
- d2b-p2#11 sev=low blast=leaf effort=S verdict=actionable - several pub items carry no doc comment and lib.rs has no crate-level `//!` doc - fix: add one-line first-sentence docs to `DoctorReport`, `run_doctor`, `render_summary`, `render_human` (doctor.rs), `ValidateReport`, `ValidateMode`, `exit_code` (host_validate.rs), `cli_command`, `run` (lib.rs), and a `//!` crate doc in lib.rs - [packages/d2b/src/doctor.rs:91, packages/d2b/src/doctor.rs:163, packages/d2b/src/host_validate.rs:229, packages/d2b/src/host_validate.rs:237, packages/d2b/src/host_validate.rs:625, packages/d2b/src/lib.rs:215, packages/d2b/src/lib.rs:221]
  evidence: docs seed 1 (`^\s*pub (fn|struct|enum|trait|const|type)`) = 30 hits, seed 2 (`/// # (Examples|Errors|Panics|Safety)`) = 0 hits, seed 3 (`-> Result<`) = 47 hits with no `# Errors` sections anywhere; the nine undocumented items were read directly

## perf
- clean: seeds 1/2/3 = 117/23/13 hits; every `format!` is in a cold CLI error, report-rendering, or one-shot diagnostic path (doctor probes, evidence payloads, completion scripts), the `Vec::new()` hits are empty-case returns or test capture buffers, and `.to_string()` sits at wire-rendering boundaries; no hot loop or per-item allocation exists in this CLI partition, so all perf observations are static (unmeasured)

## conc
- clean: seeds 1/2/3/4 = 2/2/0/1 hits, every one `#[cfg(test)]` (doctor.rs:2433 test sleep, shell.rs:343 test Mutex, lib.rs:74-82 test stdout-capture thread_local + Mutex); production code in the partition uses no threads, locks, atomics, or manual Send/Sync impls

## async
- N/A: seeds 1-4 = 0 hits combined; no `async fn`, `.await`, tokio spawn, sync primitives, or `#[tokio::]` attribute in the partition (the async transport lives in context.rs/exec_client.rs, other parts); the shell watch loop is deliberately synchronous CLI polling with `std::thread::sleep` under a deadline (shell.rs:265)

## unsafe
- N/A: seeds 1/2/3 = 0/0/0 hits; no unsafe blocks, fns, impls, SAFETY comments, or transmute/raw-pointer patterns in the partition; the crate inherits the workspace `unsafe_code = "forbid"` lint table

## ffi
- N/A: seeds 1-4 = 0 hits combined; no extern "C", no_mangle, catch_unwind, repr(C)/repr(transparent), or CStr/CString usage in the partition

## macro
- N/A: seeds 1-4 = 0 hits combined; no macro_rules!, proc-macro, syn/quote, `$crate`, or to_compile_error usage in the partition

## test
- clean: seeds 1/3/4 = 153/0/0 hits (62 unit tests in the partition's src files, 91 in tests/); the suite is behavioral and independently grounded: FIPS 180-4 SHA-256 vectors (host_validate.rs:657-672), wave-catalog parity vs nixos-modules/options-daemon.nix (host_validate.rs:687, tests/host_validate_verb.rs:209), golden CLI output pins, redaction assertions (zone_audit.rs:973-995), and fail-closed envelope checks; no proptest/insta/rstest, no `#[ignore]`; the D2B_FIXTURES-gated tests in tests/cli_json_contract.rs print an explicit SKIP line and are documented gating, not silent passes

## Coverage
- idiom: 5 finding(s)
- own: 1 finding(s)
- type: 1 finding(s)
- api: 1 finding(s)
- err: 2 finding(s)
- serde: clean (seeds ran: 42/0/26; loose daemon-state shapes deliberate, no hand-written Deserialize)
- obs: N/A (seeds: 0/0/0/19 all zero or surface_catalog substring false positives; no tracing/log dependency in Cargo.toml)
- docs: 1 finding(s)
- perf: clean (seeds ran: 117/23/13; all cold CLI paths, static unmeasured)
- conc: clean (seeds ran: 2/2/0/1; all cfg(test) hits)
- async: N/A (seeds: 0/0/0/0 all zero; no async code in the partition)
- unsafe: N/A (seeds: 0/0/0 all zero; workspace forbids unsafe_code)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 153/0/0; behavioral, golden-pinned, parity-checked suite)