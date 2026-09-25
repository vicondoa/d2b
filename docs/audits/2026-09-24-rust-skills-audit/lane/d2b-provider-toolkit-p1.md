# d2b-provider-toolkit-p1 - d2b-provider-toolkit - part 1/2
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 6919 (excl. src/generated/**) | modules: base (bootstrap, error, fd10, guest, mod, runtime, startup), plane (creations, handle, mod, reconcile), operations (envelope, mod), audit (mod, redaction), declaration (manifest, mod, schema), bin (d2b-provider-toolkit)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: src/base/**, src/plane/**, src/operations/**, src/audit/**, src/declaration/**, src/bin/**

## idiom
- d2b-provider-toolkit-p1#1 sev=medium blast=leaf effort=S verdict=actionable - the 15-operation Guest backend allowlist is spelled out twice: `GuestCredentialBackend::request` inlines the same `matches!` that `valid_guest_backend_operation` already implements, so adding one operation to one list and not the other silently diverges the client and responder admission - fix: have `request` call `valid_guest_backend_operation(&operation)` and delete the inline `matches!` arm - [packages/d2b-provider-toolkit/src/base/fd10.rs:926-941, packages/d2b-provider-toolkit/src/base/fd10.rs:1478-1496]
  evidence: seed `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 1 hit; the duplicate list verified by `grep -n 'secret-service.state'` = 2 sites (fd10.rs:928, fd10.rs:1481) with identical 15-entry bodies
- d2b-provider-toolkit-p1#2 sev=low blast=leaf effort=S verdict=actionable - `GuestCredentialBackendResponse` and `GuestCredentialBackendReply` are two public 7-field structs with the same shape (state, lease_handle, source_version, rotation_generation, expires_at_unix_ms, outcome, bytes) and duplicated accessors, both re-exported at the crate root - fix: collapse into one type carrying the accessors plus `encode`/`with_sensitive_bytes`, keeping the zeroizing bytes field - [packages/d2b-provider-toolkit/src/base/fd10.rs:545-597, packages/d2b-provider-toolkit/src/base/fd10.rs:612-700, packages/d2b-provider-toolkit/src/lib.rs:91-92]
  evidence: seed `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 1 hit; both structs and their accessor blocks read in full (fd10.rs:545-700), field lists identical
- clean: seeds `for \w+ in 0\.\.` = 2 (a bounded reconnect loop and a test loop, both index-appropriate), `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 1 (manual `Default` for `ProviderAgentAuditLog` preserving the frozen capacity invariant, deliberate), `let mut \w+ = (String|Vec)::new\(\)` = 7 (all cold build/collect loops); hand-written `Debug` impls that redact are the deliberate house pattern

## own
- d2b-provider-toolkit-p1#3 sev=low blast=leaf effort=S verdict=actionable - `is_ready_for_route` clones the retained route binding out of the mutex guard (`and_then(|ready| ready.clone())`) only to compare it, allocating the binding's strings on every route check - fix: compare through the guard, `try_lock().ok().is_some_and(|ready| ready.as_ref().is_some_and(|bound| bound.liveness().is_live() && bound == route))`, no clone - [packages/d2b-provider-toolkit/src/base/runtime.rs:530-537]
  evidence: seed `\.clone\(\)` = 64 hits; this is the only clone in the sync route-query path that borrows instead of owning (the sibling `ready_route()` clone is required to return an owned value)
- clean: 64 clones, 37 `to_owned`/`to_vec`/`to_string`, 4 `Arc<Mutex<...>>` (the envelope's shared audit ring and the backend state, genuine multi-owner runtime state), 0 `Rc`/`RefCell`/`Cow`; remaining clones are route-metadata snapshots, per-invocation audit records, and test fixtures, each explainable

## type
- clean: seeds `fn validate_\w+|fn check_\w+` = 5 (all boundary admission checks: route validation, facet validation, manifest installation validation - parse-once at the boundary, correct), `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0; the `ProviderEntrypoint` builder's seven `Option` fields are guarded against double-binding by each `with_*` method and validated at use, within the stopping rule

## api
- d2b-provider-toolkit-p1#4 sev=medium blast=leaf effort=S verdict=actionable - test-only constructors `GuestCredentialBackend::from_socket_for_test` and `from_socket_for_test_with_route` sit on the public surface (the type is re-exported at the crate root) without the house `test-support` feature gate that `d2b-session` uses for the same class of export - fix: move both behind a `test-support` feature (or `#[doc(hidden)]` + `#[cfg(any(test, feature = "test-support"))]`) so downstream crates cannot rely on them - [packages/d2b-provider-toolkit/src/base/fd10.rs:888, packages/d2b-provider-toolkit/src/base/fd10.rs:901, packages/d2b-provider-toolkit/src/lib.rs:89]
  evidence: seed `pub .*\b(Arc|Rc|Box|RefCell)<` = 2 hits (this site and the deliberate `SharedClock = Arc<dyn Clock>` alias); census: `from_socket_for_test` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel/*.bzl = 5 hits, all in test code (d2b-provider-toolkit/tests/supervised_runtime.rs:94,550,642, d2b-provider-credential-secret-service/tests/session.rs:677, d2b-provider-credential-secret-service/src/lib.rs:1942,1958)
- clean: 262 pub items, 2 `Arc`-in-signature hits (one test constructor, one deliberate clock-seam alias), 14 `pub use` re-export arms matching the house single-surface pattern; no dependency types leak into signatures beyond the declared vocabulary re-exports

## err
- d2b-provider-toolkit-p1#5 sev=medium blast=leaf effort=S verdict=actionable - the refused-forwarded-invocation audit is silently dropped for the documented U10 wire spelling: `invoke_named_with_fds_under_chain` audits the raw caller string, and `audit_named` returns when `BoundedToken::parse` fails, but the forwarded family names are PascalCase (`OpenPidfd`), which the `^[a-z][a-z0-9-]*$` token grammar rejects, so the Denied record the module contract promises for every refused invocation never lands for the uncommitted forwarded path - fix: audit the canonicalized name (lowercase/dash-strip before `BoundedToken::parse`, or audit the resolved entry's `operation.name()` when an entry exists) and add a harness case asserting the PascalCase forwarded spelling records a Denied event - [packages/d2b-provider-toolkit/src/operations/envelope.rs:429-433, packages/d2b-provider-toolkit/src/operations/envelope.rs:495-497, packages/d2b-provider-toolkit/src/operations/envelope.rs:341-346]
  evidence: seed `\.unwrap\(\)|\.expect\(` = 105 (all remaining sites are `#[cfg(test)]` or invariant expects on literally-built values); `BoundedToken::parse` grammar at packages/d2b-contracts-resource/src/v3/execution_policy.rs:191; forwarded wire spelling "OpenPidfd" pinned by the envelope's own U10 doc and the committed catalog row at packages/d2b-broker/src/generated/broker_operation_catalog.rs:2632; the harness covers only the lowercase spelling (tests/harness.rs:501-522), so no test exercises the dropped path
- clean: `let _ = |\.ok\(\);` = 4 (deliberate best-effort watch cancels and the cfg(not) discard), `panic!|unreachable!|todo!|unimplemented!` = 0, `enum \w*Error` = 11 (closed code-carrying sets with `code()` + Display, split by caller action); `commit_grant`'s fail-closed `try_write` drop is the documented U4 pattern, not a finding

## serde
- clean: seeds `derive\([^)]*(De)?[Ss]erialize` = 5, `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` = 20, `impl .*Deserialize.*for` = 0, `serde_json::from_|serde_json::to_` = 13; every wire type carries `deny_unknown_fields` + `rename_all = "camelCase"`, optionality uses `#[serde(default)]` deliberately, `skip_serializing_if = "Option::is_none"` on the reply wire, decode sites validate protocol markers and route binding, and all parse failures map to closed error codes; the hand-written `Drop` zeroizing `CredentialDeliveryKeyWire` is deliberate

## obs
- d2b-provider-toolkit-p1#6 sev=low blast=leaf effort=S verdict=actionable - `serve_enrolled` drops a base-side wire-contract violation with no event: a frame that fails `GuestFrame::new` (empty or oversized, i.e. a peer protocol violation, not an agent refusal) is `continue`d silently, and the module doc only covers agent refusals as "the agent's to record", so the malformed frame is invisible to the operator - fix: emit a `warn!` with the frame length before dropping, keeping the session up - [packages/d2b-provider-toolkit/src/base/guest.rs:494-498, packages/d2b-provider-toolkit/src/base/guest.rs:446-452]
  evidence: seed `\bprintln!\(|\beprintln!\(` = 5 (all process-entrypoint product output, `CLI-only path` allows), interpolated-message seed = 0, `tracing::` = 3; the drop sites read in full at guest.rs:490-499
- clean: all tracing events use named fields (`warn!(name, provider, expected_zone, ...)`, `debug!(generation, ...)`); the five `println`/`eprintln` sites are CLI/process-entrypoint output, not telemetry

## docs
- d2b-provider-toolkit-p1#7 sev=low blast=leaf effort=S verdict=actionable - key `Result`-returning public items document no failure conditions: `ProviderEntrypoint::new` (InvalidName), `admit` (NotAccepting), the three `with_*` binders, and `StartupPlan::derive`/`declare` (MissingInput/DuplicateOutput/Cycle) have one-line docs with no `# Errors` section or prose naming the refusal, so callers must read the error enum to learn when construction fails - fix: add `# Errors` sections (or one prose sentence naming the refusal) to the entrypoint constructors and the plan derivation - [packages/d2b-provider-toolkit/src/base/runtime.rs:248-249, packages/d2b-provider-toolkit/src/base/runtime.rs:383-384, packages/d2b-provider-toolkit/src/base/startup.rs:39]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 254, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 105; `#![deny(missing_docs)]` is on (lib.rs:46), so every item has a comment but failure conditions are prose-absent on the named items
- clean: first sentences are strong throughout (no `get_` accessors, no name-echoing openers), module docs present in every module, redaction and non-authorization contracts documented; the zero canonical-section count is house style, only the failure-condition gap is flagged

## perf
- clean: seeds `format!\(` = 13 (error diagnostics, startup `Provider/{}` refs, invocation-id minting, tests - all cold), `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 13 (bounded build/collect loops; the two receive loops cap at 64 KiB and 4 KiB), `\.to_string\(\)` = 4 (per-request ttrpc metadata values, cold); all sites static (unmeasured), no allocation in a per-packet or per-frame loop

## conc
- d2b-provider-toolkit-p1#8 sev=low blast=leaf effort=S verdict=actionable - `invocations.fetch_add(1, Ordering::AcqRel)` uses release-acquire for a monotonic counter nobody synchronizes on; the identifier only needs uniqueness, so `Ordering::Relaxed` is the weakest correct ordering - fix: `fetch_add(1, Ordering::Relaxed)` - [packages/d2b-provider-toolkit/src/operations/envelope.rs:487]
  evidence: seed `Atomic\w+|Ordering::` = 23; the counter's only reader is the minted id itself (envelope.rs:485-488), no paired load; contrast the load-bearing `admitted`/`lifecycle` atomics in base/runtime.rs, whose AcqRel/Acquire pairing is documented and deliberate
- clean: `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 7 (tokio Mutex/RwLock held across awaits, correct choice; the std `Mutex` on the audit ring is documented as the frozen constructor contract), `thread_local!|unsafe impl (Send|Sync) for` = 0; the `admitted` counter, `lifecycle` state machine, and `bound` bind-once flag carry written ordering arguments

## async
- clean: seeds `async fn|async move|\.await` = 120, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 2 (the credential-backend responder task, cancel-safe via watch channels, and one test spawn), `tokio::sync::(Mutex|RwLock|Notify)` = 5, `#\[tokio::(main|test)\]|Runtime::block_on` = 2 (tests); the drain wait arms the `Notify` before checking the count and bounds with `tokio::time::timeout` (the sanctioned shape), the backend state lock is a tokio Mutex across awaits, `serve_enrolled` uses a biased `select!`, and the three process entrypoints `block_on` on the calling thread with `CLI-only path` allows

## unsafe
- N/A: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0; the manifest sets `unsafe_code = "forbid"` (packages/d2b-provider-toolkit/Cargo.toml:13), and no `unsafe_code` allow exists in the part

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0; the crate crosses no foreign boundary

## macro
- N/A: seeds `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0

## test
- clean: seeds over src = `#\[test\]|#\[tokio::test\]` = 22, asserts = 65, `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0; over tests/ = 46 test attrs, 185 asserts, 0 property/snapshot tooling, 0 ignored; the suites assert behavior (exact closed refusals, byte-identical canonical emission against a committed digest vector, offset parity between CLI and library verification, drain/readiness lifecycle, zeroizing round trips) rather than implementation, and no test computes its expectation with the code under test

## Coverage
- idiom: 2 finding(s)
- own: 1 finding(s)
- type: clean (seeds ran: 5/0/0; all five validation fns are boundary admission checks)
- api: 1 finding(s)
- err: 1 finding(s)
- serde: clean (seeds ran: 5/20/0/13; wire types deny unknown fields, camelCase, closed-code failures)
- obs: 1 finding(s)
- docs: 1 finding(s)
- perf: clean (seeds ran: 13/13/4; all cold paths, static)
- conc: 1 finding(s)
- async: clean (seeds ran: 120/2/5/2; sanctioned Notify/timeout drain shape, tokio Mutex across awaits, cancel-safe responder)
- unsafe: N/A (seeds: 0/0/0 all zero; manifest forbids unsafe_code)
- ffi: N/A (seeds: 0/0/0/0 all zero; no foreign boundary)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 22/65/0/0 src + 46/185/0/0 tests/; behavioral boundary and round-trip suites)