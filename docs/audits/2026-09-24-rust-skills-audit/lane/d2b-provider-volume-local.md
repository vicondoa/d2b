# d2b-provider-volume-local - d2b-provider-volume-local
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 10328 (excl. src/generated/**; src 8056 + tests 2272) | modules: whole crate (adapter, acl, atomic, bindings, content, controller, diagnostics, effect_port, error, finalization, identity, layout, lock, marker, port, quota, source, status, store_view, testing, views, lib)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none (single-part lane)

## idiom
- d2b-provider-volume-local#1 sev=low blast=leaf effort=S verdict=actionable - LayoutPhase::worse hand-rolls severity comparison with an `as u8` cast although the enum derives PartialOrd/Ord; the cast also silently depends on variant declaration order matching severity order - fix: replace the `if self as u8 >= other as u8` body with `self.max(other)` (derived Ord, declaration order Pending/Ready/Degraded/Failed already encodes severity) - [src/status.rs:33-38]
  evidence: seeds: `for \w+ in 0\.\.` = 0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 0, `let mut \w+ = (String|Vec)::new\(\)` = 8 (all legitimate accumulation loops with side-effect bodies or Read::read_to_end targets; no index loops exist); finding from full-file read of src/status.rs
- clean: the 8 `let mut ... Vec::new()` sites (adapter.rs:1539,1677,1803; content.rs:505,754; controller.rs:215,370; diagnostics/storage_lifecycle.rs:48) are the canonical read-to-end or side-effecting accumulation shapes the skill itself prefers over combinator chains; no hand-written derive candidates and no index loops found

## own
- clean: seeds: `.clone\(\)` = 87, `.to_owned\(\)|.to_vec\(\)|.to_string\(\)` = 26, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 3, `Cow<` = 0; every clone inspected is a field copy from a borrowed reference into an owned struct (bindings.rs:93-99, content.rs:410-422, views.rs:103-108, marker.rs:88-93), a shared-ownership Arc (adapter.rs:181-204 Arc<dyn VolumeRootResolver> delegation impl and adapter.rs:209 Arc<OwnedFd> in FdRootResolver, both required for the Send+Sync resolver seam), or test-fixture state (testing.rs:81 Mutex<Vec<PortCall>>); no borrow-checker-silencing clones found

## type
- d2b-provider-volume-local#2 sev=low blast=leaf effort=M verdict=actionable - VolumeRootHandle carries ten Option fields that are valid only all-Some (from_anchored) or all-None (held), so eleven mixed states are representable; construction is internal today so the mixed states are unreachable, but the fail-closed root-identity handle is exactly where a future partial-construction bug would land - fix: split into a two-variant enum (e.g. `enum VolumeRootHandle { Empty, Anchored(AnchoredHandle) }`) or a typestate pair, keeping the non-Clone/non-Serialize property - [src/identity.rs:72-88, src/identity.rs:146-150]
  evidence: seeds: `fn validate_\w+|fn check_\w+` = 10, `is_\w+: bool|\w+_flag: bool` = 1, `(mode|kind|state): String` = 5; the single bool (controller.rs:41 watched_configuration_is_dependency) and the five `mode: String` fields (content.rs:112,315,382,545,699) are not findings: the bool is a lone flag and the mode strings are schema-mirroring fields validated at construction (ContentFile::validate, content.rs:142-153) - the only illegal-state candidate is the handle
- clean: validate-at-callsite is confined to the wire boundary (validate_source_spec, controller validate_spec, EntryRequest::resolve) where the VolumeSpec contract type gives no guarantees, which is the parse-once pattern rather than a violation

## api
- d2b-provider-volume-local#3 sev=medium blast=family effort=S verdict=actionable - `pub mod testing` (ScriptedPort with a Mutex, fixtures, hand-rolled block_on) is compiled unconditionally into the production library although it is consumed only by tests: this crate's tests/** and one d2bd test fn; the house pattern for cross-crate test support is a feature gate - fix: gate the module behind a `test-support` feature (`#[cfg(feature = "test-support")]` on `pub mod testing`, add `[features] test-support = []`), and enable the feature from d2bd's dev-dependencies - [src/lib.rs:82, src/testing.rs:1-402, packages/d2b-provider-volume-local/Cargo.toml:1]
  evidence: census: `volume_local::testing|ScriptedPort` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel = 26 hits, all in this crate's tests/** (layout_conformance.rs:9, store_view_and_swtpm.rs:3, views_and_sharing.rs:5, volume_effect_adapter.rs:122-493) and d2bd/src/resource_runtime.rs:12917 (inside a #[test] fn); no production code path imports it
- clean: api seed 2 (`pub .*\b(Arc|Rc|Box|RefCell)<`) = 0; the lib.rs `pub use` arms are the house single-surface pattern; public signatures carry only std types (BorrowedFd/OwnedFd in VolumeRootHandleView/AnchoredRoot, identity.rs:90-117) or contract-crate types; the adapter module double-path (pub mod adapter + root re-export) is referenced by cross-crate intra-doc links (d2b-provider-volume/src/facets.rs:18,50) and is covered by the re-export-arm false-positive note, so not flagged

## err
- clean: seeds: `\.unwrap\(\)|\.expect\(` = 121 (every hit outside #[cfg(test)] is controller.rs:88 `BoundedToken::parse("volume-local").expect("frozen provider name")` on a literally-built value, the sanctioned class), `let _ = |\.ok\(\);` = 4 (adapter.rs:313 stub arg ignore, adapter.rs:1546 unused-arg ignore, adapter.rs:1778 best-effort unlinkat in remove_temp, diagnostics/storage_lifecycle.rs:110 drop cleanup - all deliberate), `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 0, `enum \w*Error` = 5; the five error enums (VolumeLocalError, AtomicWriteError, LockError, MarkerError, VolumeEffectError) are closed sets with stable lower-kebab `code()` accessors and Display rendering the code, matching the wire-code discipline; VolumeLocalError::ALL (29) matches the code() match arms

## serde
- d2b-provider-volume-local#4 sev=medium blast=leaf effort=M verdict=actionable - ContentFile, ContentProjection, NetworkConfigContentProjection and the evidence types derive public Deserialize that bypasses the validating constructors: the crate's parse boundary is `from_value`/`from_settings` (which run validate), but the derived impl admits unvalidated projections directly, so the type the rest of the program trusts is not guaranteed valid on the derive path - fix: route the derive through `#[serde(try_from = "Raw...")]` mirror structs (wire shape unchanged: camelCase + deny_unknown_fields preserved) or drop Deserialize from the derives and parse only via the validating entries - [src/content.rs:38-39, 106-107, 203-204, 239-243, 536-537, 590-594]
  evidence: seeds: `derive\([^)]*(De)?[Ss]erialize` = 24, `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` = 19 (no try_from anywhere), `impl .*Deserialize.* for` = 0, `serde_json::from_|serde_json::to_` = 66; in-repo consumers all use the validating entries (d2bd/src/shared_provider_effects.rs:676,732 from_settings; d2bd/src/resource_plane_v3.rs:1507-1509 constructors), so the gap is the public derive itself
- clean: rename_all camelCase/kebab-case conventions are consistent per type family; deny_unknown_fields is present on every wire-mirroring struct; skip_serializing_if used correctly (status.rs:87); no hand-written Deserialize impls (the recorded-refusal admission-gate class does not appear here)

## obs
- clean: seeds: `\bprintln!\(|\beprintln!\(` = 0, `(info|debug|warn|error|trace)!\("` = 0, `\.instrument\(|#\[instrument` = 0, `tracing::` = 23; every tracing event carries named fields (volume, path, error, reason, drift, entry, provider) with message-only text, e.g. adapter.rs:332-335, 740-743, controller.rs:232-245, lock.rs:250-253; redacted Debug impls (ContentFile, EntryDigest, VolumeRootHandle, SourcePolicyCatalog, VolumeRootIdentity, LockId) keep identifiers out of any field rendering; no secrets in fields

## docs
- d2b-provider-volume-local#5 sev=low blast=leaf effort=M verdict=actionable - no canonical doc sections exist anywhere in the crate (seed 2 = 0 hits): public Result-returning items such as ContentFile::new, ContentProjection::new/from_value, EntryRequest::resolve, VolumeLocalController::reconcile, admit_attachments and validate_source_spec carry one-line docs but no `# Errors` section naming which conditions produce which failure - fix: add `# Errors` sections to the admission/parse constructors and the controller entry points, listing the closed VolumeLocalError variants each can return - [src/content.rs:118-125, src/controller.rs:148-154, src/layout.rs:54-57, src/views.rs:88-92, src/source.rs:130-131]
  evidence: seeds: `^\s*pub (fn|struct|enum|trait|const|type)` = 319, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 104; `#![deny(missing_docs)]` (src/lib.rs:18) is already enforced so every public item has a first sentence; the gap is the canonical-sections shape only

## perf
- clean: seeds: `format!\(` = 19, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 9, `\.to_string\(\)` = 26; every hit is a cold path (lock-id/temp-name/mode rendering at adapter.rs:1054,1706,1814; digest hex at content.rs:495-501,773; mount options at source.rs:253; test fixtures), a canonical read_to_end target, or wire-rendering; no format!/allocation inside any loop that runs per-entry on a hot reconcile path beyond the bounded digest preimage builders (content.rs:505,754, bounded by MAX_CONTENT_BYTES); static (unmeasured)

## conc
- clean: seeds: `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 1, `Atomic\w+|Ordering::` = 3, `thread_local!|unsafe impl (Send|Sync) for` = 0; the only production primitive is `static NEXT_TEMP: AtomicU64` with `fetch_add(1, Ordering::Relaxed)` (adapter.rs:1705,1710) - a counter nobody synchronizes on, so Relaxed is the weakest correct ordering and the static is justified for cross-instance temp-name uniqueness; the single Mutex (testing.rs:81) is test-fixture state

## async
- clean: seeds: `async fn|async move|\.await` = 60, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 0, `tokio::sync::(Mutex|RwLock|Notify)` = 0, `#\[tokio::(main|test)\]|Runtime::block_on` = 0; the adapter's synchronous filesystem work runs at future-construction inside the async port methods (adapter.rs:327-369 `let result = self.observe_sync(...); async move { result }`), but every such site carries the sanctioned per-site allow `#[allow(clippy::disallowed_methods, reason = "synchronous path")]` (adapter.rs:1666, 1520) and the crate is in the blocking census baseline (packages/xtask/data/blocking-census-baseline.json, all-zero counts), so the sync-in-async shape is recorded policy, not a new finding; the crate owns no runtime (testing.rs:29-33 hand-rolled block_on is the deliberate no-runtime design); controller awaits only port calls

## unsafe
- clean: seeds: `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 22, `unsafe_code` = 1; the seed-3 hits are all false positives: `Mode::from_raw_mode`/`FileType::from_raw_mode` are rustix safe constructors (adapter.rs:673,696,932,1048,1107,1115,1235,1253,1530,1566,1571,1716,1743) and `MaybeUninit` appears only as a stack buffer handed to rustix RawDir without any unsafe access (adapter.rs:16,1468); the manifest forbids unsafe_code (Cargo.toml `[lints.rust]`) and no `unsafe_code = "allow"` exists, so the crate is outside the U1 (d)8 exception set

## ffi
- N/A: seeds: `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0; the crate crosses no foreign-language boundary (libc::flock struct literals at adapter.rs:1604-1610,1623-1629 are data passed to rustix's fcntl wrapper, not extern declarations)

## macro
- N/A: seeds: `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0; no macros are defined or used beyond std ones; the crate's repetition is handled by traits and generics

## test
- clean: seeds (src + tests): `#\[test\]|#\[tokio::test\]` = 59, `assert_eq!\(|assert_ne!\(|assert!\(` = 300, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0; the suite asserts behavior and error variants rather than Display strings (e.g. tests/layout_conformance.rs asserts `Err(VolumeLocalError::EntryDrift)` and ConditionSeverity; tests/volume_effect_adapter.rs pins foreign-marker preservation and quarantine non-mutation with readback assertions), is table-driven with per-case messages (adapter.rs:1856-1860, tests/layout_conformance.rs:140-153), and is deterministic (tempdirs under CARGO_TARGET_TMPDIR, no network, no wall-clock dependence); no test computes its expectation with the code under test (the bindings.rs reordering test compares forward vs reordered-spec output, which is the determinism property itself); no ignored or unfailable tests found

## Coverage
- idiom: 1 finding
- own: clean (seeds ran: 87/26/3/0)
- type: 1 finding
- api: 1 finding
- err: clean (seeds ran: 121/4/0/5)
- serde: 1 finding
- obs: clean (seeds ran: 0/0/0/23)
- docs: 1 finding
- perf: clean (seeds ran: 19/9/26)
- conc: clean (seeds ran: 0/1/3/0)
- async: clean (seeds ran: 60/0/0/0)
- unsafe: clean (seeds ran: 0/0/22/1; all seed-3 hits are from_raw_mode/MaybeUninit false positives, manifest forbids unsafe_code)
- ffi: N/A (seeds: 0/0/0/0 all zero; no foreign boundary)
- macro: N/A (seeds: 0/0/0/0 all zero; no macro definitions)
- test: clean (seeds ran: 59/300/0/0)