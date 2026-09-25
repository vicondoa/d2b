# d2b-contracts - d2b-contracts
Baseline: 6ebdd4cec | LOC audited: 10,743 (excl. src/generated/**; no tests/** Rust files - only tests/fixtures/workload-execution-posture-v1.json fixture) | modules: whole crate (lib.rs + 26 public modules + src/v3/{mod,ifname}.rs; generated/ excluded)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate

## idiom
- d2b-contracts#1 sev=medium blast=family effort=S verdict=actionable - d2b-contracts-control::public_wire re-declares a private copy of `validate_audit_page` instead of reusing the public export in d2b-contracts::audit_wire (the canonical home), so the two copies can drift - fix: import `validate_audit_page` from d2b_contracts::audit_wire in d2b-contracts-control/src/public_wire.rs (alongside the AExisting AuditExportCursor/Entry import at public_wire.rs:1)and delete the pub(crate) copy at public_wire.rs:2203 - [packages/d2b-contracts/src/audit_wire.rs:51, packages/d2b-contracts-control/src/public_wire.rs:2203]
  evidence: census: `validate_audit_page` over packages/tests/docs = 5 hits: 2 definitions + 2 call sites (broker_wire.rs:2206, public_wire.rs:2193) + 1 import; the private copy duplicates the canonical home.

## own
- clean: seeds ran: ~100/~46/0/0 ).clone(), .to_owned/.to_vec/.to_string, Rc/RefCell/Arc<Mutex>/Arc<RwLock>, Cow<); every production clone has a one-sentence ownership justification (negotiation-record ownership capability.rs:218, error-context ownership controller_config.rs:158 and identity_config.rs:149, owned-String accessors error.rs:674,691, wire-rendering string surfaces identity.rs:368/473/598, RealmTarget construction from a borrow target.rs:261/430/440, duplicate-target error message unsafe_local_workloads.rs:60/67and realm.rs:194-195); remaining clones sit in unit tests, and there are no Rc/RefCell/Arc/Cow sites.

## type
- d2b-contracts#2 sev=medium blast=wide effort=M verdict=needs-contract - `complete: bool` + `next_cursor: Option<&AuditExportCursor>` in the audit-page surface encode exactly-two-valid-of-four states and both invalid combos are type-expressible; an enum would make them unrepresentable - fix: replace the pair with `enum AuditPageEnd { Complete, More(AuditExportCursor) }` and drive the wire structs/validators from it (callers: d2b-contracts-broker/src/broker_wire.rs:2206,and d2b-contracts-control/src/public_wire.rs:2193), keeping the existing admission behaviour - [packages/d2b-contracts/src/audit_wire.rs:51-59, packages/d2b-contracts-broker/src/broker_wire.rs:2184-2187, packages/d2b-contracts-control/src/public_wire.rs:2167-2173]
  evidence: seed `fn validate_\w+|fn check_\w+|is_\w+: bool|\w+_flag: bool|(mode|kind|state): String` = 16 hits; the `complete`/`next_cursor` pair is the Option-pair smell the skill names; wire shape pinned by docs/reference/daemon-api.md:385,417 and schemas/v2/wire-protocol.json (AuditExportEntry/response definitions.

## api
- d2b-contracts#3 sev=medium blast=leaf effort=S verdict=actionable - `pub fn validate_usb_bus_id` (types.rs:140) is never called in production and re-implements usbip.rs::validate_bus_id with a divergent contract (64-byte cap vs SYSFS_BUS_ID_MAX=31, requires a `-` separator whereusbip accepts bare `B`, plain String error vs typed `BusIdError`)- fix: delete types.rs::validate_usb_bus_id and move its covariance cases (types.rs:202-203into the usbip.rs test table so one canonical bus-id checker survives - [packages/d2b-contracts/src/types.rs:140, packages/d2b-contracts/src/usbip.rs:44]
  evidence: census: `validate_usb_bus_id` over packages/tests/docs = 3 hits: 1 definition + 2 test assertions, 0 production callers; the canonical sibling is usbip.rs::validate_bus_id`.
- d2b-contracts#4 sev=medium blast=leaf effort=S verdict=actionable - `pub fn MediaRef::validate_value` (types.rs:114) is dead:the `opaque_id!`-generated `MediaRef::new` accepts any string without calling it, so the public fn advertises a shape check that never runs on the type it names - fix: either delete the fn, or wire it into construction via a `TryFrom<&str>` boundary on MediaRef (the house parse-gate pattern)so calers cannot bypass it - [packages/d2b-contracts/src/types.rs:96-113, packages/d2b-contracts/src/types.rs:114]
  evidence: census: `MediaRef::validate_value` over packages/tests/docs = 3 hits: 1 definition + 2 test assertions (types.rs:200-201), 0 production callers (the generated macro body at types.rs:14-25 calls no validator.

## err
- d2b-contracts#5 sev=medium blast=leaf effort=L verdict=actionable - Public constructors/validators return `Result<_, String>` or `&'static str` (ConfiguredArgv::new configured_argv.rs:15, RealmWorkloadsLauncherV2Json::validate launcher.rs:21, UnsafeLocalWorkloadsJson/LocalVmConfiguredWorkload/UnsafeLocalWorkload::validate unsafe_local_workloads.rs:35/81/96, MediaRef::validate_value and validate_usb_bus_id types.rs:114/140, validate_audit_page audit_wire.rs:51) while sibling validators inthe same crate use typed enum errors (BusIdError, IfNameError, IdError, ContractStringError, IdentityError, TokenError), forcing callers to string-match instead of matching variants - fix: introduce typed error enums per surface (e.g. `ConfiguredArgvError`, `UnsafeLocalWorkloadsError`, `LauncherMetadataError`, `MediaRefError`)with thiserror-style Display + std::error::Error impls and return them; call sites that only `.unwrap()` (census: ConfiguredArgv::new used in d2b-contracts-control, d2bd-runtime, d2bd, d2b-unsafe-local-helper) compile unchanged - [packages/d2b-contracts/src/configured_argv.rs:15, packages/d2b-contracts/src/launcher.rs:21, packages/d2b-contracts/src/unsafe_local_workloads.rs:35, packages/d2b-contracts/src/types.rs:114, packages/d2b-contracts/src/audit_wire.rs:51]
  evidence: seed `\.unwrap\(\)|\.expect\(|let _ = |\.ok\(\);|\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 209 hits,mostly test-local; the 6 production String-error surfaces are named above;`enum \w*Error` =the typed-error sibling taxonomy the change would match.

## serde
- d2b-contracts#6 sev=medium blast=wide effort=M verdict=needs-contract - `AuditExportEntry` carries `record: Option<Value>` andi `error: Option<AuditExportErrorCode>` where the doc (audit_wire.rs:27) promises exactly one is always populated,so both-None is a wire-accepted illegal state (the broker's own writers d2b-broker/src/audit.rs:1730-1732 etc always set one,but a literal or foreign producer can emit neither) - fix: replace the pair with an enum payload representation (e.g. `#[serde(tag = "type")] enum AuditExportEntryPayload { Record { record: Value }, Error { error: AuditExportErrorCode } }` or an admission-gate enforcing exactly-one at decode),preserving or explicitly changing the wire shape - [packages/d2b-contracts/src/audit_wire.rs:27-36]
  evidence: seed `derive\([^)]*(De)?[Ss]erialize` = ~240 hits,`serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` = ~80,`impl .*Deserialize.*for` = ~30,`serde_json::from_|to_` = ~28; the crate's serde discipline is otherwise uniform (rename_all/deny_unknown_fields/skip_serializing_if defaults everywhere); wire shape pinned by docs/reference/daemon-api.md:385,417 and schemas/v2/wire-protocol.json (AuditExportEntry definition.

## obs
- N/A (seeds: 0/0/0/0 all zero; crate declares no println/log call and Cargo.toml carries no tracing/log dependency - only semver, serde, serde_json, schemars, sha2.

## docs
- d2b-contracts#7 sev=low blast=leaf effort=M verdict=actionable - Public Result-returning parse/validate fns carry failure conditions only in prose (e.g. ids.rs:138-139, usbip.rs:44-46, contract_id.rs:99, realm.rs:88-90)and only one canonical doc section exists crate-wide,so`cargo doc` readers get no uniform `# Errors` contract - fix: add canonical `# Errors` sections to the public `-> Result<` fns (keeping the existing prose as the section bodies),matching the one existing `# Examples` pattern at workload_identity.rs:46 - [packages/d2b-contracts/src/ids.rs:138, packages/d2b-contracts/src/usbip.rs:44, packages/d2b-contracts/src/contract_id.rs:99, packages/d2b-contracts/src/audit_wire.rs:51]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = ~520 hits,`/// # (Examples|Errors|Panics|Safety)` = 1 hit (workload_identity.rs:46),`-> Result<` = ~115; pub items themselves are uniformly documented(no missing-docs class found),so the finding is section-shape,not coverage.

## perf
- clean: seeds ran: ~70/~10/~27 (format!, Vec/VecDeque/HashMap/BTreeMap::new, .to_string()); all format! sites are error paths, one-shot render/schema builders, deserialize gates, or tests (e.g. error.rs:691-692, configured_argv.rs:53-74, ifname.rs:292,306,324-335),the Vec::new sites are empty-constructor/deserialize-gate/test builders, and no hot-path allocation class was found; static (unmeasured).



## conc
- N/A (seeds: 0/0/0/0 all zero; no threads, mutexes, atomics, or unsafe Send/Sync claims in the crate.

## async
- N/A (seeds: 0/0/0/0 all zero; no async fns, awaits, spawns, otokio runtime usage in the crate.

## unsafe
- N/A (seeds: 0/0/0/0 all zero; no unsafe blocks/fns/impls, SAFETY comments, transmute/from_raw/MaybeUninit, or unsafe_code text; crate inherits workspace `unsafe_code = "forbid"` via [lints] workspace=true in Cargo.toml.



## ffi
- N/A (seeds: 0/0/0/0 all zero; no extern "C", no_mangle, catch_unwind, repr(C)/repr(transparent), CStr/CString/c_char sites (serde(transparent) is not an FFI repr.

## macro
- clean: seeds ran: 7/0/0/0 (macro_rules! = the 7 definitions: contract_string!, realm_controller_string!, opaque_credential_value!, label_identity!, digest_identity!, id_newtype!, opaque_id!; proc_macro/syn/quote/$crate/to_compile_error = 0); all 7 are impl-per-type wire-newtype generators with concrete fragment specifiers (`ident`, `expr`, edition-2024 `expr_2021`, `meta`) and module-scoped textual scope (no #[macro_export], so no $crate path-shadowing hazard; no procedural macros.



## test
- clean: seeds ran: ~70/~397/0/0 (#[test]/#[tokio::test], assert_eq!/assert_ne!/assert!, proptest/insta/rstest, #[ignore]); the crate's unit suites are table-driven round-trip/round-trip-fixture tests with real `tests/fixtures/workload-execution-posture-v1.json` consumption (workload.rs:222-249), fail-closed decode tests, schema-shape assertions,and redaction assertions; no #[ignore]d, flaky, or assert-nothing tests were found; there is no tests/** Rust integration surface (only the JSON fixture), which matches the crate's contract-module shape.



## Coverage
- idiom: 1 finding(s)
- own: clean (seeds ran: 146 total, all production clones explainable; no Rc/RefCell/Arc/Cow)
- type: 1 finding(s)
- api: 2 finding(s)
- err:  1 finding(s)
- serde:  1 finding(s)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency)
- docs:  1 finding(s)
- perf: clean (seeds ran:  ~70/~10/~27; allocation sites are cold/error/test paths)
- conc: N/A (seeds: 0/0/0/0 all zero; no threading/locks/atomics)
- async: N/A (seeds:  0/0/0/0 all zero; no async surface)
- unsafe: N/A (seeds:  0/0/0/0 all zero; no unsafe blocks; workspace forbids)
- ffi: N/A (seeds:  0/0/0/0 all zero; no FFI surface)
- macro: clean (seeds ran:  7/0/0/0; 7 module-scoped impl-generating macros, no proc macros)
- test: clean (seeds ran:  ~70/~397/0/0; no #[ignore]d or vacuous tests found)