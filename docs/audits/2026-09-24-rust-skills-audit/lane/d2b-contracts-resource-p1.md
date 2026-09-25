# d2b-contracts-resource-p1 - d2b-contracts-resource - part 1/2
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 9718 (excl. src/generated/**) | modules: v3/network.rs, v3/resource_schema.rs, v3/operations/** (mod.rs, error.rs, seal.rs), v3/device.rs, v3/resource_status.rs, v3/volume_state.rs, v3/payload_schema.rs, v3/error.rs, v3/host.rs, v3/bridge.rs, v3/limits.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 1/2 per U1 (f); part 2 owns v3/volume.rs, v3/process.rs, v3/identity.rs, v3/execution_policy.rs, v3/resource.rs, v3/storage.rs, v3/volume_binding.rs, v3/activation_nixos.rs, v3/user.rs, v3/mod.rs, v3/artifact.rs, src/lib.rs

## idiom
- d2b-contracts-resource-p1#1 sev=low blast=leaf effort=S verdict=actionable - `ExternalIpv4Spec::default` (network.rs:597-605) hand-writes exactly the field-wise default (method: Ipv4Method::Dhcp, address: None, gateway: None, dns: Vec::new()) that a derive would produce - fix: add `#[default]` to `Ipv4Method::Dhcp` (network.rs:533) and `#[derive(Default)]` to `ExternalIpv4Spec`, delete the hand-written impl - [packages/d2b-contracts-resource/src/v3/network.rs:597, packages/d2b-contracts-resource/src/v3/network.rs:533]
  evidence: seed `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 6 hits; the other five Default impls (RoutingSpec, DhcpSpec, DnsSpec, MdnsSpec, EgressSpec) preserve non-derivable defaults (mandatory host blocklist, ignoreClientNames=true, cacheSize=1000, reflector=true, masquerade=true) and are deliberate, so this is the only derive-equivalent one
- clean: seeds 1-3 = 1/6/1 hits; the index loop (payload_schema.rs:383) is a bounded depth test, the `let mut values = Vec::new()` (resource_schema.rs:298) is a serde visitor collect, and no naming or conversion drift was found across the part

## own
- d2b-contracts-resource-p1#2 sev=low blast=leaf effort=S verdict=actionable - `StateDigest::parse` clones its String before delegating to `SchemaFingerprint::parse` (volume_state.rs:138), but that function takes `impl Into<String>`, so `value.as_str()` avoids the copy - fix: `SchemaFingerprint::parse(value.as_str())` - [packages/d2b-contracts-resource/src/v3/volume_state.rs:138]
  evidence: seed `\.clone\(\)` = 92 hits; real-code clones reviewed: resource_schema.rs:729/1010/1094/1212/1231 own error-payload strings (required), resource_status.rs:693 `base_projection` needs an owned copy, seal.rs:157-169 `Arc::clone` at the capability boundary (required); the remainder are test fixtures
- clean: seeds 2-4 = 54/0/0 hits; `to_owned`/`to_string` hits are schemars `schema_name()` returns, wire-rendering boundaries, and test sentinels; no Rc/RefCell/Arc<Mutex>/Cow anywhere in the part

## type
- d2b-contracts-resource-p1#3 sev=low blast=leaf effort=S verdict=actionable - `StoreSealIdentity::with_store_epoch` (seal.rs:57-61) documents "Bind the seal identity to a nonzero store epoch" but accepts 0 without a check, and the fn has no callers, so the promised invariant is unenforced and untested - fix: reject 0 (return `Result<Self, StoreError>` or `debug_assert!` plus a documented contract) or drop the nonzero claim from the doc - [packages/d2b-contracts-resource/src/v3/operations/seal.rs:57, packages/d2b-contracts-resource/src/v3/operations/seal.rs:58]
  evidence: census `with_store_epoch` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 1 hit (the definition itself); seed 1 `fn validate_\w+|fn check_\w+` = 15 hits (all in tests and validators of sibling types)
- d2b-contracts-resource-p1#4 sev=medium blast=family effort=M verdict=actionable - store-contract digests are bare `String` (`StoredResource.payload_digest` mod.rs:71, `StoredSchema.payload_digest` mod.rs:239, `PreparedStoreMutation.payload_digest` mod.rs:374) while every other identity in this crate is a parsed newtype (SchemaFingerprint, StateDigest), so a non-digest string can flow through the store boundary without a type-level guarantee - fix: type the three fields as `SchemaFingerprint` (or `StateDigest`) and parse at the backend boundary where the digest is computed - [packages/d2b-contracts-resource/src/v3/operations/mod.rs:71, packages/d2b-contracts-resource/src/v3/operations/mod.rs:239, packages/d2b-contracts-resource/src/v3/operations/mod.rs:374]
  evidence: static read of the three pub fields; consumers that construct or match them: d2b-resource-api/src/store.rs, d2b-resource-api/src/manager_backend.rs, d2b-bus/src/session_seam_tests.rs; seed 3 `(mode|kind|state): String` = 0 hits, so this is the only stringly-typed identity in the part
- clean: seeds 2 = 0 hits (no boolean flag soup); the Option pairs checked (ResourceError optional fields, ResourceStatus timestamps) are genuinely independent

## api
- d2b-contracts-resource-p1#5 sev=low blast=leaf effort=S verdict=actionable - six `pub type` aliases are exported with zero consumers anywhere: `AttachmentSpec` (network.rs:956), `AuthorityDescriptor` (device.rs:129), `OpaqueAuthorityKey` (device.rs:151), `DeviceStatus` (device.rs:760), `DeviceRbacVerb` (device.rs:918), `DeviceTelemetryLabels` (device.rs:1119) - fix: delete the unused aliases (or make them `pub(crate)` if a provider adapter is planned) - [packages/d2b-contracts-resource/src/v3/network.rs:956, packages/d2b-contracts-resource/src/v3/device.rs:129, packages/d2b-contracts-resource/src/v3/device.rs:151, packages/d2b-contracts-resource/src/v3/device.rs:760, packages/d2b-contracts-resource/src/v3/device.rs:918, packages/d2b-contracts-resource/src/v3/device.rs:1119]
  evidence: census `AttachmentSpec|AuthorityDescriptor|OpaqueAuthorityKey|DeviceStatus|DeviceRbacVerb|DeviceTelemetryLabels` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 6 hits, all the definitions themselves
- clean: seed 2 `pub .*\b(Arc|Rc|Box|RefCell)<` = 0 hits (the Arc fields in seal.rs are private); seed 3 `pub use` arms in operations/mod.rs are the house single-surface pattern; the wide wire export is the contract-crate norm (U1 (c) api false positive)

## err
- d2b-contracts-resource-p1#6 sev=medium blast=family effort=M verdict=actionable - `StoreErrorKind` (operations/error.rs:93-129) duplicates all 31 `ResourceErrorKind` variants and their `as_str` spellings verbatim, and d2b-resource-api/src/error.rs:11-56 `map_store_error_kind` re-lists all 31 a third time, so adding one resource-plane kind requires three synchronized edits and no test pins the overlap (each set only pins its own size) - fix: restructure `StoreErrorKind` as `Resource(ResourceErrorKind)` plus the three store-only variants (StoreIntegrityFailure, StoreBackpressure, StoreQuarantined), which collapses the map to one arm plus store arms while keeping `as_str` outputs identical - [packages/d2b-contracts-resource/src/v3/operations/error.rs:93, packages/d2b-contracts-resource/src/v3/operations/error.rs:132, packages/d2b-resource-api/src/error.rs:11]
  evidence: seed 4 `enum \w*Error` = 10 hits; census `StoreErrorKind` over packages/ = matches in d2b-resource-api (error.rs map, manager_backend.rs, service.rs), d2bd-runtime/src/guest_resource_runtime.rs, d2b-bus/src/session_seam_tests.rs; StoreError carries no serde derives, so the reshape is internal
- clean: seeds 1-3 = 271/9/0 hits; the 14 real-code expect sites are invariant-justified (validated CIDR internals network.rs:151-155, literal defaults network.rs:260/286/289, canonical-JSON serialization of validated values resource_schema.rs:189/396/562, static reason strings error.rs:230-232, fixed constructors device.rs:519 and host.rs:74); the 4 real `let _ =` sites are the compile-time capability assertions in seal.rs:218-224; no panic!/unreachable!/todo!/unimplemented! anywhere

## serde
- d2b-contracts-resource-p1#7 sev=medium blast=wide effort=S verdict=actionable - `ResourceError` derives Deserialize (error.rs:175-176), bypassing the invariants `ResourceError::new` enforces (current_revision only on ResourceConflict/AuthorizationDenied/RevisionExpired, retry_after_ms only with RetryClass::AfterDelay), so a wire error carrying an inconsistent combination deserializes into an illegal state that the retry decision logic then reads - fix: hand-write `Deserialize` for `ResourceError` through `Self::new`, matching every sibling wire type in this crate - [packages/d2b-contracts-resource/src/v3/error.rs:175, packages/d2b-contracts-resource/src/v3/error.rs:177]
  evidence: seed 1 `derive\([^)]*(De)?[Ss]erialize` = 85 hits; static read of error.rs:175-259; the retry fields are consumed by d2b-resource-client/src/dispatch.rs:270-273 (record_remote_error matches retry_class then retry_after_ms); census: no production JSON decode site for ResourceError exists today (the wire envelope is built by hand in d2bd-runtime/src/resource_runtime_support.rs:1627), so the bypass is latent on a public wire type
- d2b-contracts-resource-p1#8 sev=medium blast=family effort=S verdict=actionable - `PayloadSchema` derives Deserialize (payload_schema.rs:30-32), bypassing `PayloadSchema::parse`'s closed-object and writeOnly validation, and `CommandSpec::deserialize` (d2b-provider-command/src/command.rs:248-266) feeds the wire value straight in, so a wire Command carrying an open schema or a writeOnly property with a default deserializes as valid - fix: hand-write `Deserialize` for `PayloadSchema` through `Self::parse` (the wire shape is unchanged; producers already use parse) - [packages/d2b-contracts-resource/src/v3/payload_schema.rs:30, packages/d2b-contracts-resource/src/v3/payload_schema.rs:36]
  evidence: seed 1 = 85 hits; census `PayloadSchema` over packages/ = consumers d2b-provider-command/src/command.rs:185, d2b-provider-operation/src/operation.rs:458, d2bd/src/foundation_seed.rs:859-860 (which re-parses via parse, showing validation is expected); both spec types are wire shapes pinned in docs/reference/schemas/v3/
- clean: seeds 2-4 = 163/0/33 hits; the hand-written Deserialize impls (space-anchored seed misses the `impl<'de>` form; ~30 judged manually) are all Wire-struct admission gates calling the validated constructors, which is the recorded house pattern; optionality distinctions (skip_serializing_if vs RequiredNullable) are deliberate and golden-pinned

## obs
- N/A: seeds 1-4 = 0/0/0/0 hits (println, interpolated event macros, instrument, tracing/log all zero); Cargo.toml carries no tracing or log dependency, so the crate emits no telemetry

## docs
- d2b-contracts-resource-p1#9 sev=low blast=leaf effort=S verdict=actionable - limits.rs exports 30 `pub const` admission limits (lines 3-33) with no per-item docs and no rationale for the specific values (500, 100, 900000, 30000, 256 KiB, 4 MiB), so a reader cannot tell which bound is load-bearing - fix: add one-line doc comments naming the enforcing boundary (request admission, watch credits, deadline) or a module-level rationale paragraph - [packages/d2b-contracts-resource/src/v3/limits.rs:3, packages/d2b-contracts-resource/src/v3/limits.rs:22]
  evidence: seed 1 `^\s*pub (fn|struct|enum|trait|const|type)` = 523 hits; limits.rs has 30/30 pub consts undocumented apart from the one-line module doc
- d2b-contracts-resource-p1#10 sev=low blast=leaf effort=S verdict=actionable - the operations module exports pub accessors with no doc comments: `MutationOrdinal::get` (error.rs:21), `StoreSlot::get` (error.rs:50), the eight `StoreError` accessors (error.rs:262-291), `StoreSealIdentity::new/zone/slot` (seal.rs:48/63/67), `OpenedMutation::body/into_body` (seal.rs:121/125), `MutationSealAcceptor::diagnose/declared_slot` (seal.rs:177/181), and `PreparedStoreMutation::new/mutation/resource_uid/payload_digest` (mod.rs:378-400) - fix: add one-line doc comments, especially for the mutating builder `with_store_slot` and the consume-then-open capability methods - [packages/d2b-contracts-resource/src/v3/operations/error.rs:21, packages/d2b-contracts-resource/src/v3/operations/error.rs:262, packages/d2b-contracts-resource/src/v3/operations/seal.rs:48, packages/d2b-contracts-resource/src/v3/operations/seal.rs:121, packages/d2b-contracts-resource/src/v3/operations/mod.rs:378]
  evidence: seed 1 = 523 hits; these items carry no `///` at all while the surrounding contract types are otherwise documented to a high standard
- clean: seed 2 `/// # (Examples|Errors|Panics|Safety)` = 0 hits, but every Result-returning item documents its failure conditions in prose (the house style); seed 3 `-> Result<` = 127 hits; doc first sentences are strong and redaction behavior is documented on every redacted type

## perf
- d2b-contracts-resource-p1#11 sev=low blast=leaf effort=M verdict=actionable - `ResourceStatus::new` (resource_status.rs:636-658) serializes the complete status with `canonical_json_bytes(&value)` on every construction to enforce MAX_STATUS_BYTES, after `ensure_layer_size` already serialized the resource layer, so each status write pays two full serializations of the same object - fix: enforce the byte bound once at the write boundary (the caller already serializes for storage) or check the bound with a cheaper size pass; at minimum reuse the layer bytes from `ensure_layer_size` - [packages/d2b-contracts-resource/src/v3/resource_status.rs:636, packages/d2b-contracts-resource/src/v3/resource_status.rs:650]
  evidence: static (unmeasured); seed 1 `format!\(` = 75 hits (mostly tests and cold error paths), seed 2 collection-new = 35 hits (empty-case defaults), seed 3 `.to_string()` = 5 hits (all tests)
- clean: no format! or allocation in a loop in the part; digest rendering (resource_schema.rs:586-598) pre-sizes its String with with_capacity

## conc
- N/A: seeds 1-4 = 0/0/0/0 hits (no threads, Mutex/RwLock, atomics, or thread_local in the part)

## async
- N/A: seeds 1-4 = 0/0/0/0 hits (no async fn, spawn, tokio sync, or tokio attribute in the part)

## unsafe
- N/A: seeds 1-4 = 0/0/0/0 hits (no unsafe block, fn, impl, transmute, or raw-pointer construct in the part; `unsafe_code = "forbid"` via the workspace lints table)

## ffi
- N/A: seeds 1-4 = 0/0/0/0 hits (no extern "C", no_mangle, repr(C), or CStr/CString in the part)

## macro
- N/A: seeds 1-4 = 0/0/0/0 hits (no macro_rules! definitions in the part; the `redacted_debug!`/`parsed_deserialize!`/`string_schema!` invocations here are defined in v3/execution_policy.rs, part 2's scope)

## test
- clean: seeds 1-4 = 70/232/0/0 hits (test mass, assertion mass, no property/snapshot tooling, no ignored tests); the suite is golden-vector pinned (literal wire bytes in network.rs:1682, resource_schema.rs:1576-1587, volume_state.rs:590-591, host.rs:150-161), redaction-verified with process-id markers, table-driven with per-case failure messages, and the seal capability negative is enforced at compile time (seal.rs:214-225); tests/schema.rs exercises the public surface as a consumer would; no test that cannot fail was found

## Coverage
- idiom: 1 finding
- own: 1 finding
- type: 2 findings
- api: 1 finding
- err: 1 finding
- serde: 2 findings
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency in Cargo.toml)
- docs: 2 findings
- perf: 1 finding
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero)
- unsafe: N/A (seeds: 0/0/0/0 all zero)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 70/232/0/0)