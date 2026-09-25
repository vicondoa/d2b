# d2b-contracts-resource-p2 - d2b-contracts-resource - part 2/2
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 9703 (excl. src/generated/**) | modules: v3/volume.rs, v3/process.rs, v3/identity.rs, v3/execution_policy.rs, v3/resource.rs, v3/storage.rs, v3/volume_binding.rs, v3/activation_nixos.rs, v3/user.rs, v3/mod.rs, v3/artifact.rs, lib.rs (plus tests/schema.rs for the test lens)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 2/2 per U1 section f (file list above)

## idiom
- d2b-contracts-resource-p2#1 sev=medium blast=leaf effort=S verdict=actionable - the sort-dedup-compare uniqueness check is hand-rolled at three production sites while a private helper already exists - fix: extract `ensure_unique<T: Ord + Clone>(values: &[T]) -> Result<(), PrimitiveSpecError>` into execution_policy.rs (home of PrimitiveSpecError) and call it from VolumeSpec::new, ExecutionPolicy::new, and process.rs check_unique (which keeps only its max-bound check) - [packages/d2b-contracts-resource/src/v3/volume.rs:1210-1213, packages/d2b-contracts-resource/src/v3/volume.rs:1248-1253, packages/d2b-contracts-resource/src/v3/execution_policy.rs:792-796, packages/d2b-contracts-resource/src/v3/process.rs:1571-1579]
  evidence: static reading of the three sites plus the existing helper; seeds: `for \w+ in 0\.\.` = 0, `let mut \w+ = (String|Vec)::new\(\)` = 0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 7 (all Default impls preserving frozen defaults, deliberate)
- d2b-contracts-resource-p2#2 sev=low blast=leaf effort=S verdict=actionable - ResourceSpec::serialize iterates `self.base.keys()` and re-gets each key with an avoidable `expect("key returned by canonical object")` - fix: iterate `for (key, value) in &self.base` and call `map.serialize_entry(key, value)?`, deleting the expect and the double lookup - [packages/d2b-contracts-resource/src/v3/resource.rs:621-626]
  evidence: static reading; the map is a BTreeMap wrapper so pair iteration is a drop-in; seed `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 7
- d2b-contracts-resource-p2#3 sev=low blast=leaf effort=S verdict=actionable - VolumeSpec::new checks `views.contains_key` and then repeats the lookup with a dead `ok_or(MissingRequiredField)` that can never fire - fix: collapse to one `let view = views.get(attachment.view.as_str()).ok_or(PrimitiveSpecError::MissingRequiredField)?;` - [packages/d2b-contracts-resource/src/v3/volume.rs:1218-1223]
  evidence: static reading; the second `.ok_or` is unreachable after the `contains_key` early return; seeds: `for \w+ in 0\.\.` = 0, `let mut \w+ = (String|Vec)::new\(\)` = 0

## own
- clean: seeds ran: `\.clone\(\)` = 28, `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 42, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<|Cow<` = 0. Every clone inspected is explainable: `to_canonical_string` clones are the deliberate wire-rendering boundary (identity.rs:90, resource.rs:72), `views.keys()` clones feed the consuming `parse(impl Into<String>)` signature, and the rest are test fixtures; no Rc/RefCell/Arc/Cow anywhere.

## type
- d2b-contracts-resource-p2#4 sev=medium blast=leaf effort=S verdict=needs-contract - NixosGenerationStatus.observed_generation is a bare u64 while the crate already models exactly this value (zero meaning none) as ObservedGeneration in identity.rs - fix: replace the field type with `ObservedGeneration` (serde-transparent u64, same wire bytes and schemars shape) and update the accessor call sites - [packages/d2b-contracts-resource/src/v3/activation_nixos.rs:285, packages/d2b-contracts-resource/src/v3/identity.rs:595-606]
  evidence: static comparison with identity.rs ObservedGeneration whose doc states "zero meaning none", matching the field doc "Store generation revision observed by the controller"; seeds: `fn validate_\w+|fn check_\w+` = 13, `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 2 (both are validated-at-construction octal/path strings, not state)
- d2b-contracts-resource-p2#5 sev=medium blast=leaf effort=S verdict=needs-contract - ActivationRunnerInput.target_generation is a bare u64 carrying a manual zero check plus a hand-rolled `nonzero_u64_schema`, duplicating the nonzero-generation newtype the crate already generates - fix: use the `nonzero_generation!` macro output (e.g. ConfigurationGeneration, transparent u64 with JsonSchema minimum 1) for the field, deleting `ActivationRunnerInputError::GenerationInvalid`, the zero check in `new`, and `nonzero_u64_schema` - [packages/d2b-contracts-resource/src/v3/activation_nixos.rs:46-47, packages/d2b-contracts-resource/src/v3/activation_nixos.rs:59-63, packages/d2b-contracts-resource/src/v3/activation_nixos.rs:77-87, packages/d2b-contracts-resource/src/v3/identity.rs:448-472]
  evidence: static reading; the invariant (nonzero) is already enforced by the existing macro-generated types in identity.rs; wire bytes unchanged under serde-transparent; seeds: `fn validate_\w+|fn check_\w+` = 13, `(mode|kind|state): String` = 2

## api
- d2b-contracts-resource-p2#6 sev=low blast=leaf effort=S verdict=actionable - the exported type alias `ValidatedSessionPurpose` has zero callers anywhere in the workspace - fix: delete the alias (identity.rs:270) or document the intended consumer before it accrues surface - [packages/d2b-contracts-resource/src/v3/identity.rs:269-270]
  evidence: census: `ValidatedSessionPurpose` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 1 hit (the definition itself); seeds: `\bpub (fn|struct|enum|trait|type|const|mod) ` = 485, `pub .*\b(Arc|Rc|Box|RefCell)<` = 0, `^\s*pub use ` = 31 (house single-surface re-export arms, deliberate)

## err
- clean: seeds ran: `\.unwrap\(\)|\.expect\(|\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 288, `let _ = |\.ok\(\);` = 1, `enum \w*Error` = 6. Every unwrap/expect outside `#[cfg(test)]` sits on a frozen literal with a named invariant (`expect("strict is a valid token")` process.rs:237, `duration()` process.rs:1636, `system_default()` execution_policy.rs:844, `resource_type()` activation_nixos.rs:245, `UserSpec::minimal` user.rs:127, `ResourceSpec::empty()` resource.rs:543) or after a compiler-invisible byte check (`is_valid_timestamp` identity.rs:207); the single `let _ =` is the deliberate `$clear` flag in the `digest_identity!` redaction macro; the six error enums are field-free so rejection diagnostics never echo caller text.

## serde
- clean: seeds ran: `derive\([^)]*(De)?[Ss]erialize|serde\(...\)|impl .*Deserialize.*for|serde_json::from_|serde_json::to_` = 493. The hand-written `Deserialize` impls are all Wire-mirror admission gates (private-field struct, `deny_unknown_fields` Wire struct, `Self::new` validation mapped through `serde::de::Error::custom`) - the recorded house pattern per the refusal ledger (docs/explanation/over-engineering-audit-record.md, hand-written Deserialize admission-gate class); `#[serde(flatten)]` on ProcessSpec/EphemeralProcessSpec is serialization-only composition with the Wire mirror re-enabling `deny_unknown_fields`; enum representations are consistently external kebab-case/lowercase; optionality semantics (default vs Option vs skip_serializing_if) are consistent between the Serialize side and the Wire mirror on every checked type.

## obs
- clean: seeds ran: `\bprintln!\(|\beprintln!\(` = 0, `(info|debug|warn|error|trace)!\("` = 0, `\.instrument\(|#\[instrument` = 0, `tracing::|log::` = 0. The crate emits no telemetry at all; diagnostics are the redacted Debug/Display impls, which the tests pin.

## docs
- d2b-contracts-resource-p2#7 sev=medium blast=leaf effort=S verdict=actionable - artifact.rs is the only module with an undocumented public surface: MAX_ARTIFACT_ID_BYTES, ArtifactIdError::Invalid, ArtifactId, parse, and as_str all lack doc comments while every sibling module documents its pub items - fix: add one-line doc comments mirroring the BoundedToken contract (bounded lower-kebab artifact identifier, never a host path) - [packages/d2b-contracts-resource/src/v3/artifact.rs:5, packages/d2b-contracts-resource/src/v3/artifact.rs:7-10, packages/d2b-contracts-resource/src/v3/artifact.rs:22, packages/d2b-contracts-resource/src/v3/artifact.rs:25, packages/d2b-contracts-resource/src/v3/artifact.rs:35]
  evidence: seeds: `^\s*pub (fn|struct|enum|trait|const|type)` = 485, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 142; static comparison shows every other module documents its pub items (e.g. BoundedToken execution_policy.rs:186-191)
- d2b-contracts-resource-p2#8 sev=low blast=leaf effort=S verdict=actionable - two pub fns in execution_policy.rs lack doc comments: `ExecutionPolicyWire::into_policy` (pub because Host/Guest crates decode through the wire mirror) and `string_schema_object` (pub only for the exported `string_schema!` macro expansion) - fix: add one-line docs, noting for string_schema_object that it is macro-support surface - [packages/d2b-contracts-resource/src/v3/execution_policy.rs:923, packages/d2b-contracts-resource/src/v3/execution_policy.rs:944]
  evidence: seeds: `^\s*pub (fn|struct|enum|trait|const|type)` = 485, `/// # (Examples|Errors|Panics|Safety)` = 0; static reading of the two items

## perf
- clean: seeds ran: `format!\(` = 84, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 59, `\.to_string\(\)` = 8. The only non-test `format!` is `MilliCpu::to_canonical_string` (execution_policy.rs:363), a cold wire-rendering boundary; `TranscriptHash::to_hex` (identity.rs:660-668) pre-sizes with `String::with_capacity(64)`; `Vec::new()` hits are Default impls and test fixtures where the empty case is the common one; no hot path, no loop allocation, no attacker-keyed hashing (BTreeMap throughout).

## conc
- N/A (seeds: `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 0, `Atomic\w+|Ordering::` = 0, `thread_local!|unsafe impl (Send|Sync) for` = 0; all zero - the crate declares no threads, locks, atomics, or manual Send/Sync)

## async
- N/A (seeds: `async fn|async move|\.await` = 0, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 0, `tokio::sync::(Mutex|RwLock|Notify)` = 0, `#\[tokio::(main|test)\]|Runtime::block_on` = 0; all zero - the crate is synchronous contract types only)

## unsafe
- N/A (seeds: `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0, `unsafe_code` = 0; all zero - no unsafe blocks, fns, impls, or lint settings in the assigned files)

## ffi
- N/A (seeds: `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0; all zero - no foreign-language boundary)

## macro
- clean: seeds ran: `macro_rules!` = 7, `proc_macro|syn::|quote!` = 0, `\$crate` = 1, `to_compile_error|new_spanned` = 0. All seven macro_rules! definitions (label_identity, digest_identity, nonzero_generation in identity.rs; redacted_debug, parsed_deserialize, string_schema in execution_policy.rs; opaque_storage_id in storage.rs) are the genuine impl-per-type generation answer; the exported macros use fully-qualified paths and `$crate::v3::execution_policy::string_schema_object`, so hygiene holds; no proc macros exist.

## test
- clean: seeds ran: `#\[test\]|#\[tokio::test\]` = 75 (71 unit + 4 integration in tests/schema.rs), `assert_eq!\(|assert_ne!\(|assert!\(` = 299, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0. The suite is behavior-focused: golden byte vectors (MINIMAL_VOLUME_SPEC, MINIMAL_PROCESS_SPEC, GOLDEN_ENVELOPE, canonical base objects), round-trip tests, table-driven rejection tests with per-case messages, redaction tests with process-id markers, and schema-bound preservation tests; no test restates implementation and none is ignored.

## Coverage
- idiom: 3 finding(s)
- own: clean (seeds ran: 28/42/0)
- type: 2 finding(s)
- api: 1 finding(s)
- err: clean (seeds ran: 288/1/6)
- serde: clean (seeds ran: 493)
- obs: clean (seeds ran: 0/0/0/0)
- docs: 2 finding(s)
- perf: clean (seeds ran: 84/59/8)
- conc: N/A (seeds: 0/0/0/0 all zero; no threads, locks, atomics, or manual Send/Sync)
- async: N/A (seeds: 0/0/0/0 all zero; no async fn, await, tokio, or block_on)
- unsafe: N/A (seeds: 0/0/0/0 all zero; no unsafe blocks/fns/impls or lint settings)
- ffi: N/A (seeds: 0/0/0/0 all zero; no extern "C", no_mangle, repr(C), or CStr)
- macro: clean (seeds ran: 7/0/1/0)
- test: clean (seeds ran: 75/299/0/0)