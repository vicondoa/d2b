# d2b-broker-p4 - d2b-broker - part 4/7
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 10,906 (excl. src/generated/)) | modules: audit; ops::{tap, disk_init, route, store_sync_audit, spawn_runner, host_generation_handoff, store_sync_export}
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: full-file scope for these 8 modules (U1 section f part 4/7 of d2b-broker)

## idiom
- clean: seeds ran:  10/0/7 - every hit checked: the 10 `for i in 0..` hits are retry loops (quarantine-name, mkfs, udev-wait, test churn)and the 7 `let mut String/Vec::new()` hits are accumulation loops with early bounds/limit checks (legacy-export cap, bounded-line reader, scan cap, test fixtures), where an iterator pipeline would obscure the early exits; no hand-written derive-eligible impls in lane.



## own
- clean: seeds ran:  83/288/6/0 - all 83 `.clone()` hits read in full (owned-construction into `RouteConflictKey`/`SpawnRunnerPlan`/`JournalEntry`/export records, cfg(test) `Arc<Mutex>` capture plumbing, test-fixture spec cloning)and the  294 `to_owned`/`to_vec`/`to_string`/`Arc` hits sampled every-8th (37 rows read) - all owned-string construction at wire/error boundaries, test fixtures, or the cfg(test) capture channel; no borrow-checker-silencing clone found; parsers taking `impl Into<String>` (route.rs:413/418) require the owned String by callee contract, so those clones are not avoidable at the call site.



## type
- clean: seeds ran:  13/0/0 - all 13 `validate_*`/`check_*` hits (mkfs binary path, existing image type/size/identity/posture, route state, artifact-with-helper, target path) validate external filesystem/leader state that a parsed type cannot carry, and each runs once at its boundary or on mutable kernel state - no illegal constructible state to encode; no bool flag fields nor stringly-typed state in lane.



## api
- d2b-broker-p4#1 sev=medium blast=leaf effort=S verdict=actionable - `RouteConflictKey` is exported as `d2b_broker::ops::route::RouteConflictKey` (pub struct with all-pub fields) but its only users are private fns in the same file; its companion record type `RouteOwnershipRecord` is private - the visibility is a leak, not a contract with callers - fix: make it `pub(crate)` or plain `struct` (all users are in-file private helpers: `route_conflicts`, `route_matches_record`, `requested_route_conflict_key`)and drop the pub fields to private - [packages/d2b-broker/src/ops/route.rs:19]
  evidence: seed `\bpub (fn|struct|enum|trait|type|const|mod) ` = 77 hits; census: `\bRouteConflictKey\b` over packages/ = 1 file (only its defining file)

## err
- clean: seeds ran:  334/89/9/7 - combined  426 hits, sampled every-9th (48 rows read) plus all  9 panic-family hits and all  7 error enums read in full; the sampled hits are cfg(test) asserts/fixtures, deliberate best-effort `let _ = send/remove` cleanup, and invariant panics (`unreachable!` on limiter-stall/non-ext4 classification, `expect("ETXTBSY error recorded")` on a loop invariant, `expect("typed handoff serializes")` on derive-Serialize wire types) - all inside the acceptable panic-policy classes; the  7 error enums have Display/Error impls and are matched by variant, not by string; `DiskInitError` deliberately converts to `io::Error` with kind mapping (InvalidData/Other) plus actionable Display guidance - a sound coarse boundary for the `io::Result` op signature.



## serde
- clean: seeds ran:  11/10/0/47 - all derive/attr hits checked (route record `rename_all="camelCase",deny_unknown_fields`, store_sync_audit enums `snake_case` pinned by the signed-schema test, `deny_unknown_fields` on broker-written round-trip records)and the  47 `serde_json` sites are boundary parses with `map_err` to `io::Error`/`OpError::InvalidInput` or deliberate corruption surfacing (`serde_json::from_str::<Value>)...).ok()` at audit.rs:1830 becomes a typed `AuditExportEntry { error: Some)...) }`), so failures are never silently swallowed.



## obs
- clean: seeds ran:  0/0/0/2 - the only telemetry sites are two `tracing::warn!` calls in audit.rs (queue-full drop accounting, rate-limiter warning), both with named fields (`audit_drop_reason`, `audit_class`, `operation`, counters) - structured events as the skill requires; zero `println!`/`eprintln!` and no interpolated message-only events in lane.



## docs
- d2b-broker-p4#2 sev=medium blast=leaf effort=S verdict=actionable - audit.rs public surface gaps:`AuditEntry` (legacy JSONL record shape consumed by the socket-acl gate), `AuditDropSummary`, `AuditLog::open` (the daemon entry point with bootstrap/poison barrier semantics), and `audit_drop_summary` carry no doc comment, while every sibling method around them is documented - fix: add `///` first-sentence contracts (state what `disposition` vs `outcome` mean, what counters `AuditDropSummary` merges, what `open`'s barrier requires of callers) - [packages/d2b-broker/src/audit.rs:124, packages/d2b-broker/src/audit.rs:84, packages/d2b-broker/src/audit.rs:416, packages/d2b-broker/src/audit.rs:1029]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 77 hits; direct reads of lines 78-135/405-425/1025-1035 show no preceding doc comment on these four items
- d2b-broker-p4#3 sev=low blast=leaf effort=S verdict=actionable - tap.rs pub test-support surface lacks item docs:`SetBridgePortFlagsRequest`, `SetBridgePortFlagsAudit`, `set_bridge_port_flags`, `LiveCreateTapOutcome`, `LiveSetBridgePortFlagsError` have no `///` comments (the module-top doc explains the op family, but these exported shapes are the L1c canary-test contract)and should carry one-line first-sentence docs (error enum: list what each variant means to a caller) - [packages/d2b-broker/src/ops/tap.rs:157, packages/d2b-broker/src/ops/tap.rs:163, packages/d2b-broker/src/ops/tap.rs:169, packages/d2b-broker/src/ops/tap.rs:184, packages/d2b-broker/src/ops/tap.rs:828]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 77 hits; direct reads of tap.rs lines 94-190 and 819-865 show no doc comments on these five items
- d2b-broker-p4#4 sev=medium blast=leaf effort=S verdict=actionable - `HandoffOperationError` (pub enum returned from every pub handoff fn) has no top-level doc comment; the failure modes (`JournalMismatch`, `HelperUnavailable`, `ArtifactValidationOutputInvalid`, ...) have descriptive names but no contract sentence says what callers should do per variant - fix: add a `///` doc line listing the variant classes (journal replay vs helper/validation failures) - [packages/d2b-broker/src/ops/host_generation_handoff.rs:35]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 77 hits; direct read of handoff.rs lines 27-47 shows no doc comment above the enum
- d2b-broker-p4#5 sev=medium blast=leaf effort=S verdict=actionable - `ApplyWithPreflightError` (pub enum returned from the pub `apply_with_preflight_owned` entry point) has no top-level doc (only one variant carries an inline `///`); callers get no contract sentence distinguishing query failures from foreign-route refusal from reconcile failures - fix: add a one-line enum doc naming what each variant class means (route query vs ownership refusal vs executor failure) - [packages/d2b-broker/src/ops/route.rs:29]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 77 hits; direct read of route.rs lines 18-46 shows no doc comment above the enum

## perf
- clean: seeds ran:  124/20/79 - combined 223 hits sampled every-5th (45 rows read) - all sampled format!/to_string/Vec::new sites are error-path messages, audit-JSONL rendering, one-shot op-arg construction (ip link args, macvtap device paths), test fixtures, or empty-case-common buffers (`read_bounded_line`)) - all inside the card's recorded false-positive classes; no hot-loop allocation site found (lanes are one-shot broker ops, not per-packet paths).



## conc
- clean: seeds ran:  2/6/11/0 - the 2 thread sites are the dedicated audit worker spawn (std::thread::Builder at audit.rs:446, bounded sync_channel receiver per plan R4 - sanctioned) and a test thread-name read; the  6 `Arc<Mutex<` hits are all cfg(test) capture plumbing, and the  11 Atomic/Ordering hits are Relaxed drop counters on the audit worker boundary (dropped_privileged/unprivileged) plus a test scratch-id counter - all in the weakest-correct-ordering class; no prod Mutex/RwLock, no manual Send/Sync impls in lane.



## async
- d2b-broker-p4#6 sev=high blast=leaf effort=M verdict=actionable - `acquire_handoff_lock` is an `async fn` whose final step is a blocking `nix::fcntl::Flock::lock(file, LockExclusive)` on the executor worker thread; the call is not in the clippy.toml disallowed-methods list (no flock entry), not caught by the async-gate scanner) (qualified associated-function calls aren't the method-call shape the scanner flags; the hatch inventory records no marker at this line), and not in the blocking-census baseline - yet the repo's own clippy.toml names this exact class as a rule violation ("a synchronous lock acquired inside an async context ... still parks the executor worker" clippy.toml:55-56) - fix: move the flock to a dedicated bounded worker (house loader_worker shape per clippy.toml:37-40) or convert to non-blocking `LockExclusiveNonblock` plus async retry (`tokio::time::timeout`/sleep as the mkfs ETXTBSY loop does), keeping the critical section bounds off the runtime worker - [packages/d2b-broker/src/ops/host_generation_handoff.rs:246, packages/d2b-broker/src/ops/host_generation_handoff.rs:231]
  evidence: seed `async fn|async move|\.await` = 235; combined async seeds 258, sampled every-6th + full async-body reads; `Flock::lock` at handoff.rs:246; no `// async-gate-allow:` marker recorded in packages/xtask/data/async-gate-inventory.json for that line;`rg Flock packages/xtask/data/blocking-census-baseline.json` = 0 hits; the deny list (clippy.toml:53-177) names no flock API

## unsafe
- d2b-broker-p4#7 sev=low blast=leaf effort=S verdict=actionable - `command_output_inheriting_fd_async` wraps a std )safe) call (`std::process::Command::pre_exec`) in an `unsafe { ... }` block (plus a crate-level-exception `#[allow(unsafe_code)]`), making the block and the allow unnecessary:the pre_exec closure contract (async-signal-safe, error-returning) is already std's own safe-API contract,and the closure body uses only safe nix fcntl wrappers - fix: remove the `unsafe { }` block and the `#[allow(unsafe_code)]` attribute, keeping the async-signal-safety rationale as a regular comment (the site then leaves the enumerated unsafe-exception set in U1 (d)8, shrinking it) - [packages/d2b-broker/src/ops/disk_init.rs:673, packages/d2b-broker/src/ops/disk_init.rs:661]
  evidence: seed `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 1 (disk_init.rs:673), plus `// SAFETY:` = 1 and `unsafe_code` = 1); direct read:the block contains only `command.pre_exec)...)`, a safe std method, so no unsafe operation requires the block

## ffi
- clean: seeds ran:  0/0/0/10 - the 10 `CString`/`c_char` hits are all in spawn_runner.rs's execve-vector construction (`build_cstring_vectors`, `path_to_cstring`) - libc syscall wrapper data, no `extern "C"` entry point, no exported boundary, no handle ownership, no panic-across-FFI surface - the card's "libc-binding call sites that never cross a foreign caller" false-positive class; no finding.



## macro
- N/A: seeds ran:  0/0/0/0 all zero - no `macro_rules!` definition, no proc-macro/syn/quote usage, no `$crate`, no compile-error macro machinery in lane (the crate's only macros (e.g. `redacted_debug!`)) live in other modules outside this part's scope)



## test
- clean: seeds ran: combined 738 over src+tests - sampled every-10th src (47 rows) and every-8th tests (34 rows) plus full read of tests/broker_export_audit.rs; the sampled assertions pin behavior (sequence numbers, error kinds, wire op fields, canonical digest formats, quota/caps, round-trip shapes, audit redaction) rather than implementation text;; test names from the item maps are behavioral scenarios (crash-between-records, rollback-keeps-writer-usable, foreign-route-refusal, forged-marker-rejection, replay-safe-handoff); zero `#[ignore]` hits, no proptest/insta/rstest usage, deterministic scratch roots (TEST_TMPDIR + static atomic/pid suffixes, injected io failures, frozen rate-limit windows) - no test that cannot fail found in lane

## Coverage
- idiom: clean (seeds ran:  10/0/7; retry/test loops and justified accumulation loops)
- own: clean (seeds ran:  83/288/6/0; all clones read + 294 non-clone hits sampled every-8th (37 rows); no avoidable clone found)
- type: clean (seeds ran:  13/0/0; validators check external mutable state, not constructible state)
- api:  1 finding(s)
- err: clean (seeds ran:  334/89/9/7; 426 hits sampled every-9th (48 rows); sampled hits are tests/invariant asserts)
- serde: clean (seeds ran:  11/10/0/47; boundary parses + deliberate deny_unknown_fields/rename_all)
- obs: clean (seeds ran:  0/0/0/2; both tracing sites use named fields)
- docs:  4 finding(s)
- perf: clean (seeds ran:  124/20/79; 223 hits sampled every-5th (45 rows); all in card false-positive classes)
- conc: clean (seeds ran:  2/6/11/0; dedicated worker + cfg(test) sync + Relaxed counters)
- async:  1 finding(s)
- unsafe:  1 finding(s)
- ffi: clean (seeds ran:  0/0/0/10; libc execve vector construction, no boundary)
- macro: N/A (seeds:  0/0/0/0 all zero; no macro machinery in part scope)
- test: clean (seeds ran: combined 738 over src+tests); sampled 81 rows; assertions behavioral, no ignored tests)