# d2b-resource-compiler - d2b-resource-compiler
Baseline: 6ebdd4cec | LOC audited: 7461 (excl. src/generated/, which is absent; src 5514 + tests 1947) | modules: whole crate (src/lib.rs, src/linux.rs, src/main.rs, tests/cli.rs, tests/phase2.rs)
Lenses: idiom,own,type,api,err,serde,obs,docs,perf,conc,async,unsafe,ffi,macro,test | Partitions: n/a (single-part lane)

## idiom
- d2b-resource-compiler#1 sev=medium blast=leaf effort=S verdict=actionable - main.rs hand-rolls identical output-sanitizer helpers already in lib.rs (safe_token/bound_ascii duplicate sanitize_token/bound_message body-for-body) - fix: expose lib.rs sanitize_token/bound_message as pub(crate) helpers (dropping safe_label indirection if unneeded) and replace main.rs safe_token/bound_ascii with calls to the shared pair - [packages/d2b-resource-compiler/src/lib.rs:2401, packages/d2b-resource-compiler/src/lib.rs:2438, packages/d2b-resource-compiler/src/main.rs:2531, packages/d2b-resource-compiler/src/main.rs:2545]
  evidence: idiom seed 3 (`let mut \w+ = (String|Vec)::new\(\)`) = 11 hits; both helper pairs rebuild strings char-by-char with the identical filter/ASCII-bound rules
- d2b-resource-compiler#2 sev=low blast=leaf effort=S verdict=actionable - sanitize_token's char-loop filter is expressible as an iterator pipeline - fix: `value.chars().filter(|character| (character.is_ascii_graphic() && *character != '/' && *character != '\\') || *character == ' ').collect::<String>()` - [packages/d2b-resource-compiler/src/lib.rs:2402]
  evidence: idiom seed 3 hit at lib.rs:2402 (the canonical copy named by #1)
- d2b-resource-compiler#3 sev=low blast=leaf effort=S verdict=actionable - check_metadata_closure's unexpected-layout-entries accumulation could be a filter_map+collect pipeline - fix: `let unexpected: Vec<String> = entries.into_iter().filter_map(|entry_name| match entry_name.to_str() { Some(name) if expected.contains(name) => None, Some(name) => Some(truncate_entry(name)), None => Some("<non-utf8>".to_owned()) }).collect();` (kept the trailing sort* - [packages/d2b-resource-compiler/src/lib.rs:1724]
  evidence: idiom seed 3 hit at lib.rs:1724
- d2b-resource-compiler#4 sev=low blast=leaf effort=S verdict=actionable - executable-set difference builders are two push-loops a chain can express in one collect - fix: `let difference: Vec<String> = names.difference(&declared_names).map(|name| format!("bin={}", truncate_entry(name)).chain(declared_names.difference(&names).map(|name| format!("manifest={}", truncate_entry(name)).collect();` - [packages/d2b-resource-compiler/src/lib.rs:1913]
  evidence: idiom seed 3 hit at lib.rs:1913
- clean: seeds ran: 2/0/11; the two index-loop hits are test-only depth builders (main.rs:2335, a phase2.rs fixture; no hand-written derive-class impls; the remaining accumulation sites are loops with side effects or early exits where the skill's own guidance prefers a plain for loop

## own
- d2b-resource-compiler#5 sev=low blast=leaf effort=S verdict=actionable - SchemaCache uses RefCell<BTreeMap> for a lazy schema cache though the only two call sites could take `&mut self` - fix: change `fn schema(&self,...)` to `fn schema(&mut self,...)`, drop the RefCell holding the cache in plain `BTreeMap` field, and mark `let mut schema_cache` in validate_resources - [packages/d2b-resource-compiler/src/main.rs:1148, packages/d2b-resource-compiler/src/main.rs:1631, packages/d2b-resource-compiler/src/main.rs:818, packages/d2b-resource-compiler/src/main.rs:846]
  evidence: own seed 3 (`Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<|Cow<`) = 1 hit at main.rs:1148
- clean: seeds ran:62/50/1/0; all 62 `.clone()` sites are owned-output construction (map keys/values at lib.rs:546-1135, entry/store-path copies at main.rs:480-518, clone_provider_input at main.rs:538-548, Copy newtypes, or tokio-less single-owner cases; the 50 to_owned/to_vec/to_string sites are needed map keys and wire-boundary copies (`to_string_lossy().into_owned()` on non-UTF8 paths; Cow 0

##type
- clean: seeds ran:14/0/0; the 14 validate/check fns are the intrinsic JSON-schema/artifact validators over raw serde_json::Value resolved from user-authored files - no replaceable parsed type (the boundary is deliberately unstructured JSON; no bool flags, no stringly-typed state; `strict_secrets: bool` is a single CLI mode switch, not a flag soup

##api
- d2b-resource-compiler#6 sev=medium blast=leaf effort=S verdict=actionable - LinuxAnchoredDir's three flag accessors (resolve_flags, readable_flags, executable_flags) are public, return rustix::fs::{ResolveFlags,OFlags} (making rustix part of the public contract), and have zero callers anywhere - fix: delete the accessors (the anchored impl reads the module constants directly unless a real consumer arrives - [packages/d2b-resource-compiler/src/linux.rs:93, packages/d2b-resource-compiler/src/linux.rs:98, packages/d2b-resource-compiler/src/linux.rs:103]
  evidence: api seed 1 (`\bpub (fn|struct|enum|trait|type|const|mod) `) = 67 hits; census: `resolve_flags\(\)|readable_flags\(\)|executable_flags\(\)` over packages/ nixos-modules/ tests/ docs/reference/ labs/ = 3 hits (the definitions here plus 10 hits of d2b-provider-volume-local's own local same-named fn (adapter.rs:1161 - zero external callers of this crate's accessors
- clean: seeds ran:67/0/0; surface is lib.rs+linux.rs public items only (main.rs is a bin with no pub surface; no pub use re-exports, no Arc/Rc/Box/RefCell in public signatures; public items are deliberate contract types and trait impls (not internals leaking out)

##err
- d2b-resource-compiler#7 sev=low blast=leaf effort=S verdict=actionable - validate_schema_node_with_budget re-gets additionalProperties/items after an if-let shape check and panics "checked above" where a pattern bind removes the second get - fix: `if let Some(additional @ Value::Object(_)) = object.get("additionalProperties") { ... additional ... }` (same for items - [packages/d2b-resource-compiler/src/main.rs:1467, packages/d2b-resource-compiler/src/main.rs:1468, packages/d2b-resource-compiler/src/main.rs:1477, packages/d2b-resource-compiler/src/main.rs:1478]
  evidence: err seed 1 (`\.unwrap\(\)|\.expect\(`) = 45 hits; the two re-get expect sites are named "checked above"
- d2b-resource-compiler#8 sev=low blast=leaf effort=S verdict=actionable - usage() takes a program param it never uses and silences it with `let _ = program;` - fix: drop the `program` parameter (and its `env::args_os().next()` binding from usage() andits six call sites (main.rs:242,245,254,261,264,265 - [packages/d2b-resource-compiler/src/main.rs:269, packages/d2b-resource-compiler/src/main.rs:270]
  evidence: err seed 2 (`let _ = |\.ok\(\);`) = 3 hits; lib.rs:2388,2433 are deliberate `write!` fmt-Error ignores on infallible String writers (acceptable; main.rs:270 masks an unused parameter instead
- clean: seeds ran:45/3/3; remaining unwrap/expect sites are named invariants ("canonical_digest always returns a contract digest", "SHA-256 is always 32 bytes", "was checked in the first pass") or test-only; no production panic!/unreachable!/todo!/unimplemented!; the two error enums (lib.rs:461 StaticControllerProjectionError, linux.rs:44 AnchorError) are closed variant sets split by caller-visible failure (each with code()/Display/Error contract, not a string-match taxonomy

##serde
- clean: seeds ran:3/14/0/10; CLI inputs (CompileInput, ProviderInput) use camelCase + deny_unknown_fields + per-field #[serde(default)] per the card; BundleOutput is Serialize-only with skip_serializing_if for the optional zone_uid; no hand-written Deserialize impls; the serde_json from/to sites map failures to CliError/Diagnostic kinds rather than leaking stringified errors (main.rs:229,400,526, lib.rs:1549,1664

##obs
- clean: seeds ran:1/0/0/0; the only site is main.rs:208 eprintln!("{error}") in the CLI error path - product output the user asked the compiler for, not telemetry; no tracing/log dependency or span usage anywhere in the crate

##docs
- clean: seeds ran:66/0/52; every public item in lib.rs and linux.rs carries a one-line `///` first sentence (verified over the full public surface; no missing canonical sections worth flagging (Result-returning items document their failure contract via the Diagnostic/kind docs; the module doctest (lib.rs:13-16 is live, not ignored; main.rs is a binary crate where missing_docs would be wrong per the card

##perf
- d2b-resource-compiler#9 sev=low blast=leaf effort=S verdict=actionable - validate_resources serializes every resource with serde_json::to_vec just to measure its wire size, allocating a fresh buffer per resource - fix: serialize into a counting io::sink-style Write (or walk the Value once for a length to drop the per-resource Vec - [packages/d2b-resource-compiler/src/main.rs:704]
  evidence: perf combined seed (`format!\(|Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)|\.to_string\(\)`) = 157 hits; this site is the only per-iteration allocation in a resource loop - static (unmeasured, no benchmark baseline exists
- clean: seeds ran:157 (combined; all other format!/Vec::new/to_string sites are cold one-shot error diagnostics or bounded string builders (candidate refs at lib.rs:843-1254, schema filenames at main.rs:40-53, PEM/hex builders; no hot loop; codegen/layout advice out of scope

##conc
- clean: seeds ran:0/0/5/0; all five seed-3 hits are std::cmp::Ordering:: comparisons in scalar schema number checks (main.rs:1439,1813,1815,1971,1973, not atomic loads/stores - no threads, no locks, no atomics, no thread_local, no manual Send/Sync claims - no concurrency surface

##async
- N/A (seeds: 0/0/0/0 all zero; no async fn, await, spawn, tokio sync, or runtime anywhere in the crate; the compiler is a synchronous build-time CLI by design

##unsafe
- clean: seeds ran:1/1/6/0; the single unsafe block linux.rs:277 (execveat is the workspace-enumerated sanctioned site (U1 constraint (d) 8 and carries a precise // SAFETY: comment (linux.rs:273-276 asserting pointer provenance into owned CStrings, NUL termination of both pointer vectors, and the fstat-verified O_PATH|O_CLOEXEC descriptor( no pub unsafe fn, no transmute/zeroed; the four extra seed-3 hits are FileType::from_raw_mode/Mode::from_raw_mode rustix constructors - not raw-pointer work

##ffi
- clean: seeds ran:0/0/0/5; the five CString hits (linux.rs:10,251,260 build argv/envp for the rustix execveat syscall wrapper within the single unsafe block - this crate declares no extern "C"/no_mangle/repr(C boundary of its own; the actual foreign-call boundary is owned by the rustix crate

##macro
- N/A (seeds: 0/0/0/0 all zero; no macro_rules!, proc-macro, syn/quote, $crate, or to_compile_error surface in the crate

##test
- clean: seeds ran:47/101/0/0; 47 tests over 5 files are deterministic (no network, clock, or unseeded generators, contain no #[ignore], and assert behaviour-level contracts (error codes at cli.rs:126-353, wire shapes at phase2.rs:477-499, ordering/duplication at main.rs:2301-2312, with the form matching the assertion (unit tests in src for internals, integration tests in tests/ for the CLI and public lib API, a live doctest for the digest contract; no test that cannot fail identified

## Coverage
- idiom: 4 finding(s)
- own:  1 finding(s)
- type: clean (seeds ran:14/0/0; JSON-schema/artifact validators over raw Value are inherent, no typed replacement
- api: 1 finding(s)
- err:  2 finding(s)
- serde: clean (seeds ran:3/14/0/10; camelCase+deny_unknown_fields+defaults per the card, no hand-written Deserialize
- obs: clean (seeds ran:1/0/0/0; only a CLI error report, product output
- docs: clean (seeds ran:66/0/52; every pub item carries a first-sentence doc, live doctest
- perf:  1 finding(s)
- conc: clean (seeds ran:0/0/5/0; all hits are std::cmp::Ordering comparisons, no concurrency primitives
- async: N/A (seeds: 0/0/0/0 all zero; synchronous build-time CLI
- unsafe: clean (seeds ran:1/1/6/0; single sanctioned execveat block with SAFETY comment, from_raw hits are rustix constructors
- ffi: clean (seeds ran:0/0/0/5; CString sites feed the rustix syscall wrapper, no crate-owned FFI surface
- macro: N/A (seeds:  0/0/0/0 all zero; no macros defined
- test: clean (seeds ran:47/101/0/0; 47 deterministic behaviour-level tests, no ignores