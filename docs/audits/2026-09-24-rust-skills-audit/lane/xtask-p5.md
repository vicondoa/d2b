# xtask-p5 - xtask - part 5/5
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 9009 (excl. src/generated/**) | modules: changelog.rs, async_gate.rs, blocking_census.rs, delivery/evidence.rs, nix_inventories.rs, bazel_evidence.rs, delivery/seal.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: src/changelog.rs, src/async_gate.rs, src/blocking_census.rs, src/delivery/evidence.rs, src/nix_inventories.rs, src/bazel_evidence.rs, src/delivery/seal.rs

## idiom
- xtask-p5#1 sev=low blast=leaf effort=S verdict=actionable - `resource_type.to_string()` in an iterator map over `&[String]` where `.cloned()` is the idiomatic copy - fix: `STANDARD_RESOURCE_TYPES.iter().map(|resource_type| resource_type.to_string()).collect::<Vec<_>>()` -> `.iter().cloned().collect::<Vec<_>>()` - [packages/xtask/src/nix_inventories.rs:721]
  evidence: seed `\.to_string\(\)|\.to_owned\(\)|\.to_vec\(\)` = ~96 hits (lane); the cited site is the only production iterator-map copy in this part; the rest are tests and error-path strings

## own
- xtask-p5#2 sev=low blast=leaf effort=M verdict=actionable - Baseline map build clones each `CrateCensus`'s `crate_dir` and `counts` twice per crate although the later baseline check only re-borrows them - fix: build `CensusBaseline` from `crates.into_iter().map(|c| (c.crate_dir, c.counts)).collect()` (when `json_out` is set( and drive the `--baseline` check loop from `&baseline.crates` instead of `&crates` - [packages/xtask/src/blocking_census.rs:1270, packages/xtask/src/blocking_census.rs:1287]
  evidence: seed `\.clone\(\)` = ~32 hits; at blocking_census.rs:1270-1272 the pair clone fires twice per crate into the committed-baseline map; every other clone in this part buys an owned value whose borrower stays live

## type
- clean: seeds ran: `fn validate_\w+|fn check_\w+` = 3 / `is_\w+: bool|\w+_flag: bool` = 0 / `(mode|kind|state): String` = 0; the three hits (`validate_inventory`, `check_security`, `validate_single_line`) are boundary validators in a CLI/gate context where parse-once newtypes would be over-engineering per the stopping rule

## api
- clean: seeds ran: `\bpub (fn|struct|enum|trait|type|const|mod) ` = ~66 / `pub .*\b(Arc|Rc|Box|RefCell)<` = 0 / `^\s*pub use ` = 0; all pub items checked; xtask is a bin-only crate (no `lib` target, `publish = false`), so every `pub` item is crate-internal surface, and the pub fields on delivery records serve sibling-module construction

## err
- clean: seeds ran: `\.unwrap\(\)|\.expect\(` ~95 (production hits ~12, every one an invariant assert on a compiler-verified or pre-checked value - `String::from_utf8(out).expect)...)` async_gate.rs:824, `serde_json::to_string_pretty(&value).expect)...)` bazel_evidence.rs:41, allocation `.get)...).expect("...checked against the allocation")` nix_inventories.rs:620,641; the rest are `#[cfg(test)]`( / `let _ = |\.ok\(\);` ~16 (test fixture strings and deliberate best-effort `Drop` cleanup in changelog tests( / `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 1 (test( / `enum \w*Error` = 0; panic policy is sound; no swallowed Results in production paths

## serde
- clean: seeds ran: `derive\([^)]*(De)?[Ss]erialize` = 13 / `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` = 9 / `impl .*Deserialize.*for` = 0 / `serde_json::from_|serde_json::to_` ~18; wire record types (`HatchInventory`, `CensusBaseline`, `EvidenceRecord`, `OutputDigest`, `SealedLane`, `SealedValidation`, `SealRecord`) usa `rename_all = "kebab-case"` for the lane enum, `deny_unknown_fields` on the committed record shapes, and `#[serde(default, skip_serializing_if = "Option::is_none")]` on optional record fields; no hand-written deserializers and no missing-boundary validation identified

## obs
- clean: seeds ran: `\bprintln!\(|\beprintln!\(` ~31 (all CLI product output of changelog-fold, check-async-gate, blocking-census, and bazel-evidence subcommands - the card's sanctioned xtask case( / `(info|debug|warn|error|trace)!\("` = 0 / `\.instrument\(|#\[instrument` = 0 / `tracing::|log::` = 0; no telemetry or event logging in this part

## docs
- xtask-p5#3 sev=medium blast=leaf effort=M verdict=actionable - Pub field groups on the wire and census record types carry no field-level doc contracts, so units and serialization formats are guesswork - fix: add per-field doc comments to `DeniedApi.path/tail/kind`, `CensusBaseline.crates`, `OutputDigest.sha256/bytes`, `EvidenceRecord.*`, `SealedLane.lane/validations`, `SealedValidation.validation/record_sha256`, `SealRecord.*` - [packages/xtask/src/blocking_census.rs:71, packages/xtask/src/blocking_census.rs:658, packages/xtask/src/delivery/evidence.rs:120, packages/xtask/src/delivery/evidence.rs:128, packages/xtask/src/delivery/seal.rs:35, packages/xtask/src/delivery/seal.rs:48, packages/xtask/src/delivery/seal.rs:61]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = ~66 hits; at the cited records the pub fields lack docs for `sha256` (hex? base64?), `imported_at_unix` (seconds?), `schema_version` semantics,and map-key forms
- xtask-p5#4 sev=low blast=leaf effort=S verdict=actionable - Result-returning pub fns describe failure modes in prose rather than the canonical `# Errors` section - fix: add `# Errors` sections to `parse_fragment`, `EvidenceLane::parse`, `EvidenceRecord::validate`, `SealRecord::validate`, and `async_gate::scan_source` naming each rejection condition - [packages/xtask/src/changelog.rs:149, packages/xtask/src/delivery/evidence.rs:97, packages/xtask/src/delivery/evidence.rs:162, packages/xtask/src/delivery/seal.rs:77, packages/xtask/src/async_gate.rs:265]
  evidence: seed `-> Result<` ~68 hits; the cited pub fns carry prose rejection lists (e.g. "Rejected: an empty fragment, an unknown...") where the skill's canonical-section shape is absent

## perf
- xtask-p5#5 sev=low blast=leaf effort=S verdict=actionable - `contains_quoted_field` allocates two `format!`'d quoted literals per field per quote inside the per-line redaction scan, up to 8 small String allocations per log line - fix: frame the four credential field names once per `redact_text` call (or as module `const` literals( and pass `&[&str]` framed forms to `contains_quoted_field` so the per-line scan only does `.contains)...)` - [packages/xtask/src/bazel_evidence.rs:397, packages/xtask/src/bazel_evidence.rs:399, packages/xtask/src/bazel_evidence.rs:407]
  evidence: static (unmeasured); seed `format!\(` ~45 hits over the lane;(the other format sites are error paths or deliberate artifact-text generation, which the card exempts; the cited site allocates inside a per-line loop over a potentially large build log

## conc
- clean: seeds ran: `std::thread::|thread::spawn|thread::scope` ~8 / `\bMutex<|\bRwLock<` ~13 / `Atomic\w+|Ordering::` = 0 / `thread_local!|unsafe impl (Send|Sync) for` = 0; every hit is doc prose or a test-fixture source string inside scanner/gate modules; no threads, locks, atomics, or manual Send/Sync claims exist in real code of this part

## async
- clean: seeds ran: `async fn|async move|\.await` ~40 / `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` ~8 / `tokio::sync::(Mutex|RwLock|Notify)` ~3 / `#\[tokio::(main|test)\]|Runtime::block_on` ~2; every hit is doc prose or a test-fixture source string; the gate implementations themselves are synchronous, so no async context, spawn, or await exists in this part's real code

## unsafe
- N/A (seeds: `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0 / `// SAFETY:` = 0 / `transmute|from_raw|MaybeUninit|mem::zeroed` = 0; seed 4 `unsafe_code` = 1,the sole hit is `#![forbid(unsafe_code)]` at bazel_evidence.rs:1,which does not make the lens applicable)

## ffi
- N/A (seeds: `extern "C"|no_mangle|unsafe\(link_section` = 0 / `catch_unwind` = 0 / `repr\(C\)|repr\(transparent\)` = 0 / `CStr|CString|c_char` = 0; no FFI boundary exists in these files)

## macro
- xtask-p5#6 sev=low blast=leaf effort=S verdict=actionable - The test-only `crash_if_hooked!` macro is defined textually-identically in three sibling fns, differing only in the message string - fix: hoist to one module-scope `macro_rules! crash_if_hooked { ($stage:expr, $message:expr) => { #[cfg(test)] if let HookOutcome::Crash = hook($stage) { return Err(FoldError::single($message)); } }; }` and call with the stage plus message, or replace with a `#[cfg(test)]` generic helper fn - [packages/xtask/src/changelog.rs:812, packages/xtask/src/changelog.rs:886, packages/xtask/src/changelog.rs:1019]
  evidence: seed `macro_rules!` = 3 hits; all three definitions are identical except the embedded message (and the stage enum type, which the `$stage:expr` fragment never names); no proc-macro/syn usage exists in this part

## test
- clean: seeds ran: `#\[test\]|#\[tokio::test\]` ~70 / `assert_eq!\(|assert_ne!\(|assert!\(` ~500 / `proptest!|insta::assert|rstest` = 0 / `#\[ignore\]` = 0; sampled: 50 of ~500 assertion hits across the seven files' test modules;(the test-lens scope for this part is the `#[cfg(test)]` blocks inside the assigned files, since `tests/` belongs to no part partition); sampled assertions are table-driven with per-case failure messages, the fold-recovery crash-injection tests drive every journal boundary, and the evidence/seal tests assert binding and tamper rejection; no ignored, tautological, or network-touching tests spotted

## Coverage
- idiom: 1 finding(s
- own:  1 finding(s
- type: clean (seeds ran: 3/0/0; the three validators are boundary checks where parsed types would be over-engineering)
- api: clean (seeds ran: ~66/0/0; bin-only crate with no lib target, so pub surface is crate-internal)
- err: clean (seeds ran: ~95/~16/1/0; production panics are invariant asserts only)
- serde: clean (seeds ran: 13/9/0/~18; record shapes usa the right optionality and field-rejection attributes)
- obs: clean (seeds ran: ~31/0/0/0; all println sites are CLI product output)
- docs:  2 finding(s
- perf:  1 finding(s
- conc: clean (seeds ran: ~8/~13/0/0; all hits are doc prose or test-fixture strings)
- async: clean (seeds ran: ~40/~8/~3/~2; all hits are doc prose or test-fixture strings)
- unsafe: N/A (seeds: 0/0/0; unsafe_code =  1,only a forbid attribute)
- ffi: N/A (seeds: 0/0/0/0; no FFI surface)
- macro:  1 finding(s
- test: clean (seeds ran: ~70/~500/0/0; sampled:  50 of ~500 assertion hits; see section)