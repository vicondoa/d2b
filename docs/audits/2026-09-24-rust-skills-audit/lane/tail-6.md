# tail-6 - tail lane
Baseline: 6ebdd4cec | LOC audited: 2173 (excl. src/generated/**) | modules: d2b-resource-types, d2b-sk-frontend
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crates

## d2b-resource-types

### idiom
- clean: seeds `for \w+ in 0\.\.` = 0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 0, `let mut \w+ = (String|Vec)::new\(\)` = 0. No index loops, no statement-style accumulation; the only hand-written impls are `Debug` (redaction, deliberate) and trait impls, which the seed shape does not match. The `while` loop in the `ALL_TYPES` const block (resource_type.rs:36-44) is const-context-required and documented.

### own
- clean: seeds `\.clone\(\)` = 1, `\.to_owned\(\)` = 1, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 0, `Cow<` = 0. The clone is a test assertion (metadata.rs:101); the `to_owned` (descriptor.rs:107) is required by the `DriverRegistration::declared_verbs() -> Vec<String>` trait signature. `Arc` handles in `DriverDescriptor`/`KernelCaller` are shared registry/seam ownership, not clones.

### type
- clean: seeds `fn validate_\w+|fn check_\w+` = 0, `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0. State is already modelled as enums/bitflags (`AllowedSources`, `ChildCustody`, `Cardinality`, `IsolationPosture`); no boolean flag soup or stringly-typed state.

### api
- tail-6#3 sev=medium blast=family effort=M verdict=actionable - `assert_metadata_registration` is a test-only assertion helper exported unconditionally through the crate root, while the crate already declares a `test-support` feature that no consumer enables - fix: gate the fn and its `pub use` arm behind `#[cfg(feature = "test-support")]` (or `any(test, feature = "test-support")` per the house pattern in d2b-provider-activation-nixos/Cargo.toml:16-23) and enable the feature from the 11 consumer crates' test targets - [packages/d2b-resource-types/src/lib.rs:30, packages/d2b-resource-types/src/metadata.rs:72, packages/d2b-resource-types/Cargo.toml:9]
  evidence: census: `assert_metadata_registration` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel/*.bzl = 13 hits; all 11 external callers are `tests/registration.rs` in d2b-provider-{command,emergency-policy,operation,quota,resource-export,resource-import,role,role-binding,seccomp-profile,zone,zone-link}; `d2b-resource-types = {` in 38 manifests, 0 with `features = ["test-support"]`
- clean: seed `\bpub (fn|struct|enum|trait|type|const|mod) ` = 97 hits, seed `pub .*\b(Arc|Rc|Box|RefCell)<` = 2 hits (descriptor.rs:76,78; operation.rs:105,108), seed `^\s*pub use ` = 11 arms. The `Arc<dyn ...>` fields are genuine shared registry ownership (cloned by the `DriverRegistration` impl, descriptor.rs:110-113); `pub use` arms are the house single-surface pattern; remaining surface is the deliberate declaration vocabulary.

### err
- clean: seeds `\.unwrap\(\)|\.expect\(` = 5, `let _ = |\.ok\(\);` = 0, `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 0, `enum \w*Error` = 0. All 5 `expect` sites sit inside `assert_metadata_registration` (metadata.rs:100-121), a test-support assertion helper; no panic sites in library paths. `OperationFailure` carries a closed `&'static str` code mirroring the repo's wire error-code convention.

### serde
- N/A (seeds: 0/0/0/0 all zero; no serde derives, no hand-written deserializers, no JSON calls - the crate crosses no wire boundary)

### obs
- clean: seeds `\bprintln!\(|\beprintln!\(` = 0, `(info|debug|warn|error|trace)!\("` = 0, `\.instrument\(|#\[instrument` = 0, `tracing::|log::` = 0. Declaration crate emits no telemetry.

### docs
- tail-6#5 sev=low blast=leaf effort=S verdict=actionable - doc comment typos in `OperationCtx::fds` ("invocation,when any", "frame,not to the handler; the handler") - fix: restore the missing spaces after the commas in the field docs - [packages/d2b-resource-types/src/operation.rs:64, packages/d2b-resource-types/src/operation.rs:65]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 97 hits, read all; seed `/// # (Examples|Errors|Panics|Safety)` = 0; seed `-> Result<` = 1. `#![deny(missing_docs)]` (lib.rs:6) is active; every public item is documented; the two typo lines are the only defects found.
- clean: seeds run as above; all 97 public items documented with first-sentence-shaped docs under `#![deny(missing_docs)]` (lib.rs:6); no canonical-section or magic-value gaps found beyond the typo finding.

### perf
- clean: seeds `format!\(` = 0, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 1, `\.to_string\(\)` = 0. The single `Vec::new()` (operation.rs:179) is a cold result-construction path; no hot-path allocation. static (unmeasured).

### conc
- N/A (seeds: 0/0/0/0 all zero; no threads, locks, atomics, or orderings in the crate)

### async
- clean: seeds `async fn|async move|\.await` = 3, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 0, `tokio::sync::(Mutex|RwLock|Notify)` = 0, `#\[tokio::(main|test)\]|Runtime::block_on` = 0. The three hits are the `#[async_trait] OperationHandler::execute` (operation.rs:49) and the async test-support helper (metadata.rs:72,113); no runtime, spawn, or blocking work.

### unsafe
- N/A (seeds: 0/0/0/0 all zero; no unsafe blocks/fns/impls, no SAFETY comments, no transmute/raw-pointer use, no `unsafe_code` allow manifest)

### ffi
- N/A (seeds: 0/0/0/0 all zero; no extern declarations, no repr(C), no CStr/CString)

### macro
- N/A (seeds: 0/0/0/0 all zero; no macro_rules!, no proc-macro machinery)

### test
- clean: seeds `#\[test\]|#\[tokio::test\]` = 7, `assert_eq!\(|assert_ne!\(|assert!\(` = 39 (incl. 17 asserts in the shared test-support helper), `proptest!|insta::assert|rstest` = 0, `#\[ignore\]` = 0. Seven unit tests in three modules (allowed_sources.rs, child_creation.rs, resource_type.rs) assert behavior with messages; the shared `assert_metadata_registration` helper centralizes the per-type registration coverage for 11 consumer crates.

### Coverage
- idiom: clean (seeds ran: 0/0/0)
- own: clean (1/1/0/0)
- type: clean (0/0/0)
- api: 1 finding
- err: clean (5/0/0/0)
- serde: N/A (seeds: 0/0/0/0 all zero; no wire boundary)
- obs: clean (0/0/0/0)
- docs: 1 finding
- perf: clean (0/1/0)
- conc: N/A (seeds: 0/0/0/0 all zero; no threads/locks/atomics)
- async: clean (3/0/0/0)
- unsafe: N/A (seeds: 0/0/0/0 all zero; no unsafe constructs)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (7/39/0/0)

## d2b-sk-frontend

### idiom
- tail-6#1 sev=low blast=leaf effort=S verdict=actionable - `zone_path` accumulates labels with a `let mut Vec` + push loop where an iterator pipeline collects - fix: replace the loop with `value.split('/').map(|label| ZoneLabelId::parse(label).map_err(|_| format!("{name} is not a valid Zone label path"))).collect::<Result<Vec<_>, String>>()?` before `ZonePath::new(labels)` - [packages/d2b-sk-frontend/src/config.rs:178]
  evidence: seed `let mut \w+ = (String|Vec)::new\(\)` = 1 hit (config.rs:178); seeds `for \w+ in 0\.\.` = 0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 0
- clean: seeds run as above; the one hit is the finding; hand-written `Debug` impls (agent.rs:145-148, uhid.rs:77-105) are deliberate redaction.

### own
- tail-6#2 sev=low blast=leaf effort=S verdict=actionable - `main` clones the whole `PlacementConfig` (owned `ZoneEnrollmentIdentity` inside) only to keep `config` alive for its other fields, and `config.rs` builds `"/dev/uhid".to_owned()` where `PathBuf::from` suffices - fix: destructure `let Config { vm_id, link, uhid_path, placement } = config;` and call `placement.into_placement()` (drop the clone); write `PathBuf::from("/dev/uhid")` via `optional("D2B_SK_UHID_PATH").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/dev/uhid"))` - [packages/d2b-sk-frontend/src/main.rs:55, packages/d2b-sk-frontend/src/config.rs:83]
  evidence: seed `\.clone\(\)|\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = 6 hits; 2 avoidable (main.rs:55, config.rs:83); the rest are required conversions (agent.rs:178 `to_vec` for `GuestFrame::new`, config.rs:88/91/108 owned error strings)
- clean: seeds run as above; no `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` (0) and no `Cow<` (0) in the crate.

### type
- clean: seeds `fn validate_\w+|fn check_\w+` = 0, `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0. State is already modelled as enums (`UhidEvent`, `DeviceRequest`) and typed fields; the `OnceCell<Mutex<D>>` device lifecycle is a deliberate once-init shape.

### api
- tail-6#4 sev=low blast=leaf effort=S verdict=actionable - `pub mod agent/config/link/uhid` plus root `pub use` re-exports make every item reachable at two paths, deviating from the house single-surface pattern; only the binary needs a module path - fix: make the four modules private (`mod agent; ...`) and re-export `UhidDevice` (and `UhidEvent`) from lib.rs, updating main.rs:41 to `use d2b_sk_frontend::{Config, SecurityKeyFrontend, UhidDevice, VsockAllocatorLink}` - [packages/d2b-sk-frontend/src/lib.rs:22, packages/d2b-sk-frontend/src/lib.rs:27, packages/d2b-sk-frontend/src/main.rs:41]
  evidence: seed `^\s*pub use ` = 3 arms (lib.rs:27-29) alongside `pub mod` x4 (lib.rs:22-25); census: `d2b_sk_frontend::(agent|config|link|uhid)::` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel/*.bzl = 0 full-path hits, and the zone-routing test consumer uses the root re-exports (tests/guest_enrollment.rs:210,422), so only main.rs:41's group-import form needs the module path
- clean: seed `\bpub (fn|struct|enum|trait|type|const|mod) ` = 25 hits, seed `pub .*\b(Arc|Rc|Box|RefCell)<` = 0. No internals leak into signatures beyond the double-path shape above.

### err
- clean: seeds `\.unwrap\(\)|\.expect\(` = 1, `let _ = |\.ok\(\);` = 1, `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(` = 0, `enum \w*Error` = 0. The expect is in a `#[cfg(test)]` test (config.rs:213); the `let _ = event_type;` (uhid.rs:203) silences an unused binding, not a `Result`. `Config::from_env` returns `String` errors consumed once by the binary's `exit_on_error` print - acceptable binary-boundary shape.

### serde
- N/A (seeds: 0/0/0/0 all zero; no serde derives or JSON crossing - the crate's wire surface is the toolkit's session framing, not serde)

### obs
- clean: seeds `\bprintln!\(|\beprintln!\(` = 2, `(info|debug|warn|error|trace)!\("` = 0, `\.instrument\(|#\[instrument` = 0, `tracing::|log::` = 0. Both `eprintln!` sites are the binary's own startup banner and fatal-error output (main.rs:47,57) - product output, not telemetry; the library half emits nothing.

### docs
- clean: seeds `^\s*pub (fn|struct|enum|trait|const|type)` = 29 hits, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 5. `#![deny(missing_docs)]` (lib.rs:6) is active; all 29 public items and the `Result`-returning fns (from_env, into_placement, create, read_event, send_input_report) carry first-sentence-shaped prose docs; the UHID constants document their kernel-header provenance.

### perf
- clean: seeds `format!\(` = 16, `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 2, `\.to_string\(\)` = 0. All `format!` sites are error paths (config.rs), one-shot device creation (uhid.rs:275), or tests (uhid.rs:481); the event builders pre-size with `with_capacity` and read into a stack buffer. static (unmeasured).

### conc
- clean: seeds `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 2, `Atomic\w+|Ordering::` = 0, `thread_local!|unsafe impl (Send|Sync) for` = 0. The two `Mutex<` hits are `tokio::sync::Mutex` (agent.rs:105,133) - async-aware primitives judged under the async lens; no threads, atomics, or manual Send/Sync claims.

### async
- clean: seeds `async fn|async move|\.await` = 39, `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 0, `tokio::sync::(Mutex|RwLock|Notify)` = 1, `#\[tokio::(main|test)\]|Runtime::block_on` = 0. The device I/O uses the sanctioned `AsyncFd` + `try_io` readiness loops over `rustix::io` (uhid.rs:233-259, the clippy.toml replacement vocabulary), `tokio::fs` open with `into_std().await` (uhid.rs:156-163), and `tokio::sync::Mutex`/`OnceCell` guards held across awaits (agent.rs:133-141,166-177) - all async-aware; no std guards across `.await`, no executor blocking, no runtime started in the library.

### unsafe
- N/A (seeds: 0/0/0/1; the only hits are `#![forbid(unsafe_code)]` (lib.rs:20) and two `io::Error::from_raw_os_error` std-safe calls (uhid.rs:238,251) matching seed 3's `from_raw` substring - no unsafe blocks/fns/impls exist, and the manifest is `deny`, not `allow`)

### ffi
- N/A (seeds: 0/0/0/0 all zero; no extern declarations, no repr(C), no CStr/CString - `libc::O_NONBLOCK` and `rustix::io` are syscall bindings, not an FFI surface)

### macro
- N/A (seeds: 0/0/0/0 all zero; no macro_rules!, no proc-macro machinery)

### test
- tail-6#6 sev=medium blast=leaf effort=S verdict=actionable - the parse side of the byte-exact UHID protocol (`read_event`'s event-type dispatch, the OUTPUT size field at payload[4096], GET_REPORT id, lifecycle mapping, short-header error) has no test while the builders have 12 byte-exact tests, so a regression in the parse offsets passes the suite - fix: extract `parse_event(buf: &[u8]) -> io::Result<Option<UhidEvent>>` from `read_event` and table-test the dispatch against hand-built buffers (plus a `build_get_report_reply_error` layout test) - [packages/d2b-sk-frontend/src/uhid.rs:175, packages/d2b-sk-frontend/src/uhid.rs:186]
  evidence: seed `#\[test\]|#\[tokio::test\]` = 14 hits (12 in uhid.rs, 2 in config.rs); seed `assert_eq!\(|assert_ne!\(|assert!\(` = 24; none of the uhid.rs tests exercise `read_event` or `build_get_report_reply_error` (uhid.rs:283-299)
- clean: seeds run as above; the 14 tests are behavior-focused with messages, deterministic, and byte-exact for the builders; no `#[ignore]` (0), no property/snapshot tooling (0).

### Coverage
- idiom: 1 finding
- own: 1 finding
- type: clean (0/0/0)
- api: 1 finding
- err: clean (1/1/0/0)
- serde: N/A (seeds: 0/0/0/0 all zero; no serde wire crossing)
- obs: clean (2/0/0/0)
- docs: clean (29/0/5)
- perf: clean (16/2/0)
- conc: clean (0/2/0/0)
- async: clean (39/0/1/0)
- unsafe: N/A (seeds: 0/0/0/1; no real unsafe constructs, forbid is seed 4 only)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: 1 finding