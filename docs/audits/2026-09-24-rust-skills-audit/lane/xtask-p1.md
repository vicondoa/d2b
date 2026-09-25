# xtask-p1 - xtask - part 1/5
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 8903 (excl. src/generated/**; policy range 5354-11075 of provider_crate_policy.rs) | modules: provider_crate_policy.rs (5354-11075), main.rs, gen_layer_catalogs.rs, provider_registration_authority.rs, service_catalog.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: provider_crate_policy.rs:5354-11075 (item-range split; sed line + 5353 = absolute)

## idiom
- xtask-p1#1 sev=medium blast=leaf effort=S verdict=actionable - gen_layer_catalogs.rs has two byte-identical helpers under different names: `string_slice` and `string_array` share the same signature and body (both emit a `pub const <name>: &[&str]` array), so callers guess which to use and a future shape change drifts only one copy - fix: delete `string_array` and route its 10 call sites (lines 299, 362, 367, 456, 466, 471, 496, 507, 512, 517) through `string_slice`, keeping `string_pair_slice` for the tuple case - [packages/xtask/src/gen_layer_catalogs.rs:147, packages/xtask/src/gen_layer_catalogs.rs:158]
  evidence: seed `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 0; static comparison of the two function bodies; census: `string_(slice|array)\(` over packages/xtask/src = 14 hits
- xtask-p1#2 sev=low blast=leaf effort=M verdict=actionable - emitted Rust source is embedded as single-line escaped string literals with backslash line continuations (`"... \n    \` chains, e.g. the `typed_noun_type` block), making the generator bodies unreadable and brittle to edit; a reviewer cannot diff the embedded code - fix: embed the emitted blocks as raw string literals (the content contains `"` but not `"##`, so `r##"..."##` delimiters work) in `surface_catalog_source` and in `redact_generated_protobuf_formatting`'s `raw_display`/`redacted_formatting` templates - [packages/xtask/src/gen_layer_catalogs.rs:377, packages/xtask/src/main.rs:473]
  evidence: seed `format!\(` = 115 in lane; static (unmeasured) - the escaped-string sites read at gen_layer_catalogs.rs:297-453 and main.rs:473-528
- xtask-p1#3 sev=medium blast=leaf effort=M verdict=actionable - the daemon-api IPC collector (`parse_rust_items` + `IpcItemCollector`) parses files with `syn`, then slices the original source text back out and re-parses fields and variants with ~150 lines of hand-rolled scanners (`parse_fields`, `parse_variants`, `split_top_level_entries`, `extract_body`, `strip_non_code_lines`, `normalize_ws`, `line_col_to_offset`), duplicating what the `syn` AST already provides and re-implementing angle-bracket depth counting for generics - fix: in `visit_item_struct`/`visit_item_enum`, extract `Field { name, ty }` and variants from `syn::Fields`/`syn::Variant` directly (type text via `quote::ToTokens`), then delete the text parsers and `line_col_to_offset`'s per-span O(n) scan - [packages/xtask/src/main.rs:1031, packages/xtask/src/main.rs:1142, packages/xtask/src/main.rs:1203]
  evidence: seed `for \w+ in 0\.\.` = 1 (main.rs:530, a bounded retry loop, not an index loop); the parse chain read at main.rs:1112-1245
- xtask-p1#4 sev=low blast=leaf effort=S verdict=actionable - `sanitize_generated_rust` contains a corrupted replacement literal `"#![allow(clipto_camel_casepy)]\n"` that can never match any generator output, so the sanitizer silently keeps whatever attribute the line was meant to strip in the committed generated file - fix: replace the literal with the actual protobuf/ttrpc-emitted marker it targets (or delete the line if the marker is no longer emitted) at main.rs:459 - [packages/xtask/src/main.rs:459]
  evidence: seed `generated\.replace` (static); `clipto_camel_casepy` occurs once in the workspace (packages/xtask/src/main.rs:459); the surrounding replace chain read at main.rs:454-472
- xtask-p1#5 sev=low blast=leaf effort=S verdict=actionable - in `collect_self_binding_scope` the row-close reset block at provider_crate_policy.rs:6662 is dead: a line whose trim equals `}` cannot also contain `SeedSelfBinding`, so the inner reset never fires, its comment describes behavior that never runs, and the inner scan has no exit at the row close (it runs to EOF for every `SeedProvider {`) - fix: drop the dead inner condition, reset `pending_subject`/`pending_role` when `code_text(lines[stop]).trim() == "}"`, and `break` the `while stop < lines.len()` loop there - [packages/xtask/src/provider_crate_policy.rs:6662, packages/xtask/src/provider_crate_policy.rs:6612]
  evidence: seed `fn validate_\w+|fn check_\w+` = 17 (the `check_*` family this scanner belongs to); static reading of the block at provider_crate_policy.rs:6608-6672

## own
- xtask-p1#6 sev=low blast=leaf effort=S verdict=actionable - `apply_citation_fixes` clones `lines[index]` before mutating it (`let mut line = lines[index].clone();`) although the slot is borrowed `&mut` and then reassigned on the same iteration - fix: `let mut line = std::mem::take(&mut lines[index]);` per the skill's `mem::take` pattern - [packages/xtask/src/provider_crate_policy.rs:7257]
  evidence: seed `\.clone\(\)` = 21 in lane (non-test policy range: 16); the site is the mutate-then-reassign shape at provider_crate_policy.rs:7255-7262
- xtask-p1#7 sev=low blast=leaf effort=S verdict=actionable - two ratchet lookups build an owned tuple just to call `BTreeSet::contains`, allocating a cloned String per signal during tree-wide scans (`family_exempt.contains(&(signal.module.clone(), token))` and `exempt.contains(&(signal.crate_name.clone(), signal.module.clone(), signal.token))`) - fix: replace `contains` with `family_exempt.iter().any(|(module, token)| *module == signal.module && *token == token)` (and the 3-tuple equivalent), or key both sets on `&str` like the neighboring `structural_exempt` set - [packages/xtask/src/provider_crate_policy.rs:6375, packages/xtask/src/provider_crate_policy.rs:8750]
  evidence: seed `\.clone\(\)` = 21 in lane; both sites read in context (provider_crate_policy.rs:6369-6378 and 8746-8753); no Rc/RefCell/Arc/Cow hits (0/0)
The remaining 17 clones and the sampled `to_owned`/`to_string` hits (every 8th of 128) all move borrowed scanner values into owned outputs or clone to descend clap subcommands (main.rs:902) - each explainable.

## type
- xtask-p1#8 sev=low blast=leaf effort=S verdict=actionable - `process_provider_ids(metric_label: Option<bool>)` uses an optional boolean to select among three label domains (all, metric-only, plus an unreachable `Some(false)` state) where the two production call sites only ever pass `None` or `Some(true)` - fix: split into `all_process_provider_ids()` and `metric_process_provider_ids()` (or a two-variant enum), and update the call sites at gen_layer_catalogs.rs:370, 474, 515 - [packages/xtask/src/gen_layer_catalogs.rs:288, packages/xtask/src/gen_layer_catalogs.rs:515]
  evidence: seed `(mode|kind|state): String` = 0; seed bool-flag = 0; the Option<bool> parameter shape read at gen_layer_catalogs.rs:288-295 and its call sites

## api
- clean: seeds ran: `\bpub (fn|struct|enum|trait|type|const|mod) ` = 42 (9 real items, the rest template strings and doc mentions), `pub .*\b(Arc|Rc|Box|RefCell)<` = 0, `^\s*pub use ` = 0. The part's surface is `pub fn check`/`pub fn regenerate` in service_catalog.rs:46,78 and provider_registration_authority.rs:65,84, `pub fn run_cli` in gen_layer_catalogs.rs:584, `pub fn fix` in provider_crate_policy.rs:7181, plus `pub(crate) const GENERATED_ARTIFACT` in two modules - every item is doc-commented, nothing leaks Arc/Rc or dependency types, and the crate is a binary (bin-only crates get no missing_docs).

## err
- clean: seeds ran: `\.unwrap\(\)|\.expect\(` = 159 (main 4, gen_layer_catalogs 6, provider_registration_authority 24, policy range 125 - all 125 policy hits and the other 34 sit inside `#[cfg(test)]` modules), `let _ = |\.ok\(\);` = 14 (test helpers plus the deliberate best-effort `let _ = fs::remove_dir_all` at main.rs:409), `panic!\(|unreachable!\(|todo!\(|unimplemented!\(` = 3 (main.rs:901 startup invariant, main.rs:1597 cfg(test) helper, one in policy tests), `enum \w*Error` = 0. No production-code unwrap/expect or swallowed Result in the lane; error reporting is `Result<_, String>` with canonical JSON diagnostics, which suites this CLI-policy context.

## serde
- xtask-p1#9 sev=medium blast=leaf effort=S verdict=actionable - `service_catalog.rs`'s `DeclarationFile` parses the committed per-crate `service-catalog.json` with `#[derive(Deserialize)]` and no `deny_unknown_fields`, while the sibling `RegistrationDeclaration` parsing `registrations.json` denies unknowns (`provider_registration_authority.rs:54`); a typo'd key in a declaration (e.g. `providerUid` misspelled) is silently ignored and the daemon's fixed-UID row silently disappears instead of failing the gate - fix: add `#[serde(deny_unknown_fields)]` to `DeclarationFile` - [packages/xtask/src/service_catalog.rs:22, packages/xtask/src/provider_registration_authority.rs:54]
  evidence: seeds ran: `derive\([^)]*(De)?[Ss]erialize` = 4, `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` = 5; the sibling-type comparison is direct line reading of the two declaration structs
The other serde shapes (camelCase rename on `RegistrationDeclaration`, `#[serde(default)]` optionality on provider_uid/services/wire_variant, `BrokerOperations` projecting only two of a row's many committed fields - deliberate, documented) are all sound; no hand-written Deserialize impls and no wire round-trips in the lane.

## obs
- clean: seeds ran: `\bprintln!\(|\beprintln!\(` = 16 (all in main.rs; those lines are the CLI's product output - artifact paths, usage, failures - and one in policy tests), `(info|debug|warn|error|trace)!\("` = 0, `\.instrument\(|#\[instrument` = 0, `tracing::|log::` = 3 (false positives: `changelog::` subcommand paths). No telemetry exists in this crate; user-facing stdout is the output, per the skill's CLI carve-out.

## docs
- xtask-p1#10 sev=low blast=leaf effort=S verdict=actionable - doc comments across the policy range carry text-corruption artifacts from an earlier automated rewrite: 20 lines end with a stray `/` after the closing period (`/// ... only shrinking from here./`) and 4+ comments have doubled opening parens (`((its Cargo package name).`, `((an edit to a`), plus the typo `whiche is what`; the artifacts render as odd punctuation in rustdoc and rot the file's readability - fix: mechanical doc cleanup over the file: replace `\./$` with `.` and `((`-doubles with single parens on the doc lines (lines 5360-6556 and 8428, 8640, 8648, 9043) - [packages/xtask/src/provider_crate_policy.rs:5360, packages/xtask/src/provider_crate_policy.rs:8428, packages/xtask/src/provider_crate_policy.rs:9043]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 9; the artifact pattern `^\s*//[/!] .*\./$` = 20 and `^\s*//[/!].*\(\(` = 4 within the policy range 5354-9438
- xtask-p1#11 sev=low blast=leaf effort=S verdict=actionable - `civil_from_days` (a port of the Howard Hinnant civil-calendar conversion) carries magic constants (719_468, 146_097, 146_096, 36_524, 153) with no citation or why, and `today_utc_iso8601` silently maps a before-epoch clock to epoch via `unwrap_or(0)` - fix: add a doc comment naming the algorithm and its constants, and decide the before-epoch behaviour explicitly (return an error or a documented fallback) - [packages/xtask/src/main.rs:1555, packages/xtask/src/main.rs:1544]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 9; the magic-number block read at main.rs:1544-1568

## perf
- xtask-p1#12 sev=low blast=leaf effort=S verdict=actionable - `message_only_proto` calls `trimmed.starts_with(&format!("service {service_name} "))` inside the per-line loop, allocating a String on every line of the proto file when the prefix never changes - fix: hoist `let marker = format!("service {service_name} ");` (or compare `trimmed.strip_prefix("service ")` then the name) above the loop - [packages/xtask/src/main.rs:431]
  evidence: seed `format!\(` = 115 in lane; site is a loop-body allocation, cold CLI path - static (unmeasured)
- xtask-p1#13 sev=low blast=leaf effort=S verdict=actionable - `repo_root()` returns `Ok(Box::leak(path.into_boxed_path()))`, so every successful call leaks a heap allocation and re-scans env vars and parent directories; it is called by nearly every command handler - fix: cache the result once, e.g. `static ROOT: OnceLock<&'static Path>` (std, no dependency) computed on first call - [packages/xtask/src/main.rs:582]
  evidence: seed `\.to_string\(\)` = 35 and `Vec::new\(\)` family = 68 in lane; the leak site read at main.rs:558-590 - static (unmeasured)
- xtask-p1#14 sev=low blast=leaf effort=S verdict=actionable - `render_schema(&RootSchema)` clones the entire schema document (large `serde_json::Value` trees for the 19 `schema_for!` documents) only to override `meta_schema` before serializing - fix: have `write_schemas` take ownership of the `Vec<(&str, RootSchema)>` and mutate each schema in place (callers already hold the schemas by value from `schema_documents()`) - [packages/xtask/src/main.rs:972]
  evidence: seed `format!\(` = 115 in lane; the clone-then-mutate shape read at main.rs:958-978, called from gen_schemas/gen_cli_schemas/gen_zone_storage_schema - static (unmeasured)

## conc
- clean: seeds ran: `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 0, `Atomic\w+|Ordering::` = 3, `thread_local!|unsafe impl (Send|Sync) for` = 0. The only concurrency in the lane is a test-fixture `AtomicU32` counter plus `Ordering::Relaxed` in the policy tests module (provider_crate_policy.rs:9440-9455); production code has no threads, locks, or atomics.

## async
- N/A (seeds: `async fn|async move|\.await` = 0, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 0, `tokio::sync::(Mutex|RwLock|Notify)` = 0, `#\[tokio::(main|test)\]|Runtime::block_on` = 0 - all zero; the lane declares no async fn and no runtime usage)

## unsafe
- N/A (seeds: `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0 - all zero; the crate manifest sets `unsafe_code = "forbid"` and the single `unsafe_code` string in main.rs:456 is the sanitizer's removal literal, not code; per the card, seed 4 alone does not make the lens applicable)

## ffi
- N/A (seeds: `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0 - all zero; the lane crosses no FFI boundary)

## macro
- clean: seeds ran: `macro_rules!` = 3, `proc_macro|syn::|quote!` = 15, `\$crate` = 0, `to_compile_error|new_spanned` = 0. No macro definitions exist in the lane: the `macro_rules!` hits are a doc-comment mention (main.rs:1054) and the citation scanner's `item_binding` matcher (provider_crate_policy.rs:7721), and the `syn::` hits are the IPC visitor using `syn` as a parsing library, not a proc macro; `proc_macro_deps` hits are BUILD-attribute strings in docs and scanner constants.

## test
- xtask-p1#15 sev=low blast=leaf effort=S verdict=actionable - the broker-operation domain test recomputes its expectation with the same filter the function under test applies (`catalog.rows.iter().filter_map(|row| row.wire_variant.clone())` re-derives `broker_operation_values`' own pick), so the `assert_eq!(values, expected)` can never disagree with the projection logic; only the human-written pins (`UsbipBind` present, `SpawnRunner`/`vmStart` absent) carry behaviour - fix: drop the recomputed `expected` and assert the human-written pins only (the vector equality adds nothing the pins do not) - [packages/xtask/src/gen_layer_catalogs.rs:705]
  evidence: seeds ran: `#\[test\]|#\[tokio::test\]` = 155 (61 in the policy tests module, 79 in xtask/tests/**), `assert_eq!\(|assert_ne!\(|assert!\(` = 471, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0; the same-logic expectation read at gen_layer_catalogs.rs:694-720
The remaining suite is behavior-first (fixture trees exercising both gate directions, committed-tree ratchet pins, drift and idempotence tests in provider_registration_authority.rs and main.rs; all policy unwraps live in `#[cfg(test)]`); no `#[ignore]`, no property/snapshot tooling (snapshot pins instead live in `xtask/tests/**` and the policy ratchet tests).

## Coverage
- idiom: 5 finding(s)
- own: 2 finding(s)
- type: 1 finding(s)
- api: clean (seeds ran: 42/0/0)
- err: clean (seeds ran: 159/14/3/0)
- serde: 1 finding(s)
- obs: clean (seeds ran: 16/0/0/3)
- docs: 2 finding(s)
- perf: 3 finding(s)
- conc: clean (seeds ran: 0/0/3/0)
- async: N/A (seeds: 0/0/0/0 all zero; no async fn, spawn, or runtime in the lane)
- unsafe: N/A (seeds: 0/0/0 all zero; manifest forbids; seed 4 alone not applicable)
- ffi: N/A (seeds: 0/0/0/0 all zero; no FFI surface)
- macro: clean (seeds ran: 3/15/0/0)
- test: 1 finding(s)