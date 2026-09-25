# d2b-provider-guest-qemu-media - d2b-provider-guest-qemu-media
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 3,632 (src 2,688 + tests 944 (excl. src/generated (none) | modules: whole crate
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate (single-part lane)

## idiom
- d2b-provider-guest-qemu-media#1 sev=low blast=leaf effort=S verdict=actionable - The `impl Default` bodies re-spell the serde `default_*` helper values in a second place (`"qemu-system-x86-64".to_owned()` at packages/d2b-provider-guest-qemu-media/src/config.rs:93 vs `default_qemu_artifact()` at 272; the whole GuestProviderSpecSettings default body at packages/d2b-provider-guest-qemu-media/src/types/guest.rs:182-196 vs the serde default fns at ~440-447); two spellings of one default drift independently - fix: have the Default impls call the serde default fns (`qemu_binary_artifact_id: default_qemu_artifact()`, `vcpu: default_vcpu()`, `boot_media_view: default_boot_media_view()`( ( - [packages/d2b-provider-guest-qemu-media/src/config.rs:93, packages/d2b-provider-guest-qemu-media/src/config.rs:272, packages/d2b-provider-guest-qemu-media/src/types/guest.rs:182, packages/d2b-provider-guest-qemu-media/src/types/guest.rs:440]
  evidence: seed2 (`impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`= 4 hits;(2 of the 4 hand-written Default impls duplicate the deserialization defaults (the other 2 are deliberate non-derivable (reconcile.rs:146 display_ready=true (qmp/mod.rs:164 delegates to `new()`. 
- clean: seeds ran:  0/4/1;0 index loops;(4 hand-written Default impls (2 flagged as #1;(1 `let mut ... = Vec::new()` accumulation (flagged as perf#1; nothing else found.

## own
- d2b-provider-guest-qemu-media#2 sev=low blast=leaf effort=S verdict=actionable - `QmpSession::execute` clones every dispatched QmpCommand into the bounded history before executing (packages/d2b-provider-guest-qemu-media/src/qmp/mod.rs:266); per-field borrow splitting makes the clone avoidable: push the owned command into `commands` and execute from `commands.back()` (`let Self { transport, commands, .. } = self;` then `commands.push_back(command); transport.execute(commands.back().expect("just pushed"))`(removes 1-4 String copies per QMP command - fix: destructure the two fields and reorder push/execute (drop `command.clone()` - [packages/d2b-provider-guest-qemu-media/src/qmp/mod.rs:266]
  evidence: seed1 (`.clone()`= 9 hits;(8 of the 9 clones are required ownership moves (projections at config.rs:157, ticket slots at process_builder.rs:244,255, recovery state at reconcile.rs:297, launch-ticket process at 447-448, expected-identity persistence at 495, feature dedup at types/guest.rs:228 (this history-copy is the only avoidable one (the skill's borrow-splitting pattern.



- clean: seeds ran:  9/30/0/0;(the 30 to_owned/to_vec/to_string sites are wire-string construction and owned conversions for wire fields (fine;(no Rc/RefCell/Arc<Mutex>/Cow.

  

## type
- d2b-provider-guest-qemu-media#3 sev=medium blast=leaf effort=M verdict=actionable - `validate_token` (packages/d2b-provider-guest-qemu-media/src/types/guest.rs:417(re-implements exactly the bounds of the in-tree `BoundedToken::parse` (`^[a-z][a-z0-9-]*$`, up to 63 bytes (at packages/d2b-contracts-resource/src/v3/execution_policy.rs:187-191); a second, slightly looser copy lives in `validate_object_id` (packages/d2b-provider-guest-qemu-media/src/qmp/mod.rs:274) - fix: replace the 7 gate calls (config.rs:142, hotplug.rs:62, process_builder.rs:288, volume.rs:121,165, guest.rs:115,213(with `BoundedToken::parse)...).is_ok()` (route qmp's variant through a first-char check plus the helper), delete the local helper - [packages/d2b-provider-guest-qemu-media/src/types/guest.rs:417, packages/d2b-contracts-resource/src/v3/execution_policy.rs:187, packages/d2b-provider-guest-qemu-media/src/config.rs:142]
  evidence: seed1 (`fn validate_\w+|fn check_\w+`= 3 hits;(validate_token definition + 7 call sites =8 matches over src/; exact bound match with the BoundedToken doc (execution_policy.rs:190-191.
- d2b-provider-guest-qemu-media#4 sev=medium blast=leaf effort=S verdict=actionable - Public `impl Default for ProviderConfig` manufactures an invalid config (`controller_execution_ref: ResourceRef::parse("Guest/invalid").expect)...)` at packages/d2b-provider-guest-qemu-media/src/config.rs:92), which fails its own `validate()` ); its only consumer is a test asserting that invalidity - fix: delete the Default impl (and rewrite the test to build valid-then-mutated configs as its sibling test at tests/config_schema_projection.rs:27 already does), or replace with a `#[doc(hidden)]` `for_test()`-style constructor - [packages/d2b-provider-guest-qemu-media/src/config.rs:89, packages/d2b-provider-guest-qemu-media/tests/config_schema_projection.rs:5]
  evidence: census: `ProviderConfig::default` over packages/,nixos-modules/,tests/,docs/reference/,labs/=1 hit (src: 0 (tests:  1 (packages/d2b-provider-guest-qemu-media/tests/config_schema_projection.rs:5);(seed2 (`impl Default for`= 4 hits;(this is the sole Default violating its own validate).
- clean: seeds ran:  3/0/2;2 stringly-typed state String fields (volume.rs:31,75 (are mirror images of the v3 Volume wire-contract String fields (false positive (not flags (no boolean-flag soup (otherwise clean.

## api
- d2b-provider-guest-qemu-media#5 sev=low blast=leaf effort=S verdict=actionable - Test-support exports `ScriptedQmpTransport` (packages/d2b-provider-guest-qemu-media/src/qmp/mod.rs:129(and `ProcessIdentity::for_test` (packages/d2b-provider-guest-qemu-media/src/adoption.rs:24(are unconditionally pub+re-exported with no consumer outside this crate's own tests (while the house convention for test-only items is `#[doc(hidden)]` (see `mark_ready_for_test` at packages/d2b-provider-guest-qemu-media/src/controller/reconcile.rs:327-328( - fix: mark both `#[doc(hidden)]` (or gate behind a `test-support` feature - [packages/d2b-provider-guest-qemu-media/src/qmp/mod.rs:129, packages/d2b-provider-guest-qemu-media/src/lib.rs:32, packages/d2b-provider-guest-qemu-media/src/adoption.rs:24]
  evidence: census: `ScriptedQmpTransport` over packages/,nixos-modules/,tests/,docs/reference/,labs/,BUILD.bazel/=3 hits (`src/qmp/mod.rs:129` def + `src/lib.rs:32` re-export + `tests/qmp_protocol.rs:2`); `for_test` over the same roots = 6 hits (1 definition + 5 test uses (no external crate references either.
- clean: seeds ran:  127/0/15;(the 15 `pub use` arms in lib.rs are the house single-surface re-export pattern (false positive;(no Arc/Rc/Box/RefCell in public signatures;(nothing else found.

## err
- clean: seeds ran:  5/1/0/9;5 unwrap/expect (4 on parsed literal constants inside Default/new (config.rs:92,99,101, volume.rs:99 (exempt (1 in #[cfg(test] (hotplug.rs:96 (exempt ( (1 swallowed Result (qmp/mod.rs:238 (deliberate best-effort rollback (logged at 233 (original error propagates ( (0 panic-family macros;(9 error enums all carry stable `code()` Display strings (closed taxonomy split by caller action (fine.

## serde
- clean: seeds ran:  18/38/0/4; all 38 serde attr sites obey rename_all + deny_unknown_fields + default/skip_serializing_if conventions (the 3 hand-written Deserialize impls (config.rs:45, guest.rs:122,238 (are the recorded live admission gates (refused class per docs/explanation/over-engineering-audit-record.md (do not re-flag ( (the boundary is covered by real-payload and round-trip tests (tests/config_schema_projection.rs:43, tests/guest_schema_roundtrip.rs:5 (fine.

## obs
- d2b-provider-guest-qemu-media#6 sev=low blast=leaf effort=M verdict=actionable - All 21 tracing events repeat the same two context fields (`resource = %self.guest_ref`, `provider = "runtime-qemu-media"`(inline ( ~20 sites in reconcile.rs + qmp/mod.rs:233); a span per reconcile/finalize would carry them once - fix: `#[tracing::instrument(skip(self, effect))]` on `QemuMediaController::reconcile`/`finalize` (or an explicit enter/exit span (dropping the duplicated pairs from the per-event fields - [packages/d2b-provider-guest-qemu-media/src/controller/reconcile.rs:352, packages/d2b-provider-guest-qemu-media/src/controller/reconcile.rs:380, packages/d2b-provider-guest-qemu-media/src/controller/reconcile.rs:605, packages/d2b-provider-guest-qemu-media/src/qmp/mod.rs:233]
  evidence: seed2 (`(info|debug|warn|error|trace)!\(`= 21 hits;(all use named fields but the resource/provider pair is repeated at every event;(seed3 (`\.instrument\(|#\[instrument`= 0 spans (none to inherit them.
- clean: seeds ran:  0/21/0/21;0 println!/eprintln! in src (library isolates from stdout;(the gated fixture test prints SKIP to stderr (test-only (fine;(all 21 events carry named fields (no interpolated message-only events (no spans (finding #6 ( (21 `tracing::` sites (same set.

## docs
- d2b-provider-guest-qemu-media#7 sev=medium blast=leaf effort=M verdict=actionable - None of the 41 `-> Result<` items carry a `# Errors` section (seed2 =0 ( (e.g. `DeviceAdmission::validate` has 6 failure kinds (device_watch.rs:82-90), `QemuMediaController::reconcile` 8 (reconcile.rs:338), `LaunchTicket::new` 3 (process_builder.rs:221) ), `QmpSession::negotiate` 3 (qmp/mod.rs:199) (leaving the caller to read the enum to map conditions - fix: add `# Errors` sections naming which conditions produce which variants on the non-obvious pub Result APIs - [packages/d2b-provider-guest-qemu-media/src/controller/device_watch.rs:82, packages/d2b-provider-guest-qemu-media/src/controller/reconcile.rs:338, packages/d2b-provider-guest-qemu-media/src/controller/process_builder.rs:221, packages/d2b-provider-guest-qemu-media/src/qmp/mod.rs:199]
  evidence: seed3 (`-> Result<`= 41 hits; seed2 (`/// # (Examples|Errors|Panics|Safety)`= 0 (no canonical sections anywhere.
- clean: seeds ran:  115/0/41;`#![deny(missing_docs)]` (lib.rs:3(keeps all 115 pub items documented (the 41 Result-returning items are the # Errors gap (finding #7 ( (module docs present at every module head (fine.

## perf
- d2b-provider-guest-qemu-media#8 sev=low blast=leaf effort=S verdict=actionable - `LaunchTicket::new` grows `attachments` by push from a fresh `Vec::new()` with an a-priori known upper bound (up to media_refs.len()+3 slots (packages/d2b-provider-guest-qemu-media/src/controller/process_builder.rs:239-274) - fix: `Vec::with_capacity(media_refs.len() + 3)` ( (static (unmeasured. - [packages/d2b-provider-guest-qemu-media/src/controller/process_builder.rs:239]
  evidence: seed2 (`Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)`= 10 hits;(this is the only grow-by-push site with an a priori bound (the rest are legit empty-case defaults (static (unmeasured.
- clean: seeds ran:  5/10/0;5 format! sites all one-shot cold paths (scaffold/id construction, short-key hex rendering (fine;(0 to_string copies in src (nothing else found.

## conc
- clean: N/A (seeds:  0/0/0/0 all zero; no threads, mutexes/rwlocks, atomics/orderings, or thread_locals in src (and no tokio::sync re-exports (lens inapplicable.

## async
- clean: N/A (seeds:  0/0/0/0 all zero; no async fn, .await, tokio::spawn/select/join, tokio::sync types, or block_on in src (this Provider's effect and QMP seams are deliberately synchronous (lens inapplicable.

## unsafe
- clean: N/A (seeds 1-3:  0/0/0 all zero; seed4 (`unsafe_code`= 2 (manifest `forbid` (Cargo.toml:9 (and crate-level `#![forbid(unsafe_code)]` (lib.rs:4) (per U1 card seed4 alone does not make the lens applicable (no unsafe sites.

## ffi
- clean: N/A (seeds:  0/0/0/0 all zero; no extern "C"/no_mangle, catch_unwind, repr(C/transparent, or C string types in src (the crate has no FFI surface.

## macro
- clean: N/A (seeds:  0/0/0/0 all zero; no macro_rules!/proc-macro/syn/quote machinery in src (lens inapplicable.

## test
- d2b-provider-guest-qemu-media#9 sev=low blast=leaf effort=S verdict=actionable - tests/lifecycle.rs repeats the same 8-field `DeviceObservation` literal ~8 times (e.g. 132-140,154-163,220-228,292-300,377-385( (each test then mutates a field or two (the fixture setup dominates the test bodies - fix: extract `fn device() -> DeviceObservation` helper (as `fn controller()` at tests/lifecycle.rs:104 already factors the bigger fixture (or build from a small builder - [packages/d2b-provider-guest-qemu-media/tests/lifecycle.rs:132, packages/d2b-provider-guest-qemu-media/tests/lifecycle.rs:154, packages/d2b-provider-guest-qemu-media/tests/lifecycle.rs:220, packages/d2b-provider-guest-qemu-media/tests/lifecycle.rs:292, packages/d2b-provider-guest-qemu-media/tests/lifecycle.rs:377]
  evidence: seed1 (`#\[test\]`= 33 hits over src+tests (the 8-field literal recurs at ~8 test sites in tests/lifecycle.rs (same file already uses `fn controller()` to factor the bigger fixture (so the pattern exists.
- clean: seeds ran:  33/96/0/0 over src+tests;33 #[test] (all behavioral (effect-order events, real-payload round-trips, stable error codes( (the 96 assertions use human-written expected values (no assertion restates the implementation ( (no property/snapshot tooling (closed bound tables suffice (0 #[ignore] (the gated fixture scan (tests/fixture_projection.rs:19-22(prints SKIP when D2B_FIXTURES unset (documented gate (not an ignore.

## Coverage
- idiom: 1 finding(s)
- own:  1 finding(s)
- type: 2 finding(s
- api:  1 finding(s
- err: clean (seeds ran:  5/1/0/9)
- serde: clean (seeds ran:  18/38/0/4)
- obs:  1 finding(s
- docs:1 finding(s
- perf:1 finding(s
- conc: N/A (seeds:  0/0/0/0 all zero; no concurrency usage)
- async: N/A (seeds:  0/0/0/0 all zero; no async code)
- unsafe: N/A (seeds 1-3:  0/0/0 all zero; seed4 = forbid manifest/lint settings only)
- ffi: N/A (seeds:  0/0/0/0 all zero; no FFI)
- macro: N/A (seeds:  0/0/0/0 all zero; no macros)
- test:1 finding(s