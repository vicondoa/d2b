# d2b-contracts-provider-p1 - d2b-contracts-provider - part 1/2
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 7304 (excl. src/generated/**) | modules: v3/provider.rs, v3/credential.rs, v3/credential/service.rs, v3/provider_registry.rs, v3/mod.rs, lib.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 1/2 per U1 (f): src/v3/provider.rs, src/v3/credential/**, src/v3/credential.rs, src/v3/provider_registry.rs, src/v3/mod.rs, src/lib.rs

## idiom
- d2b-contracts-provider-p1#1 sev=low blast=leaf effort=S verdict=actionable - hand-written `Default` impls on `CredentialRotationPolicy` and `CredentialRevocationPolicy` reproduce the field-wise default a derive would generate - fix: add `#[default]` to `RotationPolicyClass::OnExpiry` and `RevocationAction::Immediate` and replace both impls with `#[derive(Default)]` - [packages/d2b-contracts-provider/src/v3/credential.rs:362, packages/d2b-contracts-provider/src/v3/credential.rs:453]
  evidence: seed `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 5 hits in lane; 2 are derive-replaceable Defaults (the Clone/PartialEq/Eq on CredentialAuthorization at service.rs:661-687 are required because of the `Arc<dyn Any>` field and are not findings)
- d2b-contracts-provider-p1#2 sev=low blast=leaf effort=S verdict=actionable - manual `Debug` impl on `UpgradePolicy` prints exactly the three closed pub fields a derive would print, with nothing to redact - fix: replace `impl core::fmt::Debug for UpgradePolicy` with `#[derive(Debug)]` on the struct - [packages/d2b-contracts-provider/src/v3/provider.rs:2374]
  evidence: seed `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 5 hits; the other manual Debug impls in the lane redact caller-supplied values (module rule at provider.rs:12-22, deliberate) and are not findings

## own
- d2b-contracts-provider-p1#3 sev=low blast=leaf effort=S verdict=actionable - `ProviderManifest::validate_runtime_artifacts` takes `impl IntoIterator<Item = TargetRuntimeArtifacts>` by value, forcing `entries.clone()` and `self.runtime_artifacts.clone()` at both call sites that already hold the vec - fix: change the signature to `entries: &[TargetRuntimeArtifacts]` and drop both clones (census shows no external callers, so the pub signature change is contained) - [packages/d2b-contracts-provider/src/v3/provider.rs:2497, packages/d2b-contracts-provider/src/v3/provider.rs:2531, packages/d2b-contracts-provider/src/v3/provider.rs:2580]
  evidence: seed `.clone()` = 26 hits in lane; census: `validate_runtime_artifacts` over packages/ + nixos-modules/ + tests/ + docs/reference/ + labs/ = 4 hits, all inside provider.rs (2497, 2531, 2580, 3224 test)
- d2b-contracts-provider-p1#4 sev=low blast=leaf effort=S verdict=actionable - `ProviderManifest::new` and `ComponentDescriptor::with_state_namespaces` clone identifiers into dedup sets that could borrow - fix: use `BTreeSet<&BoundedToken>` / `BTreeSet<&ResourceTypeName>` for `component_ids`, `owned_types`, `bound_types`, and `ids` - [packages/d2b-contracts-provider/src/v3/provider.rs:2438, packages/d2b-contracts-provider/src/v3/provider.rs:2445, packages/d2b-contracts-provider/src/v3/provider.rs:2455, packages/d2b-contracts-provider/src/v3/provider.rs:1459]
  evidence: seed `.clone()` = 26 hits in lane; the remaining clones are required (owned projection return at provider.rs:1169-1182, `Arc` proof clone at service.rs:664-668, test fixtures) and are not findings

## type
- d2b-contracts-provider-p1#5 sev=low blast=family effort=M verdict=actionable - `ComponentDescriptor::new` takes a `declares_state_volume: bool` parameter that can only be `false`: `true` is rejected at the top of the constructor, the Deserialize path passes the literal `false`, and the flag is only ever set through `with_state_namespaces` - fix: remove the parameter from `ComponentDescriptor::new` and update the ~20 call sites (census below), keeping the wire-only `declaresStateVolume` field and its consistency check inside the Deserialize Wire struct - [packages/d2b-contracts-provider/src/v3/provider.rs:1366, packages/d2b-contracts-provider/src/v3/provider.rs:1368, packages/d2b-contracts-provider/src/v3/provider.rs:1720, packages/d2b-contracts-provider/src/v3/provider.rs:3455]
  evidence: seed `fn validate_\w+|fn check_\w+` = 8 hits; census: `ComponentDescriptor::new` over packages/ + tests/ = ~20 hits across 7 crates (d2b-bus, d2b-core-controller, d2b-provider-provider, d2b-provider-toolkit, d2bd-runtime, d2bd, d2b-resource-compiler), all passing `false` or relying on the default path
- d2b-contracts-provider-p1#6 sev=low blast=leaf effort=M verdict=actionable - `ComponentDescriptor` stores `execution` and `execution_wire` as parallel fields where the wire shape is derived from the enum, so one fact has two representations that only `with_execution` keeps in sync - fix: implement `Serialize` for `ComponentExecution` emitting the flat `binaryRef` key (absent for `InProcessBootstrap`), drop the `execution_wire` field and the private `ComponentExecutionWire` struct - [packages/d2b-contracts-provider/src/v3/provider.rs:1333, packages/d2b-contracts-provider/src/v3/provider.rs:1335, packages/d2b-contracts-provider/src/v3/provider.rs:1418]
  evidence: seed `fn validate_\w+|fn check_\w+` = 8 hits; the redundant pair is visible in the struct literal at provider.rs:1418-1419 and the From bridge at provider.rs:1300-1307

## api
- d2b-contracts-provider-p1#7 sev=low blast=family effort=S verdict=actionable - `SchemaVersion` in d2b-contracts-resource exposes no `major()`/`minor()` accessors, so `CompatibilityRange::admits_state` re-parses the canonical string in `schema_version_parts` with three `expect`s and an allocation per call - fix: add `pub const fn major(self) -> u32` and `minor(self) -> u32` to `SchemaVersion` (non-breaking) and delete `schema_version_parts` - [packages/d2b-contracts-provider/src/v3/provider.rs:568, packages/d2b-contracts-resource/src/v3/resource_schema.rs:592]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = ~120 hits; the expects are invariant-justified (U1 (c) err false-positive class), so the finding is the missing accessor, not the panics

## err
- d2b-contracts-provider-p1#8 sev=medium blast=leaf effort=S verdict=actionable - `ProviderRegistryPublication::new` maps `generation == 0` to `MappingBoundExceeded` even though the `ZeroGeneration` variant exists and is used by the entry constructor, so a caller distinguishing invalid generation from bound overflow receives the wrong code - fix: split the check into `if generation.get() == 0 { return Err(ZeroGeneration) }` before the mapping-count bound - [packages/d2b-contracts-provider/src/v3/provider_registry.rs:186, packages/d2b-contracts-provider/src/v3/provider_registry.rs:92]
  evidence: seed `enum \w*Error` = 2 hits in lane; the variant pair is visible at provider_registry.rs:40 (ZeroGeneration) and provider_registry.rs:48 (MappingBoundExceeded); no external matchers exist (census below in #9)
- d2b-contracts-provider-p1#9 sev=low blast=leaf effort=S verdict=actionable - an entry-generation mismatch in `ProviderRegistryPublication::new` reports `AxisMismatch`, whose Display code is `provider-registry-axis-mismatch`, although no binding axis is involved - fix: add a `GenerationMismatch` variant with its own kebab code and return it for the `entry.provider_generation != generation` check - [packages/d2b-contracts-provider/src/v3/provider_registry.rs:189, packages/d2b-contracts-provider/src/v3/provider_registry.rs:193]
  evidence: seed `enum \w*Error` = 2 hits; census: `ProviderRegistryError` over packages/ + nixos-modules/ + tests/ + docs/reference/ + labs/ = 12 hits, all inside provider_registry.rs, so adding a variant breaks no external exhaustive match

## serde
- clean: seeds ran: derive Serialize/Deserialize ~41 hits, serde attrs ~30 hits, hand-written `impl Deserialize` 18 hits, serde_json 0 production hits; the hand-written Deserialize impls are live admission gates (recorded refusal class per U1 (d)6, not re-flagged), `rename_all` conventions are consistent, and every Wire admission shape carries `deny_unknown_fields`; the PascalCase wire spellings of `CredentialLeaseState`, `CredentialConditionType`, and `CredentialInteractionState` are pinned by the golden vector at credential.rs:1106 and are deliberate

## obs
- clean: seeds ran: println!/eprintln! 0, event-macro pattern 17 hits (all `redacted_debug!` macro-name false positives), `.instrument`/`#[instrument]` 0, tracing::/log:: 0; the crate emits no telemetry and every Debug/Display surface redacts caller-supplied values (module rule provider.rs:12-22), which the ADR 0010/0028 redaction gate covers

## docs
- d2b-contracts-provider-p1#10 sev=low blast=leaf effort=M verdict=actionable - no canonical `# Errors` sections exist on any Result-returning pub item in the lane even though failure conditions are the load-bearing part of these admission constructors - fix: add `# Errors` sections naming the closed variants (e.g. `ProviderContractError::InvalidPrimitive` for `BinaryRef::parse`, `ProviderContractError::TrustNotEstablished` for `TrustEvidence::admit`) to the pub constructors and admission methods - [packages/d2b-contracts-provider/src/v3/provider.rs:250, packages/d2b-contracts-provider/src/v3/provider.rs:481, packages/d2b-contracts-provider/src/v3/credential.rs:120, packages/d2b-contracts-provider/src/v3/credential/service.rs:1002]
  evidence: seed `/// # (Examples|Errors|Panics|Safety)` = 0 hits vs seed `-> Result<` = ~80 hits in lane; every pub item is otherwise documented (first sentences are contract-shaped), so this is a section-shape gap, not missing docs

## perf
- clean: seeds ran: format! 13 hits (11 in tests, 2 cold: opaque_digest credential.rs:80 and the test-adjacent fingerprint helper), Vec::new 5 hits (encode_outer service.rs:1003 and write_message service.rs:1386 are cold per-operation paths; constructor empties are the common case), to_string 5 hits (all tests); no hot loop allocates, and all findings here would be static (unmeasured) per the perf gate

## conc
- d2b-contracts-provider-p1#11 sev=low blast=leaf effort=S verdict=actionable - `SensitiveDeliveryRecord` uses `Ordering::SeqCst` for per-byte loads and stores that have no release/acquire pairing with any other atomic, so the strongest ordering buys nothing - fix: use `Ordering::Relaxed` in `copy_to`, `clear`, and `is_zeroized` - [packages/d2b-contracts-provider/src/v3/credential/service.rs:955, packages/d2b-contracts-provider/src/v3/credential/service.rs:963, packages/d2b-contracts-provider/src/v3/credential/service.rs:977]
  evidence: seed `Atomic\w+|Ordering::` = 7 hits in lane, all on this one record type (import at service.rs:6 plus 6 uses); the `Arc<dyn Any>` proof in `CredentialAuthorization` is genuine shared ownership (U1 (c) api false-positive class) and is not a finding

## async
- clean: seeds ran: async fn/async move/.await 3 hits (dispatch_async service.rs:867, dispatch_authorized_provider_async service.rs:896, .await service.rs:904), tokio::spawn/spawn_blocking/JoinSet/select!/join! 0, tokio::sync 0, #[tokio::main/test] 0; the `#[async_trait]` trait has one required sync method plus a default async wrapper that does no blocking work, holds no locks across `.await`, and is runtime-agnostic

## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` 0, `// SAFETY:` 0, `transmute|from_raw|MaybeUninit|mem::zeroed` 0, `unsafe_code` 0 in lane; workspace lints forbid unsafe (U1 (d)1) and the crate is not on the (d)8 exception list

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` 0, `catch_unwind` 0, `repr\(C\)|repr\(transparent\)` 0, `CStr|CString|c_char` 0; the crate crosses no foreign boundary

## macro
- clean: seeds ran: macro_rules! 1 hit (opaque_credential_value! credential.rs:111), proc_macro/syn/quote 0, $crate 0, to_compile_error/new_spanned 0; the single macro is a genuine impl-per-type generator with narrow fragment specifiers ($name:ident, $max:expr, $domain:literal, $doc:literal) and is module-local, so no `$crate` path is needed; no proc-macro and no trybuild suite are warranted at this size

## test
- d2b-contracts-provider-p1#12 sev=medium blast=leaf effort=M verdict=actionable - `credential/service.rs` (1461 lines) contains zero tests: the strict protobuf codec (the five `CredentialWire` impls at service.rs:1071-1350, `WireReader` at service.rs:1393-1452, `set_once` duplicate-field rejection at service.rs:1296, and the `encode_outer`/`decode_outer` ceilings at service.rs:1002-1028) is entirely unverified, so a malformed-input, truncation, or non-canonical-varint regression passes the suite silently - fix: add round-trip tests per DTO plus malformed/truncated/duplicate-field/non-canonical-varint/oversize tests for `WireReader` and `encode_outer`/`decode_outer` - [packages/d2b-contracts-provider/src/v3/credential/service.rs:1071, packages/d2b-contracts-provider/src/v3/credential/service.rs:1393, packages/d2b-contracts-provider/src/v3/credential/service.rs:1296]
  evidence: seed `#\[test\]|#\[tokio::test\]` = 54 hits in lane, 0 of them in service.rs (43 in provider.rs, 9 in credential.rs, 2 in provider_registry.rs); the sibling files' tests are behavior-focused (schema vectors, fail-closed checks, redaction canaries) and no `#[ignore]` or tautological tests were found

## Coverage
- idiom: 2 finding(s)
- own: 2 finding(s)
- type: 2 finding(s)
- api: 1 finding(s)
- err: 2 finding(s)
- serde: clean (seeds ran: 41/30/18/0; hand-written Deserialize = recorded admission-gate class, U1 (d)6)
- obs: clean (seeds ran: 0/17/0/0; the 17 event-macro hits are `redacted_debug!` name matches)
- docs: 1 finding(s)
- perf: clean (seeds ran: 13/5/5; all hits cold or test-only)
- conc: 1 finding(s)
- async: clean (seeds ran: 3/0/0/0)
- unsafe: N/A (seeds: 0/0/0/0 all zero; unsafe_code forbid per workspace lints, U1 (d)1)
- ffi: N/A (seeds: 0/0/0/0 all zero; no foreign boundary)
- macro: clean (seeds ran: 1/0/0/0)
- test: 1 finding(s)