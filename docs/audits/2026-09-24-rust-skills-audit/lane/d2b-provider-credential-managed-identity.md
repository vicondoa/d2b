# d2b-provider-credential-managed-identity - d2b-provider-credential-managed-identity
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 5439 (excl. src/generated/**: none) | modules: whole crate (agent, audit, controller, lib, service, telemetry; tests/*)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none (single-part lane)

## idiom
- clean: seeds ran:  0/0/0 (index loops, hand-written impl a derive may replace, statement-style accumulation); no hits; expression shapes all idiomatic.

## own
- clean: seeds ran:  37/8/8/0; every clone/to_owned site read: Arcs clone only at `async move` boundaries to avoid borrowing self (lib.rs:210, lib.rs:251, lib.rs:280, lib.rs:309, lib.rs:338); owned constructs (checkpoint/lease/record building) genuinely need owned fields; restore_checkpoints clones the lease map wholesale to keep the original intact on early error (lib.rs:1344); no Cow; all explainable.



## type
- d2b-provider-credential-managed-identity#1 sev=medium blast=leaf effort=S verdict=actionable - `ManagedIdentityTeardownPlan` exposes its three bools as pub fields, letting a caller construct invalid combos (`stop_agent && delete_agent`, `delete_agent && clear_provider_revoke` that `teardown_plan` never emits - fix: make the fields private with `pub const fn` accessors (or replace with an ordered stage enum); update the literal constructions in tests/binding.rs:1214-1234 - [controller.rs:85-89, tests/binding.rs:1214-1234]
  evidence: static: seed 2 (`is_\w+: bool|\w+_flag: bool`) =0 but the pub bool triad at controller.rs:85-89 admits invalid states; literal constructions in tests/binding.rs:1214-1234 prove the surface constructible;`teardown_plan` (controller.rs:158-166)is the only in-crate producer and never emits them.



## api
- d2b-provider-credential-managed-identity#2 sev=low blast=leaf effort=S verdict=actionable - `ManagedIdentityPlacement::in_zone` is an exact duplicate constructor of `new` with zero callers anywhere in the repo, doubling the public construction path - fix: delete `in_zone` (and its docs at lib.rs:611-618); `new` already validates and names the behavior - [lib.rs:612-618]
  evidence: census: `in_zone\\(` over packages/nixos-modules/tests/docs/reference/labs/BUILD.bazel/*.bzl = definition-only (lib.rs:612); no caller; api seeds  86/0/2 (pub items/pub-internals-in-signatures/pub use; the pub use re-exports are the house single-surface pattern (lib.rs:40-45)).

## err
- d2b-provider-credential-managed-identity#3 sev=low blast=leaf effort=S verdict=actionable - `export_checkpoints` panics via `.expect("lease map keys are validated Credential refs")` where every sibling invariant failure in the crate map_errs to `CredentialServiceErrorCode::InvariantFailure` - fix: replace with `.map_err(|_| invariant())?` (leveragingthe existing `invariant()` helper at lib.rs:1491) - [lib.rs:1261-1262]
  evidence: err seed 1 (`\.unwrap\(\)|\.expect\(`) = 20 hits; 19 sit in `#[cfg(test)]`; 1 non-test hit: lib.rs:1262.



## serde
- N/A (seeds:  0/0/0/0 all zero; the crate crosses the guest backend only by building `serde_json::json!` values, no serde-derived type, hand-written deserializer, or from_/to_ boundary sits here).

## obs
- clean: seeds ran:  0/0/0/26 (tracing-presence); every tracing event carries named fields (`provider`, `resource`, `operation`, `state`, `%error`); no println, interpolated message-only event, or instrument site; library installs no subscriber.





## docs
- d2b-provider-credential-managed-identity#4 sev=low blast=leaf effort=M verdict=actionable - Public `Result`-returning items document failures only in prose and carry no `# Errors` sections (e.g. `ManagedIdentityClientConfig::new`, `ManagedIdentityPlacement::new`, `ImdsEndpointAlias::parse`, `ManagedIdentityCredentialProviderFactory::new`, controller projections) - fix: add `# Errors` sections naming the specific `ManagedIdentityProviderError`/`CredentialServiceError`/`CredentialObservabilityError` variant each failure returns - [lib.rs:449, lib.rs:514, lib.rs:592, lib.rs:796, controller.rs:137, controller.rs:171, controller.rs:203, controller.rs:227]
  evidence: docs seeds  86/0/43; seed 2 (`/// # (Examples|Errors|Panics|Safety)`)=0 while 43 pub items return `Result<`;`#![deny(missing_docs)]` (lib.rs:7) forces doc presence but not canonical sections.

## perf
- clean: seeds ran:  3/2/0; all `format!` sites are test canaries (audit.rs:39, lib.rs:1531, telemetry.rs:34); `Vec::new` at construction and at the restore buffer (lib.rs:819, lib.rs:1283, both cold/empty-case-true); no to_string copies; no allocation in a hot path.



## conc
- clean: seeds ran:  1/3/0/0; the sole thread hit is the `use std::thread` import for `poll_client_sync`'s park/unpark waker ( lib.rs:22); std `Mutex` is try_lock-only on synchronous surfaces (lib.rs:902, lib.rs:1094-1103)andthe `tokio::sync::Mutex` pairs guard state across awaits( lib.rs:901, lib.rs:903); no atomics, scoped/spawned threads,or manual Send/Sync.



## async
- clean: seeds ran: ~50/0/4/0; all awaits occur with tokio-aware locks, no guard is held across an await (acquire/refresh/inspect re-acquire per section);`await_client` bounds every injected-client future with `tokio::time::timeout`( service.rs:566-577);`poll_client_sync` is the deliberate synchronous surface documented at lib.rs:1238-1245 (try_lock + park_timeout per plan U4; no spawn/select/runtime-in-library).





## unsafe
- N/A (seeds:  0/0/0; seed4=1 only the `#![forbid(unsafe_code)]` attribute at lib.rs:8, which per the lens card doe not make the lens applicable).

## ffi
- N/A (seeds:  0/0/0/0 all zero; nothing crosses a foreign caller).

## macro
- N/A (seeds:  0/0/0/0 all zero; no macro_rules, proc-macro usage, or `$crate` paths).

## test
- d2b-provider-credential-managed-identity#5 sev=low blast=leaf effort=S verdict=actionable - Table-driven loops assert without per-case failure messages, so the first failing case reports only a shared line number and not which case - fix: append `"method: {method:?}"` / `"binding: {binding:?}"` style messages to the `assert!`/`assert_eq!` calls in the method/route/placement matrices (mirroring the canary loops' messages at canary.rs:229-241) - [tests/conformance.rs:77-82, tests/topology.rs:33-38, tests/topology.rs:68-76]
  evidence: test seeds ~52/~180/0/0; the three named loops are the only assertion-loops without per-case messages; the suite otherwise asserts behavior (binding subject matrices, canary redaction scans, fault-injection fakes, conformance fixtures) and has no proptest/insta/rstest or `#[ignore]` tests.

## Coverage
- idiom: clean (seeds ran:  0/0/0
- own: clean (seeds ran:  37/8/8/0
- type:  1 finding(s)
- api:  1 finding(s)
- err:  1 finding(s)
- serde: N/A (seeds:  0/0/0/0 all zero; no serde type boundary
- obs: clean (seeds ran:  0/0/0/26
- docs:  1 finding(s)
- perf: clean (seeds ran:  3/2/0
- conc: clean (seeds ran:  1/3/0/0
- async: clean (seeds ran: ~50/0/4/0
- unsafe: N/A (seeds:  0/0/0; seed4=1 only the forbid attribute
- ffi: N/A (seeds:  0/0/0/0
- macro: N/A (seeds:  0/0/0/0
- test:  1 finding(s)