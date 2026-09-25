# d2bd-p5 - d2bd - part 5/8
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 10,670 (excl. src/generated/**) | modules: interaction_composition, foundation_seed, principal_allocation, provider_shutdown, process_resource_runtime
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: src/interaction_composition.rs, src/foundation_seed.rs, src/principal_allocation.rs, src/provider_shutdown.rs, src/process_resource_runtime.rs

## idiom
- clean: seeds run: `for \w+ in 0\.\.` = 7, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 0, `let mut \w+ = (String|Vec)::new\(\)` = 7; the 7 index-loop hits are fixed-count poll/retry loops (`for _ in 0..N`) inside the tests module,and the 7 accumulator hits are mixed-effect builders (store rows, materialized specs, client lists( where the equivalent collect chain would be longer than the loop.

## own
- d2bd-p5#1 sev=low blast=leaf effort=S verdict=actionable - the `run_effect` closure (bound `F: FnOnce`) clones `supervisor` and `process_ticket` a second time inside its body, though the captured values can move straight into the `async move` block (which only borrows them( - fix: remove `let supervisor = supervisor.clone();` and `let process_ticket = adoption_ticket.clone();`, letting the outer captures move into the `async move` - [packages/d2bd/src/interaction_composition.rs:4343, packages/d2bd/src/interaction_composition.rs:4344]
  evidence: seed `\.clone\(\)` = 228 hits (sampled:  47 of 228); the sampled pair at 4343-4344 sits inside a `FnOnce` closure (signature at 6150), so the duplicates cannot be required; every other sampled clone is explainable (tokio::spawn capture boundaries, owned-struct assembly, error-path copies)

## type
- clean: seeds run: `fn validate_\w+|fn check_\w+` = 4, `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0; the four validator functions check cross-field compositions of daemon-owned or wire-derived compound state with no parse-once replacement candidate



## api
- clean: seeds run: `\bpub (fn|struct|enum|trait|type|const|mod) ` = 97, `pub .*\b(Arc|Rc|Box|RefCell)<` = 1, `^\s*pub use` = 1; the one `Arc` in a public signature (`stop_token` -> `Arc<AtomicBool>`, 5109( shares a genuinely multi-owner shutdown token, and the `pub use` re-export block (provider_shutdown.rs:8( is the house single-surface pattern; the rest of the exported surface exposes fields privately, and no dependency types leak

## err
- d2bd-p5#2 sev=medium blast=leaf effort=S verdict=actionable - `reap_finished_handlers` joins finished listener handler tasks with `let _ = handlers.swap_remove(index)..await;`, silently discarding the `JoinError`, so a panicked handler (whose `handler_active.fetch_sub` decrement sits after the panic-capable body( neither logs and leaks its bounded 64-slot admission reservation( - fix: log the `JoinError` with `tracing::warn!` at the reap site,and wrap the spawn body so the `fetch_sub` decrement runs in a panic-safe guard, not after the admit body - [packages/d2bd/src/interaction_composition.rs:5365, packages/d2bd/src/interaction_composition.rs:5310]
  evidence: seed `\.unwrap\(\)|\.expect\(` = 408 hits (sampled:  47 of 408);`let _ = |\.ok\(\);` = 9 (the other eight are deliberate best-effort cleanup with follow-up polls or shutdown joins);`\bpanic!\)...` = 4 (all in the tests module);`enum \w*Error` = 6

## serde
- clean: seeds run: `derive\([^)]*(De)?[Ss]erialize` = 5, `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` = 10, `impl .*Deserialize.*for` = 0, `serde_json::from_|serde_json::to_` = 71; the five `#[derive(Deserialize)]` request structs use `#[serde(default)]` Option fields for service-consumed messages (absent/null conflation acceptable there),and no hand-written deserializer exists.

## obs
- clean: seeds run: `\bprintln!\(|\beprintln!\(` = 0, `(info|debug|warn|error|trace)!\("` = 0, `\.instrument\(|#\[instrument` = 0, `tracing::|log::` = 13 (one hit is the substring `log::` inside `ApiCatalog::`, not a log site); all real events use named fields (e.g. 929-931, 1067-1069, 5268-5325), no interpolated messages,and no secrets in fields.

## docs
- clean: seeds run: `^\s*pub (fn|struct|enum|trait|const|type)` = 97, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 107; spot reads of the pub surface (RegisteredInteractionSession methods, CoreDisplayResourceEvidence::from_committed_policy, InteractionListenerSet methods, the Seed*PrincipalAllocation/HostAccounts APIs( all carry one-line first-sentence docs; no canonical-section-needing item surfaced in the sample



## perf
- clean: seeds run: `format!\(` = 48, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 53, `\.to_string\(\)` = 15; all allocation sites are error paths, one-shot diagnostics, listener-path/key building,and daemon-owned string boundaries (cold per static read),no hot-loop `format!` or grow-by-push pattern found (static (unmeasured()

## conc
- clean: seeds run: `std::thread::|thread::spawn|thread::scope` = 3 (one is the sanctioned `synchronous path` `#[allow]` std::thread::sleep at 6194, two are test-loop yields), `\bMutex<|\bRwLock<` = 1 (a test Backend fixture), `Atomic\w+|Ordering::` = 32 (stop flag stores Release/loads Acquire,reservation counter uses AcqRel; no weak ordering misuse found), `thread_local!|unsafe impl (Send|Sync) for` = 0

## async
- d2bd-p5#3 sev=medium blast=leaf effort=M verdict=actionable - `admit_interaction_socket`'s per-request dispatch holds the daemon-global `runtime` lock (the `AsyncMutex<Option<InteractionRuntimeSet>>`( across the whole `.await` of `dispatch_component_request_for_session`, serializing every Zone's sessions andthe VM-start display reconcile behind one contended lock; the code itself records this as a residual at 5518-5529 - fix: per the recorded note, hand out a per-Zone handle (`BTreeMap<String, Arc<AsyncMutex<InteractionComposition>>>`) cloned under the outer lock,and move the sync-seat methods off their global lock, adding the named concurrency test - [packages/d2bd/src/interaction_composition.rs:5518-5530]
  evidence: seed `async fn|async move|\.await` = 302 hits (sampled:  44 of 302);`tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 7, `tokio::sync::(Mutex|RwLock|Notify)` = 1 (the `AsyncMutex` alias at 93), `#\[tokio::(main|test)\]|Runtime::block_on` = 10; the guard-hold across `.await` is observed at 5530 onward, documented as deliberate-but-unfixed at 5518-5529

## unsafe
- N/A (seeds: `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0; all zero; the partition declares no unsafe blocks/fns/impls, so the lens criteria fail.

## ffi
- N/A (seeds: `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0; all zero; no FFI surface in this partition.

## macro
- N/A (seeds: `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0; all zero; no macro definitions or expansions in these files.

## test
- clean: seeds run: `#\[test\]|#\[tokio::test\]` = 95, `assert_eq!\(|assert_ne!\(|assert!\(` = 430 (sampled: 48 of 430), `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0; the sampled assertions (src test modules and tests/** integration files( assert behavior, error variants, statuses,and outcomes with human-written expected values and contextual failure messages; tests use tempdirs and fixed poll counts, no network dependency,and every sampled test can fail on a real behavior change

## Coverage
- idiom: clean (seeds ran:  7/0/7)
- own:  1 finding(s)
- type: clean (seeds ran:  4/0/0)
- api: clean (seeds ran:  97/1/1)
- err:  1 finding(s)
- serde: clean (seeds ran:  5/10/0/71)
- obs: clean (seeds ran:  0/0/0/13)
- docs: clean (seeds ran:  97/0/107)
- perf: clean (seeds ran:  48/53/15)
- conc: clean (seeds ran:  3/1/32/0)
- async:  1 finding(s)
- unsafe: N/A (seeds:  0/0/0 all zero; no unsafe blocks)
- ffi: N/A (seeds:  0/0/0/0 all zero; no FFI surface)
- macro: N/A (seeds:  0/0/0/0 all zero; no macros)
- test: clean (seeds ran:  95/430/0/0)