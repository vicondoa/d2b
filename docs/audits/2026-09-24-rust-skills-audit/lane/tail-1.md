# tail-1 - tail lane
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 1113 (src + tests, excl. src/generated/** and integration/) | modules: d2b-broker-fixture-handlers, d2b-broker-fixture-syscall-surface, d2b-controller-toolkit, d2b-host-activation-helper, d2b-provider-audio-binding
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crates

## d2b-broker-fixture-handlers

### idiom
- clean: seeds 0/0/0 (no index loops, no hand-written impls, no statement-style accumulation); the crate is one pure echo fn and reads idiomatic.

### own
- clean: seeds 1/0/0/0; the single `.clone()` at lib.rs:21 copies the borrowed `payload: &CanonicalJsonObject` into the owned `DispatchOutcome.result` - the echo must own its result, so the clone is required (census: payload type at packages/d2b-broker/src/envelope/mod.rs:974; result type at :904).

### type
- N/A: seeds 0/0/0 all zero; the crate declares no struct or enum (card criterion).

### api
- clean: seeds 1/0/0; one pub fn (`echo`) documented and deliberately exported for the composition seam (census: registered as handler at packages/d2b-broker-composition/src/seam.rs:339, 538).

### err
- clean: seeds 0/0/0/0; no panic sites, no swallowed Results; the fn returns the seam's `Result<DispatchOutcome, DispatchFailure>`.

### serde
- N/A: seeds 0/0/0/0 all zero; the crate crosses no wire.

### obs
- N/A: seeds 0/0/0/0 all zero; no tracing/log dependency (card criterion).

### docs
- clean: seeds 1/0/0; the single pub fn carries a one-line contract doc and the module has `//!` docs.

### perf
- clean: seeds 0/1/0; the one `Vec::new()` (lib.rs:22) builds the empty fds list - the empty case is common, false-positive class.

### conc
- N/A: seeds 0/0/0/0 all zero.

### async
- clean: seeds 1/0/0/0; one `Box::pin(async move ...)` with no `.await`, no spawn, no shared state, no blocking - a single immediate future.

### unsafe
- N/A: seeds 0/0/0/2; seeds 1-3 all zero - the two `unsafe_code` hits are the `#![deny(unsafe_code)]` attribute (lib.rs:13) and the manifest `deny` (Cargo.toml:9); no unsafe code exists.

### ffi
- N/A: seeds 0/0/0/0 all zero.

### macro
- N/A: seeds 0/0/0/0 all zero.

### test
- N/A: seeds 0/0/0/0 all zero; the crate ships no tests (the seam exercises it from d2b-broker-composition).

### Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: clean (seeds ran: 1/0/0/0)
- type: N/A (seeds: 0/0/0 all zero; no struct/enum declared)
- api: clean (seeds ran: 1/0/0)
- err: clean (seeds ran: 0/0/0/0)
- serde: N/A (seeds: 0/0/0/0 all zero; no wire)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency)
- docs: clean (seeds ran: 1/0/0)
- perf: clean (seeds ran: 0/1/0)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: clean (seeds ran: 1/0/0/0)
- unsafe: N/A (seeds: 0/0/0/2; seeds 1-3 all zero, only the deny attribute/manifest text)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: N/A (seeds: 0/0/0/0 all zero; no tests)

## d2b-broker-fixture-syscall-surface

### idiom
- clean: seeds 0/0/0; two small fns and a static, no expression-shape issues.

### own
- clean: seeds 0/0/0/0; no clones or shared-ownership types.

### type
- N/A: seeds 0/0/0 all zero; the crate declares no struct or enum (card criterion).

### api
- clean: seeds 3/0/0; three pub fns are the deliberate hostile surface the dependency-surface audit scans (census: audit probes at packages/d2b-broker-composition/src/dependency_surface.rs:436-471; refusal tests at seam.rs:612-638); the `FIXTURE_MARKER` static is private.

### err
- clean: seeds 0/0/0/0; no panic sites; `install_isolation_silencer`'s panic-hook set is the deliberate audit target, not a panic policy issue.

### serde
- N/A: seeds 0/0/0/0 all zero; the crate crosses no wire.

### obs
- N/A: seeds 0/0/0/0 all zero; no tracing/log dependency (card criterion).

### docs
- clean: seeds 3/0/0; all three pub fns carry contract docs including the fixture rationale.

### perf
- N/A: seeds 0/0/0 all zero.

### conc
- N/A: seeds 0/0/0/0 all zero.

### async
- N/A: seeds 0/0/0/0 all zero.

### unsafe
- tail-1#1 sev=medium blast=leaf effort=S verdict=actionable - the x86_64 `asm!` block omits the registers the `syscall` instruction clobbers (rcx and r11), so the compiler's no-clobber assumption is violated if the fn is ever executed - fix: add `lateout("rcx") _`, `lateout("r11") _` (or `clobber_abi("C")`) to the asm operands at lib.rs:26-32 - [packages/d2b-broker-fixture-syscall-surface/src/lib.rs:25-32]
  evidence: seed `\bunsafe \{` = 2 hits, `// SAFETY:` = 2; static read of the asm block (no reachability path: the crate is a dev-dependency of the composition root only, never linked into the broker binary - packages/d2b-broker-composition/Cargo.toml:16-18, and none of the fns are ever called)
- clean: both unsafe blocks carry `// SAFETY:` comments (lib.rs:23-24, 40); the `#[unsafe(link_section)]` attribute and panic-hook registration are the deliberate fixture surface the audit must reject (U1 card false-positive class), not re-flagged.

### ffi
- clean: seeds 1/0/0/0; the single hit is the `#[unsafe(link_section)]` marker - a link-time attribute, not a foreign-caller boundary (no extern "C", no repr, no CStr); deliberately hostile fixture surface.

### macro
- N/A: seeds 0/0/0/0 all zero.

### test
- N/A: seeds 0/0/0/0 all zero; no tests (the audit probes it from d2b-broker-composition).

### Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: clean (seeds ran: 0/0/0/0)
- type: N/A (seeds: 0/0/0 all zero; no struct/enum declared)
- api: clean (seeds ran: 3/0/0)
- err: clean (seeds ran: 0/0/0/0)
- serde: N/A (seeds: 0/0/0/0 all zero; no wire)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency)
- docs: clean (seeds ran: 3/0/0)
- perf: N/A (seeds: 0/0/0 all zero)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero)
- unsafe: 1 finding
- ffi: clean (seeds ran: 1/0/0/0)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: N/A (seeds: 0/0/0/0 all zero; no tests)

## d2b-controller-toolkit

### idiom
- clean: seeds 0/3/0; the three hand-written `Debug` impls (context.rs:92, 125, contract.rs:35) are the deliberate-redaction class (canonical_json shown as byte count, ResourceKey prints structural constants) - a derive would leak, recorded false-positive class.

### own
- clean: seeds 0/0/0/0; no clones, no refcounts, no Cow; accessors borrow.

### type
- tail-1#2 sev=low blast=family effort=S verdict=actionable - `ResourceSnapshot` carries `owner_uid: Option<ResourceUid>` and `owner_generation: Option<ResourceGeneration>` that are only ever set together, leaving the illegal one-Some/one-None combination constructible - fix: introduce `OwnerIdentity { uid, generation }` and replace the pair with a single `Option<OwnerIdentity>` (fields context.rs:17-18, constructor :61-67; the only external setter call passes both Some - packages/d2b-provider-provider/src/driver.rs:559) - [packages/d2b-controller-toolkit/src/context.rs:17-18, packages/d2b-controller-toolkit/src/context.rs:61-67]
  evidence: seeds 0/0/0 (lens applicable via the declared structs); census: `with_owner_identity` over packages/ = 1 call site (d2b-provider-provider/src/driver.rs:559, both Some) + definition; owner_generation() has no external consumers
- clean: no validate/check fns, no boolean-flag fields, no stringly-typed state; the single `deleting: bool` is a lone flag with no soup.

### api
- clean: seeds 18/0/2; the `pub use` re-export arms (lib.rs:11-12) are the house single-surface pattern; no Arc/Rc/Box/RefCell in signatures; all three exported types are consumed (census: d2b-core-controller/src/lib.rs:52 re-exports them; d2b-provider-provider/src/driver.rs:52-53, providers.rs:8 use them).

### err
- clean: seeds 0/0/0/0; no panic sites, no swallowed Results, no error enum (the crate's constructors cannot fail).

### serde
- N/A: seeds 0/0/0/0 all zero; the snapshots are in-memory types (no serde derives), no wire.

### obs
- N/A: seeds 0/0/0/0 all zero; no tracing/log dependency (card criterion).

### docs
- clean: seeds 18/0/0; every pub item carries a one-line contract doc; no Result returns so no `# Errors` sections are owed.

### perf
- N/A: seeds 0/0/0 all zero.

### conc
- N/A: seeds 0/0/0/0 all zero.

### async
- N/A: seeds 0/0/0/0 all zero.

### unsafe
- N/A: seeds 0/0/0/0 all zero; the manifest's `[lints] workspace = true` reference carries no `unsafe_code` text (seed 4 zero too).

### ffi
- N/A: seeds 0/0/0/0 all zero.

### macro
- N/A: seeds 0/0/0/0 all zero.

### test
- N/A: seeds 0/0/0/0 all zero; no tests and an empty dev-dependencies table (Cargo.toml:16).

### Coverage
- idiom: clean (seeds ran: 0/3/0)
- own: clean (seeds ran: 0/0/0/0)
- type: 1 finding
- api: clean (seeds ran: 18/0/2)
- err: clean (seeds ran: 0/0/0/0)
- serde: N/A (seeds: 0/0/0/0 all zero; no wire)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency)
- docs: clean (seeds ran: 18/0/0)
- perf: N/A (seeds: 0/0/0 all zero)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero)
- unsafe: N/A (seeds: 0/0/0/0 all zero; workspace lints reference)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: N/A (seeds: 0/0/0/0 all zero; no tests)

## d2b-host-activation-helper

### idiom
- clean: seeds 0/0/0; no index loops, no hand-written impls, no statement-style accumulation; the arg-parsing match is idiomatic.

### own
- clean: seeds 0/1/0/0; the single `.to_string()` (main.rs:69) builds a cold error string; no clones or refcounts anywhere.

### type
- clean: seeds 0/0/0; the `Config` struct's lone `fail_closed: bool` is a single flag with no soup; gid parsing is checked at the boundary (`parse_gid` returns Result).

### api
- N/A: seeds 0/0/0 all zero; bin-only crate with no lib target and no pub items (card criterion).

### err
- clean: seeds 0/0/0/0 in production code; `main` uses `unwrap_or_else` (main.rs:341) and exits with codes; the only `.expect()` hits are in `#[cfg(test)]` (false-positive class).

### serde
- N/A: seeds 0/0/0/0 all zero; the crate crosses no wire.

### obs
- clean: seeds 7/0/0/0; all seven `eprintln!` sites are CLI product output (usage, error reporting, migration audit lines) per the card's CLI false-positive class; no tracing/log dependency.

### docs
- N/A: seeds 0/0/11; seed 1 zero - no public items exist in the bin crate (card: never add missing_docs to a binary crate); the 11 `-> Result<` hits are internal fns.

### perf
- clean: seeds 5/0/1; all `format!` sites are cold paths (usage text, arg errors, one log line per migrated entry); no hot-loop allocation.

### conc
- N/A: seeds 0/0/0/0 all zero; single-threaded walk.

### async
- N/A: seeds 0/0/0/0 all zero.

### unsafe
- tail-1#3 sev=medium blast=leaf effort=S verdict=actionable - 22 production `unsafe` blocks (libc calls plus `errno_clear`'s `__errno_location` write) carry no `// SAFETY:` comment, violating the skill's mechanical rule and U1 (d) 8 - fix: add a `// SAFETY:` comment to each block stating the invariant (CString NUL-termination, checked return before use, fd ownership) - [packages/d2b-host-activation-helper/src/main.rs:96, packages/d2b-host-activation-helper/src/main.rs:129, packages/d2b-host-activation-helper/src/main.rs:140, packages/d2b-host-activation-helper/src/main.rs:194, packages/d2b-host-activation-helper/src/main.rs:213, packages/d2b-host-activation-helper/src/main.rs:271]
  evidence: seed `\bunsafe \{` = 24 hits (22 production + 2 cfg(test)), `// SAFETY:` = 0, `MaybeUninit` = 2; U1 (d) 8 mechanical rule (a block without a SAFETY comment is a finding)
- tail-1#4 sev=high blast=leaf effort=S verdict=actionable - `walk_dir` leaks the `fdopendir` DIR* handle on every error-path early return: `closedir` runs only on the readdir-null path (main.rs:206), so the `?` at :218, the `return Err` at :222 and :260, and `result?` at :265 all leak the directory stream (route review-pass) - fix: restructure so `closedir` runs on every exit (closure + close after, or an RAII guard) - [packages/d2b-host-activation-helper/src/main.rs:194, packages/d2b-host-activation-helper/src/main.rs:218, packages/d2b-host-activation-helper/src/main.rs:222, packages/d2b-host-activation-helper/src/main.rs:260, packages/d2b-host-activation-helper/src/main.rs:265]
  evidence: static read of walk_dir (main.rs:186-268); closedir appears once on the success/end-of-stream path; the four early returns after fdopendir succeed skip it (correctness defect, not UB)
- clean: the libc calls themselves (open/fcntl/fstat/fstatat/dup/fdopendir/readdir/closedir/fchownat/openat/fchown/close) are standard usage with checked returns, `CString` NUL handling, and correct `MaybeUninit` (assume_init only after rc == 0); the two cfg(test) unsafe sites carry policy-tracked sanctioned allows.

### ffi
- clean: seeds 0/0/0/5; the CStr/CString usage at the libc boundary is correct (NUL-termination via `CString::new` with InvalidInput errors, `CStr::from_ptr` on readdir's NUL-terminated d_name); these are libc-binding call sites that never cross a foreign caller (card false-positive class).

### macro
- N/A: seeds 0/0/0/0 all zero.

### test
- clean: seeds 2/5/0/0; two behavioral tests (migration walk + fail-closed rescan, held-lock fail-closed exit) with real tempdir filesystems, deterministic, no `#[ignore]`; each assertion can fail.

### Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: clean (seeds ran: 0/1/0/0)
- type: clean (seeds ran: 0/0/0)
- api: N/A (seeds: 0/0/0 all zero; bin-only crate, no pub items)
- err: clean (seeds ran: 0/0/0/0)
- serde: N/A (seeds: 0/0/0/0 all zero; no wire)
- obs: clean (seeds ran: 7/0/0/0)
- docs: N/A (seeds: 0/0/11; seed 1 zero - no public items; bin-only crate)
- perf: clean (seeds ran: 5/0/1)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero)
- unsafe: 2 findings
- ffi: clean (seeds ran: 0/0/0/5)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 2/5/0/0)

## d2b-provider-audio-binding

### idiom
- clean: seeds 0/1/0; the hand-written `impl Default for AudioBinding` (audio_binding.rs:98) is justified - `Arc<dyn AudioBindingChildSource>` has no field-wise default, and the impl wires the crate's own `BindingChildSource`.

### own
- clean: seeds 2/1/0/0; the two `.clone()` calls (audio_binding.rs:130) copy `ResourceRef` values into the owned dependencies Vec the trait contract returns; the `to_owned` hit is a test fixture string; `Arc<dyn AudioBindingChildSource>` is genuine shared ownership (behavior shared by driver and factory).

### type
- clean: seeds 0/0/0; no validate/check fns, no boolean flags, no stringly-typed state; the child-request struct is three plain borrows.

### api
- clean: seeds 11/2/1; the two `Arc<dyn ...>` in public signatures (`AudioBinding::new` at :93, `audio_binding_spec_decoder` at :158) are the interaction-family pattern with genuine shared ownership (call sites: tests/registration.rs:72, descriptor construction at :163-180); the `pub use` arm (lib.rs:16-19) is the house single-surface pattern; the trait has one required method.

### err
- clean: seeds 0/0/0/0 in src; no unwrap/expect/panic in production code; the `map_err(|_| InteractionEffectError::InvalidResource)` translation is the family port's coarse two-variant vocabulary (interaction.rs:125-130), the same translation the whole family applies - restructuring it is a family-contract change, not a binding-crate defect.

### serde
- N/A: seeds 0/0/0/0 all zero; src crosses no wire (spec decoding happens through the family's `spec_decoder`); serde_json appears only in tests.

### obs
- N/A: seeds 0/0/0/0 all zero; no tracing/log dependency (card criterion).

### docs
- tail-1#5 sev=low blast=leaf effort=S verdict=actionable - the four Result-returning trait methods (`binding_children`, `validate`, `dependencies`, `desired_children`) lack `# Errors` sections saying which conditions yield `Unavailable` vs `InvalidResource`, even though the crate denies missing_docs - fix: add `# Errors` sections naming the family's two failure variants - [packages/d2b-provider-audio-binding/src/audio_binding.rs:56, packages/d2b-provider-audio-binding/src/audio_binding.rs:117, packages/d2b-provider-audio-binding/src/audio_binding.rs:125, packages/d2b-provider-audio-binding/src/audio_binding.rs:134]
  evidence: docs seeds 11/0/4; all four `-> Result<` hits are pub trait methods without `# Errors` sections (the variants are documented only on the family enum, packages/d2b-provider-wayland-policy/src/interaction.rs:124-130)
- clean: every pub item carries a one-line contract doc (missing_docs denied in lib.rs:13); no `# Examples` sections owed where signatures are self-evident.

### perf
- N/A: seeds 0/0/0 all zero.

### conc
- N/A: seeds 0/0/0/0 all zero.

### async
- N/A: seeds 0/0/0/0 all zero in src; the only async fns are the test harness's `async_trait` effect-port stubs (tests/registration.rs:33, 41).

### unsafe
- N/A: seeds 0/0/0/1; seeds 1-3 all zero - the single `unsafe_code` hit is the manifest `forbid` (Cargo.toml:9), which does not make the lens applicable (card rule).

### ffi
- N/A: seeds 0/0/0/0 all zero.

### macro
- N/A: seeds 0/0/0/0 all zero.

### test
- clean: seeds 5/14/0/0; five behavioral tests (declaration row, registry duplicate refusal, service/target reads, child-row materialization, foreign-row refusal) asserting observable contracts with real assertions; deterministic, no `#[ignore]`, no network.

### Coverage
- idiom: clean (seeds ran: 0/1/0)
- own: clean (seeds ran: 2/1/0/0)
- type: clean (seeds ran: 0/0/0)
- api: clean (seeds ran: 11/2/1)
- err: clean (seeds ran: 0/0/0/0)
- serde: N/A (seeds: 0/0/0/0 all zero; no wire in src)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency)
- docs: 1 finding
- perf: N/A (seeds: 0/0/0 all zero)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero in src)
- unsafe: N/A (seeds: 0/0/0/1; seed 4 = manifest forbid only)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds ran: 5/14/0/0)