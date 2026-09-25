# xtask-p2 - xtask - part 2/5
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 8954 (excl. src/generated/**) | modules: provider_crate_policy.rs:1-5353, gen_broker_operations.rs, delivery/eligibility.rs, diagnostic_redaction.rs, delivery/history_proof.rs
Lenses: idiom own type api err serde obs docs perf conc async unsafe ffi macro test | Partitions: provider_crate_policy.rs:1-5353 (item-range split; absolute line = sed line)

## idiom
- clean: seeds `for \w+ in 0\.\.` = 1, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 0, `let mut \w+ = (String|Vec)::new\(\)` = 18; the one index loop (diagnostic_redaction.rs:284 `for _ in 0..overflow`) is the idiomatic repeat-n form, the hand-rolled scanners (identifier_words, string_literal_spans, driver_declarations, normalize_ansi_escape_sequences) are byte/indent state machines with no stdlib equivalent, and the `let mut rows = String::new()` accumulators are generator text builders whose artifact is the emitted file.

## own
- xtask-p2#1 sev=low blast=leaf effort=S verdict=actionable - `check_members(&repo_root, members.clone())` clones the whole workspace-member vec at the check entry point because `check_members` (provider_crate_policy.rs:7916) takes `Vec<WorkspaceMember>` by value while its body only reads it (`.iter()`, `.iter().map()`); the signature forces the clone - fix: change `fn check_members(repo_root: &Path, members: &[WorkspaceMember])` and drop the clone at the call site (second caller at 9560 passes `&manifest_workspace_members(&root)?`) - [packages/xtask/src/provider_crate_policy.rs:577, packages/xtask/src/provider_crate_policy.rs:7916]
  evidence: seed `\.clone\(\)` = 47 hits in lane; signature read at 7916-7918 shows read-only use
- xtask-p2#2 sev=low blast=leaf effort=S verdict=actionable - `check_shared_family_knowledge_with` builds `exempt: BTreeSet<(String, &str)>` with `row.module.to_owned()` and probes it with `signal.module.clone()`, when the ratchet rows are `&'static str` and the signal already owns a `String`; both the build-time to_owned and the per-signal clone disappear by keying borrowed strs - fix: `let exempt: BTreeSet<(&str, &str)> = ratchet.iter().map(|row| (row.module, row.token)).collect()` and probe `exempt.contains(&(signal.module.as_str(), signal.token))` - [packages/xtask/src/provider_crate_policy.rs:5200, packages/xtask/src/provider_crate_policy.rs:5206]
  evidence: seeds `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 135, `\.clone\(\)` = 47; sites 5200/5206 read
- xtask-p2#3 sev=low blast=leaf effort=S verdict=actionable - `profile_catalog` clones every row's `wire_variant` String (`row.wire_variant.clone()`) only to format it into the generated profile catalog text; the values are never mutated or stored - fix: return `Vec<&str>` via `.filter_map(|row| row.wire_variant.as_deref())` and change `string_list` (gen_broker_operations.rs:772) to take `Item = &str` (its only two call sites are 860-861) - [packages/xtask/src/gen_broker_operations.rs:844, packages/xtask/src/gen_broker_operations.rs:772]
  evidence: seed `\.clone\(\)` = 47 hits in lane; string_list call sites verified at 860-861

## type
- xtask-p2#4 sev=low blast=leaf effort=S verdict=actionable - `FamilyKnowledgeSignal.text: String` (provider_crate_policy.rs:4664-4675) carries two meanings discriminated only by `class`: literal/identifier text for Literal/Assembled/Identifier, and a serialized count for ServerState (`text: format!("{server_state_count}")` at 5104) that the renderer re-parses (`signal.text.parse::<usize>().unwrap_or(0)` at 5179), silently defaulting a non-numeric to 0 - fix: add a typed `count: Option<usize>` field (or split the struct per class), fill it at 5104, and render by matching `class` without the parse - [packages/xtask/src/provider_crate_policy.rs:5104, packages/xtask/src/provider_crate_policy.rs:5179]
  evidence: seeds `fn validate_\w+|fn check_\w+` = 12, `(mode|kind|state): String` = 5 (3 are `artifact_kind` false positives); sites 5104/5179 read

## api
- clean: seeds `\bpub (fn|struct|enum|trait|type|const|mod) ` = 27, `pub .*\b(Arc|Rc|Box|RefCell)<` = 0, `^\s*pub use ` = 0; xtask is a bin-only crate (no lib target, publish = false, packages/xtask/Cargo.toml), so the pub items are crate-internal surface with no external callers to break; no internals-in-signature shapes and no re-export arms exist.

## err
- clean: seeds `\.unwrap\(\)|\.expect\(` = 94, `let _ = |\.ok\(\);` = 2, `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 4, `enum \w*Error` = 0; 90 of the 94 unwrap/expect hits sit in `#[cfg(test)]`; the four production hits are invariant-named expects on literally-built values (provider_crate_policy.rs:446, gen_broker_operations.rs:1155), an expect after a check the compiler cannot see (gen_broker_operations.rs:423, guarded by the pair check at 404), and `unreachable!` arms after closed-set validation (530, 913, 926, 1037); the two `let _ =` sites (diagnostic_redaction.rs:412, 421) are deliberate best-effort temp-dir cleanup.

## serde
- clean: seeds `derive\([^)]*(De)?[Ss]erialize` = 22, `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` = 42, `impl .*Deserialize.*for` = 0, `serde_json::from_|serde_json::to_` = 11; every wire shape uses `rename_all` plus `deny_unknown_fields` with closed-set validation in `validate_row`/`parse_text`, the three optionality meanings are used correctly (`#[serde(default)]` vs `Option` vs `skip_serializing_if`), and `CheckConclusion`/`HistoryVerdict` fail closed on unknown conclusions; no hand-written deserializers.

## obs
- clean: seeds `\bprintln!\(|\beprintln!\(` = 1, `(info|debug|warn|error|trace)!\("` = 0, `\.instrument\(|#\[instrument` = 0, `tracing::|log::` = 0; the single eprintln! (diagnostic_redaction.rs:379) is the operator-facing failure line of a CLI filter whose stderr is the product output, not telemetry; no tracing/log dependency in the lane.

## docs
- clean: seeds `^\s*pub (fn|struct|enum|trait|const|type)` = 25, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 40; bin-only crate, so per the card's false-positive note undocumented pub items are not findings; all five files carry a `//!` module doc and every public entry point (run, run_capture, evaluate, open_sealed_candidate, prove, gen_broker_operations, check) has a doc comment whose first sentence carries the contract.

## perf
- clean: seeds `format!\(` = 143, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 21, `\.to_string\(\)` = 17; every hit is an error path, a one-shot policy diagnostic, or a generator text builder whose artifact is the emitted file (card false positive); the only bounded-buffer code (read_diagnostic_tail, VecDeque::with_capacity at the 4 MiB cap) is deliberate; all findings would be static (unmeasured) and none rises to a proposal.

## conc
- clean: seeds `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 0, `Atomic\w+|Ordering::` = 3, `thread_local!|unsafe impl (Send|Sync) for` = 0; the only atomic is the test-only `SCRATCH_SEQUENCE: AtomicU32` (diagnostic_redaction.rs:393-408) used with Relaxed ordering, the weakest correct ordering for a scratch-dir uniquifier.

## async
- N/A (seeds: 0/0/0/0 all zero; no async fn, no .await, no spawn, no runtime in the lane)

## unsafe
- N/A (seeds: 0/0/0 all zero; packages/xtask/Cargo.toml sets `unsafe_code = "forbid"`)

## ffi
- N/A (seeds: 0 all zero; no extern surface, no repr(C)/repr(transparent), no CStr/CString in the lane)

## macro
- N/A (seeds: 0 all zero; no macro_rules!, no proc-macro, no $crate in the lane)

## test
- clean: seeds `#\[test\]|#\[tokio::test\]` = 138 (59 in-module across the five files, 79 in tests/), `assert_eq!\(|assert_ne!\(|assert!\(` = 382, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0; the in-module tests for all five files are behavior assertions with failure messages and table-driven cases (eligibility.rs:717-734 loops over every non-success conclusion; gen_broker_operations.rs:1327-1334 proves the triage view moves byte-for-byte with a row edit; diagnostic_redaction.rs:520-586 exercises truncation, multibyte splits, and malformed bytes); no test that cannot fail was found.

## Coverage
- idiom: clean (seeds ran: 1/0/18)
- own: 3 finding(s)
- type: 1 finding(s)
- api: clean (seeds ran: 27/0/0)
- err: clean (seeds ran: 94/2/4/0)
- serde: clean (seeds ran: 22/42/0/11)
- obs: clean (seeds ran: 1/0/0/0)
- docs: clean (seeds ran: 25/0/40)
- perf: clean (seeds ran: 143/21/17)
- conc: clean (seeds ran: 0/0/3/0)
- async: N/A (seeds: 0/0/0/0 all zero; no async code in lane)
- unsafe: N/A (seeds: 0/0/0 all zero; unsafe_code = "forbid" in packages/xtask/Cargo.toml)
- ffi: N/A (seeds: 0 all zero; no FFI surface)
- macro: N/A (seeds: 0 all zero; no macros)
- test: clean (seeds ran: 138/382/0/0)