# d2b-provider-config-nixos - d2b-provider-config-nixos
Baseline: 6ebdd4cec | LOC audited: 1816 (src/ 1427, tests/ 389; excl. src/generated/**, none present) | modules: whole crate (lib, controller, service, ttrpc)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none (whole crate)

## idiom
- d2b-provider-config-nixos#1 sev=low blast=leaf effort=S verdict=actionable - the path-component rejection walk is written twice with identical rules, and the two copies can drift (one checks, one checks and collects) - fix: extract one helper classifying `Path::components()` into `Result<Vec<&OsStr>, ConfigError>` (reject `CurDir`/`ParentDir`/`Prefix`) and call it from both `validate_reader_path` and `read_bounded_file` - [packages/d2b-provider-config-nixos/src/ttrpc.rs:379-386, packages/d2b-provider-config-nixos/src/ttrpc.rs:414-421]
  evidence: seeds 0/0/1 (`let mut components = Vec::new()` at ttrpc.rs:414 plus the loop); both walks read in full
- clean: hand-written `Debug` impls (controller.rs:99-119, 120-131, ttrpc.rs:40-49, 160-164) redact secret fields and are the deliberate-derive-exclusion class; naming (`as_str`, `ALL`, no `get_`) is consistent; no index loops over `0..n`

## own
- d2b-provider-config-nixos#2 sev=low blast=leaf effort=S verdict=actionable - `ConfigService::validate_operation` clones the whole JSON payload per request (`serde_json::from_value::<T>(payload.clone())`, 6 arms) - up to `MAX_CONFIG_ENCODED_BYTES` (~683 KiB base64 limit) per Stage/ReadGuestConfig admission on the async polling worker, when `serde` can deserialize borrowed: `T::deserialize(payload)` works on `&serde_json::Value` - fix: replace the 6 `from_value(payload.clone())` calls with `T::deserialize(payload)` (or change the signature to take `Value` by value and clone once at the single call site) - [packages/d2b-provider-config-nixos/src/controller.rs:298, packages/d2b-provider-config-nixos/src/controller.rs:304, packages/d2b-provider-config-nixos/src/controller.rs:311, packages/d2b-provider-config-nixos/src/controller.rs:318, packages/d2b-provider-config-nixos/src/controller.rs:325, packages/d2b-provider-config-nixos/src/controller.rs:331]
  evidence: seed `\.clone\(\)` = 19 hits in src (6 here; the rest are guest_ref copies into owned responses and test fixtures, each explainable); `Arc` sharing (ttrpc.rs:126, 267, 315) is genuine multi-handler/worker ownership with the daemon caller at d2bd/src/composition.rs:4718
- d2b-provider-config-nixos#3 sev=low blast=leaf effort=S verdict=actionable - `GuestConfigReader::dispatch` copies the just-validated document bytes (`document.bytes().to_vec()`, up to 512 KiB per guest read on the dedicated worker) only so `read_guest_config` can re-validate the already-valid `GuestConfigDocument` - fix: give `GuestConfigDocument` a consuming accessor (`into_bytes()` or `impl From<GuestConfigDocument> for Vec<u8>`, `bytes` field stays private) and pass it straight into the `impl Into<Vec<u8>>` parameter - [packages/d2b-provider-config-nixos/src/ttrpc.rs:111, packages/d2b-provider-config-nixos/src/controller.rs:106]
  evidence: seed `\.to_vec\(\)` = 1 prod hit; copy bound is static (MAX_CONFIG_BYTES = 512 KiB, service.rs:15)
- clean: `GUEST_CONFIG_IDENTIFIER.to_owned()` in 7 constructors, `guest_ref.clone()` into owned responses, `sha256.clone()` into the approval receipt, and the test fixture clones are each the cheapest correct ownership move; no `Rc`/`RefCell`/`Cow`/`Arc<Mutex>` in production code

## type
- d2b-provider-config-nixos#4 sev=low blast=leaf effort=M verdict=actionable - stringly-typed request fields (`identifier`, `against`, `destination`) are re-validated at every entry (constructors, store methods, and `validate_operation`), and the duplicated guest-ref checks have already drifted: `ConfigSyncRequest::new` checks only the resource type while `validate_guest_ref` also requires a non-empty name, so "Guest/" passes the constructor yet fails the boundary - fix: parse-once request fields (private fields, `new()`/`try_from` as the only constructors, transparent serde keeps the wire JSON unchanged) so the per-entry `validate_*` calls collapse; align `ConfigSyncRequest::new` with `validate_guest_ref` - [packages/d2b-provider-config-nixos/src/service.rs:31-34, packages/d2b-provider-config-nixos/src/service.rs:300-306, packages/d2b-provider-config-nixos/src/service.rs:316-323, packages/d2b-provider-config-nixos/src/service.rs:326-336]
  evidence: seed `fn validate_\w+` = 6 hits (validate_operation, validate_guest_ref, validate_identifier, validate_view_identifier, validate_destination, validate_reader_path), each with 2-3 call sites; `is_\w+: bool` and `(mode|kind|state): String` = 0; identifier stays a wire field (forward-compat token), no `needs-contract` claim

## api
- d2b-provider-config-nixos#5 sev=low blast=wide effort=S verdict=actionable - `decode_document` (service.rs:296-298) is a public one-line forwarder duplicating the already-public `ConfigSyncResponse::document()`, giving two API paths for one operation - fix: drop the export and call `.document()` at the one live caller (d2bd/src/composition.rs:11585), or privatize `document()` and keep the named helper - [packages/d2b-provider-config-nixos/src/service.rs:296-298, packages/d2b-provider-config-nixos/src/lib.rs:22]
  evidence: census: `decode_document` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 3 hits (lib.rs:22 re-export, service.rs:296 definition, composition.rs:11585 caller); `ConfigNixosClient` (composition.rs:11571), `GuestConfigReader` + `create_ttrpc_services` (composition.rs:4711-4720) and `ConfigStagingStore` (composition.rs:578, 29824) all have live daemon callers - per assignment, do not re-flag the policy status (provider_crate_policy.rs:21 lists config-nixos as non-provider-prefixed; finding 6 elsewhere owns it)
- clean: `Arc<dyn ConfigServiceBackend>` in `create_ttrpc_services` is genuine shared ownership (6 method handlers + one dispatch worker share the backend; caller wraps `Arc::new` at composition.rs:4718); single-path re-export of the whole surface from lib.rs; no dependency types leak; `ConfigServiceBackend` is a one-required-method trait

## err
- d2b-provider-config-nixos#6 sev=low blast=leaf effort=S verdict=actionable - `invalid_status()` maps client-side request-encoding failures to ttrpc `INVALID_ARGUMENT` plus the `config-document-encoding-failed` code, telling the caller their request was invalid when the client implementation failed to serialize - fix: map that site to `INTERNAL` (or reuse `rpc_error(ConfigError::EncodingFailed)`) so status class matches the code - [packages/d2b-provider-config-nixos/src/ttrpc.rs:372-376, packages/d2b-provider-config-nixos/src/ttrpc.rs:189]
  evidence: seed `\.unwrap\(\)|\.expect\(` = 16 hits: 2 in production (ttrpc.rs:133, 194), both `expect("canonical config operation prefix")` on constant `ConfigOperation::as_str()` output (false-positive class); the rest are `#[cfg(test)]`; 1 `panic!` (ttrpc.rs:548, test); 2 `let _ =` (ttrpc.rs:282 deliberate panic-safe send documented inline, ttrpc.rs:493 test)
- clean: `ConfigError` is a 12-variant taxonomy split by caller action with stable `code()` strings, `Display` = code, and a clean `rpc_error` mapping (ttrpc.rs:360-376); codes are provider-local (no hits in docs/reference/error-codes.md or tests/golden/), so no wire contract is pinned

## serde
- clean: seeds 10/20/0/11 (10 `derive)...Serialize...JsonSchema)` types, 20 `serde(rename_all/deny_unknown_fields)` attributes, 0 hand-written `Deserialize`, 11 `serde_json::from_/to_` sites). Every DTO is `rename_all = "camelCase"` + `deny_unknown_fields`, the derive-based admission gate at the RPC boundary is the deliberate pattern, bounds are re-applied after decode (`ConfigSyncResponse::document`, `ConfigStageRequest::document`), and round-trips are exercised via `validate_operation` in tests/service_contract.rs

## obs
- clean: seeds 0/22/0/22 (0 `println!`/`eprintln!`, 22 tracing events, 0 `.instrument`, 22 `tracing::` uses). Every event uses named fields (`resource`, `operation`, `error`) under the consistent `config-nixos ...` scheme; the message-only `debug!` rejection events in `read_bounded_file` (ttrpc.rs:444-466) sit under the boundary `warn!` that carries `resource` (ttrpc.rs:99-102); no secret or path text reaches a field (redaction tested in tests/redaction.rs); errors are logged once at the handling boundary

## docs
- d2b-provider-config-nixos#7 sev=low blast=leaf effort=S verdict=actionable - none of the ~14 public `Result`-returning APIs carry a `# Errors` section, so callers cannot learn which `ConfigError` variants each returns without reading the implementation (lib.rs:6 denies missing_docs but only the one-liners exist) - fix: add `# Errors` to `GuestSessionEvidence::new`, `GuestConfigDocument::new`, `ConfigSyncResponse::document`, `ConfigStageRequest::document`, the five `ConfigStagingStore` methods, `GuestConfigReader::new`, and the two `ConfigService` methods - [packages/d2b-provider-config-nixos/src/controller.rs:45-64, packages/d2b-provider-config-nixos/src/service.rs:359-475, packages/d2b-provider-config-nixos/src/ttrpc.rs:52-72]
  evidence: seeds 63/0/14 (63 public items, 0 canonical `# Examples`/`# Errors`/`# Panics`/`# Safety` sections, 14 `-> Result<` items); all first sentences are one-line and strong, docs otherwise exemplary

## perf
- d2b-provider-config-nixos#8 sev=low blast=leaf effort=M verdict=actionable - the RPC path parses the request JSON up to three times per call: handler `from_slice` (ttrpc.rs:326), `validate_operation` `from_value` plus the full-document base64 decode for Stage (controller.rs:298-331), and the backend dispatch `from_value` again (ttrpc.rs:81); Stage payloads can reach ~683 KiB base64 - fix: decode the typed request once in `ConfigMethod::handler`, validate the typed value, and pass the original `Value` to the backend hop (removes one parse and the admission-time document decode) - [packages/d2b-provider-config-nixos/src/controller.rs:298, packages/d2b-provider-config-nixos/src/ttrpc.rs:326, packages/d2b-provider-config-nixos/src/ttrpc.rs:81]
  evidence: static (unmeasured); seeds: `format!\(` = 4 prod sites (crate-constant service strings and the sha256 prefix, all required), `Vec::new()` = 1 (cold path), `.to_string()` = 0; `read_bounded_file` already uses `with_capacity` (ttrpc.rs:452)

## conc
- clean: seeds 0/1/0/0 (1 `Mutex<` - the `std::sync::Mutex<Option<Receiver<()>>>` inside the `#[cfg(test)]` `ParkingBackend` at ttrpc.rs:483, a sanctioned `cfg(test) helper` with the recorded allow; 0 atomics, 0 `thread_local!`). The one production thread (`thread::Builder` at ttrpc.rs:241-248) is the R4 dedicated bounded worker with the `disallowed_methods` allow citing "dedicated bounded worker per plan R4" and a bounded `sync_channel` queue - cited, not re-flagged (U1 constraint d.2/d.4)

## async
- clean: seeds 7/1/0/1 (3 `async fn`, 4 `.await`, 1 `select!`, 1 `#[tokio::test]`). The blocking guest-config read is correctly hoisted off the polling worker: `try_send` admission, `oneshot` reply, full queue maps to `Unavailable` (never parks the executor, never grows threads), the dropped-sender path is `map_err`-handled (ttrpc.rs:266-312) - the sanctioned R4 pattern; the `current_thread` parked-dispatch test (ttrpc.rs:509-558) proves the actual hazard; no lock held across `.await`

## unsafe
- clean: seeds 0/0/1/1 - no `unsafe` blocks, fns, impls, or `extern`; crate-level `#![forbid(unsafe_code)]` (lib.rs:8) and manifest `unsafe_code = "forbid"`; the single seed-3 hit is `rustix::fs::FileType::from_raw_mode` (ttrpc.rs:442), a safe constructor whose name merely contains "from_raw" - false positive; crate is not in the U1 (d) 8 exception list, consistent

## ffi
- clean: N/A (seeds 0/0/0/0 - no `extern "C"`, `no_mangle`, `catch_unwind`, `repr(C)`/`repr(transparent)`, or `CStr`/`CString`/`c_char`; the ttrpc service registration is in-process proto dispatch, not a C ABI boundary)

## macro
- clean: N/A (seeds 0/0/0/0 - no `macro_rules!`, proc-macro machinery, `$crate`, or `to_compile_error`/`new_spanned`; no macro use that a function cannot serve)

## test
- d2b-provider-config-nixos#9 sev=medium blast=leaf effort=S verdict=actionable - `ConfigSyncResponse::document()`'s integrity contract (forged `sha256`/`bytes` mismatch must fail `EncodingFailed`, over-bound `content_base64` must fail `InvalidRequest`) is untested, and the daemon depends on this exact decode path (d2bd/src/composition.rs:11585) - fix: add integration tests that literal-construct a `ConfigSyncResponse` (fields are pub) with a wrong digest, a wrong byte count, and an over-`MAX_CONFIG_ENCODED_BYTES` payload and assert the failure codes - [packages/d2b-provider-config-nixos/src/service.rs:70-90, packages/d2b-provider-config-nixos/tests/config_lifecycle.rs:8-18]
  evidence: seeds `#\[test\]|#\[tokio::test\]` = 9 tests, `assert*` = 43; none touch the integrity branch of `document()` (read via full scan of tests/)
- d2b-provider-config-nixos#10 sev=low blast=leaf effort=S verdict=actionable - no test exercises the `deny_unknown_fields` admission (an extra JSON key must make `validate_operation` fail `InvalidRequest`) or a payload with wrong field types (serde error path), which is exactly the typo-key case the attribute exists for - fix: extend `operation_validation_enforces_closed_identifiers_and_semantic_bounds` (tests/service_contract.rs:41-79) with an unknown-field payload and a wrong-typed payload - [packages/d2b-provider-config-nixos/src/service.rs:20-21, packages/d2b-provider-config-nixos/tests/service_contract.rs:41-79]
  evidence: seeds `assert*` = 43, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0; the existing rejection tests cover semantic values only (wrong guest type, wrong identifier, non-digest view)
- clean: the 9 tests cover happy read, stale-session fail-closed, staging lifecycle, idempotent approval retry, zone isolation, unauthorized callers, path rejection, hardlink rejection, redaction, bounds, and the closed method surface; the parked-dispatch test (ttrpc.rs:509-558) asserts the real executor hazard; expectations are human-written, no network, no clock, seeded determinism holds

## Coverage
- idiom: 1 finding
- own: 2 findings
- type: 1 finding
- api: 1 finding
- err: 1 finding
- serde: clean (seeds ran: 10/20/0/11)
- obs: clean (seeds ran: 0/22/0/22)
- docs: 1 finding
- perf: 1 finding
- conc: clean (seeds ran: 0/1/0/0)
- async: clean (seeds ran: 7/1/0/1)
- unsafe: clean (seeds ran: 0/0/1-false-positive/1; only `from_raw_mode` name collision and the `forbid` attribute)
- ffi: N/A (seeds: 0/0/0/0 all zero; no FFI boundary exists - ttrpc registration is in-process)
- macro: N/A (seeds: 0/0/0/0 all zero; no macro definitions or proc-macro machinery)
- test: 2 findings