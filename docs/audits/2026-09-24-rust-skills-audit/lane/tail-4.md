# tail-4 - tail lane
Baseline: 6ebdd4cec | LOC audited: 2515 (excl. src/generated/**) | modules: d2b-provider-role-binding, d2b-provider-seccomp-profile, d2b-provider-shell-pool, d2b-provider-shell-session, d2b-provider-telemetry-binding
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test, supply | Partitions: whole crates

## d2b-provider-role-binding

### idiom
- clean: seeds `for \w+ in 0\.\.` = 0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 0, `let mut \w+ = (String|Vec)::new\(\)` = 0; the only fn body is the one-line descriptor declaration, nothing to judge.

### own
- clean: seeds `.clone()` = 0, `.to_owned()|.to_vec()|.to_string()` = 0, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 0, `Cow<` = 0; no copies or shared ownership anywhere.

### type
- N/A: seeds `fn validate_\w+|fn check_\w+` = 0, `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0 all zero; the crate declares no struct or enum (one free fn only).

### api
- clean: seeds `\bpub (fn|struct|enum|trait|type|const|mod) ` = 1, `pub .*\b(Arc|Rc|Box|RefCell)<` = 0, `^\s*pub use ` = 1; single `pub fn role_binding_descriptor` re-exported through the house single-surface pattern (lib.rs:19).

### err
- clean: seeds `.unwrap()|.expect()` = 0, `let _ = |.ok();` = 0, `panic!|unreachable!|todo!|unimplemented!` = 0, `enum \w*Error` = 0; no panic sites and no error surface of its own (the shared metadata driver owns failures).

### serde
- N/A: seeds `derive)...Serialize` = 0, `serde)...)` = 0, `impl .*Deserialize.*for` = 0, `serde_json::from_|serde_json::to_` = 0 all zero; the crate crosses no wire (declaration-only driver).

### obs
- N/A: seeds `println!|eprintln!` = 0, `(info|debug|warn|error|trace)!\("` = 0, `.instrument(|#[instrument` = 0, `tracing::|log::` = 0 all zero and the manifest (Cargo.toml) lists no tracing/log dependency.

### docs
- clean: seeds `^\s*pub (fn|struct|enum|trait|const|type)` = 1, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 0; the one pub item carries a doc comment and `#![deny(missing_docs)]` is on (lib.rs:14).

### perf
- clean: seeds `format!(` = 0, `Vec::new()|VecDeque::new()|HashMap::new()|BTreeMap::new()` = 0, `.to_string()` = 0; no allocation sites at all.

### conc
- N/A: seeds `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 0, `Atomic\w+|Ordering::` = 0, `thread_local!|unsafe impl (Send|Sync) for` = 0 all zero; no threads, locks, or atomics.

### async
- N/A: seeds `async fn|async move|.await` = 0, `tokio::spawn|spawn_blocking|JoinSet|select!|join!` = 0, `tokio::sync::(Mutex|RwLock|Notify)` = 0, `#[tokio::(main|test)]|Runtime::block_on` = 0 all zero in src; the only async code is the `#[tokio::test]` registration shim in tests/, which the test lens owns.

### unsafe
- N/A: seeds `\bunsafe {|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0 all zero; manifest sets `unsafe_code = "forbid"` and no `unsafe_code = "allow"` exists.

### ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0 all zero; no foreign boundary.

### macro
- N/A: seeds `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0 all zero; no macro definitions.

### test
- clean: seeds `#[test]|#[tokio::test]` = 1, `assert_eq!|assert_ne!|assert!` = 6, `proptest!|insta::assert|rstest` = 0, `#[ignore]` = 0; the single registration test is the documented shared `assert_metadata_registration` shim (tests/registration.rs:11-17, recorded pattern, not flagged).

### Coverage
- idiom: clean (seeds: 0/0/0)
- own: clean (seeds: 0/0/0/0)
- type: N/A (seeds: 0/0/0 all zero; no struct or enum declared)
- api: clean (seeds: 1/0/1)
- err: clean (seeds: 0/0/0/0)
- serde: N/A (seeds: 0/0/0/0 all zero; no wire crossing)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency)
- docs: clean (seeds: 1/0/0)
- perf: clean (seeds: 0/0/0)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero in src)
- unsafe: N/A (seeds 1-3: 0/0/0; manifest forbid)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds: 1/6/0/0)
- supply: clean (X1 owns the lens; per-crate manifest check: d2b-resource-types used in driver.rs, tokio dev-dep used in tests/registration.rs)

## d2b-provider-seccomp-profile

### idiom
- tail-4#1 sev=low blast=leaf effort=S verdict=actionable - three impl-block closing braces are indented at 4 spaces instead of column 0 (fmt drift; `cargo fmt --check` would fail) - fix: dedent the closing braces of `impl DeviceNodePath`, `impl DeviceBind`, and `impl SeccompProfileSpec` to column 0 (rustfmt) - [packages/d2b-provider-seccomp-profile/src/seccomp_profile.rs:43, packages/d2b-provider-seccomp-profile/src/seccomp_profile.rs:162, packages/d2b-provider-seccomp-profile/src/seccomp_profile.rs:196]
  evidence: seed `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 0, `for \w+ in 0\.\.` = 0, `let mut \w+ = (String|Vec)::new\(\)` = 0; finding from read: lines 43/162/196 are `    }` closing impl blocks opened at column 0.
- clean: seeds 0/0/0; the only expression-shape items are the validated newtype and constructor, which already follow the parse-once pattern.

### own
- clean: seeds `.clone()` = 0, `.to_owned()|.to_vec()|.to_string()` = 2, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 0, `Cow<` = 0; the two `to_owned()` calls (seccomp_profile.rs:56, 66) build the JsonSchema schema-name strings, which must be owned.

### type
- clean: seeds `fn validate_\w+|fn check_\w+` = 0, `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0; `DeviceNodePath` is a private-field parse-only newtype and `SeccompProfileSpec::new` validates at construction (the patterns this lens wants); the `SeccompNamespaces` booleans are schema-mirroring wire fields with `deny_unknown_fields` (recorded false positive).

### api
- tail-4#2 sev=low blast=family effort=S verdict=actionable - every `seccomp_profile` item is reachable at two paths: `pub mod seccomp_profile` (lib.rs:19) plus the glob `pub use seccomp_profile::*` (lib.rs:22), and d2bd imports via both paths - fix: make the module private (`mod seccomp_profile;`) and replace the glob with the house-style explicit re-export list (as shell-pool/shell-session/telemetry-binding lib.rs do), then update the two d2bd imports at foundation_seed.rs:25 and :1091 to the crate-root paths - [packages/d2b-provider-seccomp-profile/src/lib.rs:19, packages/d2b-provider-seccomp-profile/src/lib.rs:22, packages/d2bd/src/foundation_seed.rs:25, packages/d2bd/src/foundation_seed.rs:1091]
  evidence: census: `seccomp_profile::` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel = 3 hits (lib.rs:22 plus d2bd/src/foundation_seed.rs:25, 1091); `d2b_provider_seccomp_profile` = 14 hits; d2bd also uses the crate-root path at foundation_seed.rs:1268, so both paths are live.
- clean: seeds `\bpub (fn|struct|enum|trait|type|const|mod) ` = 17, `pub .*\b(Arc|Rc|Box|RefCell)<` = 0, `^\s*pub use ` = 2; no internals in signatures; the two-path glob is the only surface issue.

### err
- clean: seeds `.unwrap()|.expect()` = 10, `let _ = |.ok();` = 0, `panic!|unreachable!|todo!|unimplemented!` = 0, `enum \w*Error` = 1; all 10 unwrap/expect sit in `#[cfg(test)] mod tests` (seccomp_profile.rs:253-308); `SeccompProfileContractError` is a small documented enum with Display, no wire code surface.

### serde
- clean: seeds `derive)...Serialize` = 8, `serde)...)` = 16, `impl .*Deserialize.*for` = 2, `serde_json::from_|serde_json::to_` = 2; the two hand-written `Deserialize` impls (DeviceNodePath, SeccompProfileSpec) are live admission gates that call the validating constructors - the parse-once pattern this lens recommends (recorded refusal class for hand-written admission gates); `deny_unknown_fields` + `#[serde(default)]` discipline is consistent, and a round-trip plus unknown-field-refusal test exists (seccomp_profile.rs:304-315).

### obs
- N/A: seeds `println!|eprintln!` = 0, `(info|debug|warn|error|trace)!\("` = 0, `.instrument(|#[instrument` = 0, `tracing::|log::` = 0 all zero and the manifest lists no tracing/log dependency.

### docs
- tail-4#3 sev=medium blast=leaf effort=S verdict=actionable - the crate's two public Result-returning constructors carry no `# Errors` section, and their failure conditions are non-obvious (byte-length bound, control characters, `/dev` prefix; syscall/device list bounds) - fix: add `# Errors` sections to `DeviceNodePath::parse` and `SeccompProfileSpec::new` naming the three `SeccompProfileContractError` variants each can return - [packages/d2b-provider-seccomp-profile/src/seccomp_profile.rs:31, packages/d2b-provider-seccomp-profile/src/seccomp_profile.rs:176]
  evidence: seeds `^\s*pub (fn|struct|enum|trait|const|type)` = 17, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 2; the two Result returns are the public parse/new constructors; every other pub item is documented under `#![deny(missing_docs)]`.
- clean: module docs (`//!`) present in all three files; every public item has a doc comment and the crate denies missing_docs (lib.rs:14).

### perf
- clean: seeds `format!(` = 0, `Vec::new()|VecDeque::new()|HashMap::new()|BTreeMap::new()` = 1, `.to_string()` = 0; the single `Vec::new()` (seccomp_profile.rs:297) is a test argument where the empty case is the point.

### conc
- N/A: seeds `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 0, `Atomic\w+|Ordering::` = 0, `thread_local!|unsafe impl (Send|Sync) for` = 0 all zero.

### async
- N/A: seeds `async fn|async move|.await` = 0, `tokio::spawn|spawn_blocking|JoinSet|select!|join!` = 0, `tokio::sync::(Mutex|RwLock|Notify)` = 0, `#[tokio::(main|test)]|Runtime::block_on` = 0 all zero in src; the only async code is the `#[tokio::test]` registration shim in tests/.

### unsafe
- N/A: seeds 1-3 (`\bunsafe {|\bunsafe fn|\bunsafe impl|\bunsafe extern`, `// SAFETY:`, `transmute|from_raw|MaybeUninit|mem::zeroed`) = 0/0/0; manifest sets `unsafe_code = "forbid"`.

### ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0 all zero.

### macro
- N/A: seeds `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0 all zero; `redacted_debug!` appears only as an invocation of the deliberately exported macro (recorded false positive).

### test
- clean: seeds `#[test]|#[tokio::test]` = 4, `assert_eq!|assert_ne!|assert!` = 7, `proptest!|insta::assert|rstest` = 0, `#[ignore]` = 0; three unit tests (table-driven path rejection with `{path:?}` messages, fail-closed list bounds, wire round-trip plus unknown-field refusal) plus the documented registration shim; no test that cannot fail.

### Coverage
- idiom: 1 finding(s)
- own: clean (seeds: 0/2/0/0)
- type: clean (seeds: 0/0/0)
- api: 1 finding(s)
- err: clean (seeds: 10/0/0/1, all unwrap in cfg(test))
- serde: clean (seeds: 8/16/2/2)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency)
- docs: 1 finding(s)
- perf: clean (seeds: 0/1/0)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero in src)
- unsafe: N/A (seeds 1-3: 0/0/0; manifest forbid)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds: 4/7/0/0)
- supply: clean (X1 owns the lens; per-crate manifest check: d2b-contracts-resource, d2b-resource-types, schemars, serde all used in src; tokio + serde_json dev-deps used in tests)

## d2b-provider-shell-pool

### idiom
- clean: seeds `for \w+ in 0\.\.` = 0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 0, `let mut \w+ = (String|Vec)::new\(\)` = 0; the crate is one trait impl plus declarations, all in expression shape.

### own
- clean: seeds `.clone()` = 0, `.to_owned()|.to_vec()|.to_string()` = 0, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 0, `Cow<` = 0; no copies or shared ownership.

### type
- clean: seeds `fn validate_\w+|fn check_\w+` = 0, `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0; the only type is the unit struct `ShellPool`; no flags or stringly state.

### api
- clean: seeds `\bpub (fn|struct|enum|trait|type|const|mod) ` = 8, `pub .*\b(Arc|Rc|Box|RefCell)<` = 1, `^\s*pub use ` = 1; the single `Arc<dyn SpecDecoder>` return (shell_pool.rs:93) is required by the `DriverDescriptor.decoder` field type, and the re-export list is the explicit house pattern.

### err
- clean: seeds `.unwrap()|.expect()` = 0, `let _ = |.ok();` = 0, `panic!|unreachable!|todo!|unimplemented!` = 0, `enum \w*Error` = 0; failures are the family's `InteractionEffectError`, propagated with `?`; no panic sites.

### serde
- N/A: seeds `derive)...Serialize` = 0, `serde)...)` = 0, `impl .*Deserialize.*for` = 0, `serde_json::from_|serde_json::to_` = 0 all zero; the spec is authored as the Provider's reference document and decoded by the family decoder (`spec_decoder()`), so this crate crosses no wire of its own.

### obs
- N/A: seeds `println!|eprintln!` = 0, `(info|debug|warn|error|trace)!\("` = 0, `.instrument(|#[instrument` = 0, `tracing::|log::` = 0 all zero and the manifest lists no tracing/log dependency.

### docs
- clean: seeds `^\s*pub (fn|struct|enum|trait|const|type)` = 8, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 3; every pub item carries a doc comment under `#![deny(missing_docs)]` (lib.rs:13); the Result-returning `InteractionType` impls (shell_pool.rs:62, 70, 81) carry prose docs and their contract lives on the family trait in d2b-provider-wayland-policy.

### perf
- clean: seeds `format!(` = 0, `Vec::new()|VecDeque::new()|HashMap::new()|BTreeMap::new()` = 1, `.to_string()` = 0; the single `Vec::new()` (shell_pool.rs:82) is the empty desired-children return, the common case.

### conc
- N/A: seeds `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 0, `Atomic\w+|Ordering::` = 0, `thread_local!|unsafe impl (Send|Sync) for` = 0 all zero.

### async
- N/A: seeds `async fn|async move|.await` = 0, `tokio::spawn|spawn_blocking|JoinSet|select!|join!` = 0, `tokio::sync::(Mutex|RwLock|Notify)` = 0, `#[tokio::(main|test)]|Runtime::block_on` = 0 all zero in src; all async code lives in tests/.

### unsafe
- N/A: seeds 1-3 (`\bunsafe {|\bunsafe fn|\bunsafe impl|\bunsafe extern`, `// SAFETY:`, `transmute|from_raw|MaybeUninit|mem::zeroed`) = 0/0/0; manifest sets `unsafe_code = "forbid"`.

### ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0 all zero.

### macro
- N/A: seeds `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0 all zero.

### test
- clean: seeds `#[test]|#[tokio::test]` = 4, `assert_eq!|assert_ne!|assert!` = 12, `proptest!|insta::assert|rstest` = 0, `#[ignore]` = 0; four tests with doc comments - declaration surface, duplicate-registration refusal, dependency reads, and table-driven malformed-reference refusal with `{spec}` failure messages; no test that cannot fail.

### Coverage
- idiom: clean (seeds: 0/0/0)
- own: clean (seeds: 0/0/0/0)
- type: clean (seeds: 0/0/0)
- api: clean (seeds: 8/1/1)
- err: clean (seeds: 0/0/0/0)
- serde: N/A (seeds: 0/0/0/0 all zero; family decoder owns the spec decode)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency)
- docs: clean (seeds: 8/0/3)
- perf: clean (seeds: 0/1/0)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero in src)
- unsafe: N/A (seeds 1-3: 0/0/0; manifest forbid)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds: 4/12/0/0)
- supply: clean (X1 owns the lens; per-crate manifest check: all five deps used in src; async-trait + serde_json dev-deps used in tests)

## d2b-provider-shell-session

### idiom
- clean: seeds `for \w+ in 0\.\.` = 0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 0, `let mut \w+ = (String|Vec)::new\(\)` = 0; the `dependencies()` accumulation (shell_session.rs:76-80) is a conditional push the iterator form would obscure, so the plain form is right.

### own
- clean: seeds `.clone()` = 0, `.to_owned()|.to_vec()|.to_string()` = 1, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 0, `Cow<` = 0; the single `to_owned()` (shell_session.rs:118) builds the owned child name at the wire boundary, required.

### type
- clean: seeds `fn validate_\w+|fn check_\w+` = 0, `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0; only the unit struct `ShellSession`; the supervisor-child derivation is a pure function of the validated envelope.

### api
- clean: seeds `\bpub (fn|struct|enum|trait|type|const|mod) ` = 8, `pub .*\b(Arc|Rc|Box|RefCell)<` = 1, `^\s*pub use ` = 1; the `Arc<dyn SpecDecoder>` return (shell_session.rs:137) is required by the `DriverDescriptor.decoder` field; explicit house re-export list; private consts (`SHELL_SUPERVISOR_PROVIDER_REF`, `SHELL_SUPERVISOR_TEMPLATE`) stay private.

### err
- clean: seeds `.unwrap()|.expect()` = 0, `let _ = |.ok();` = 0, `panic!|unreachable!|todo!|unimplemented!` = 0, `enum \w*Error` = 0; failures are the family's `InteractionEffectError` propagated with `?`; the `invalid()` closure (shell_session.rs:94) maps impossible construction failures to `InvalidResource` without panicking.

### serde
- clean: seeds `derive)...Serialize` = 0, `serde)...)` = 0, `impl .*Deserialize.*for` = 0, `serde_json::from_|serde_json::to_` = 2; the two `serde_json::to_vec` calls (shell_session.rs:119-120) render the supervisor child's spec and metadata at the wire boundary; no derives needed because the spec is `json!`-built.

### obs
- N/A: seeds `println!|eprintln!` = 0, `(info|debug|warn|error|trace)!\("` = 0, `.instrument(|#[instrument` = 0, `tracing::|log::` = 0 all zero and the manifest lists no tracing/log dependency.

### docs
- clean: seeds `^\s*pub (fn|struct|enum|trait|const|type)` = 8, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 3; every pub item documented under `#![deny(missing_docs)]` (lib.rs:13); the Result-returning `InteractionType` impls carry prose docs, contract on the family trait.

### perf
- clean: seeds `format!(` = 1, `Vec::new()|VecDeque::new()|HashMap::new()|BTreeMap::new()` = 0, `.to_string()` = 0; the single `format!` (shell_session.rs:99) builds the supervisor child reference once per reconcile pass - cold, static (unmeasured).

### conc
- N/A: seeds `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 0, `Atomic\w+|Ordering::` = 0, `thread_local!|unsafe impl (Send|Sync) for` = 0 all zero.

### async
- N/A: seeds `async fn|async move|.await` = 0, `tokio::spawn|spawn_blocking|JoinSet|select!|join!` = 0, `tokio::sync::(Mutex|RwLock|Notify)` = 0, `#[tokio::(main|test)]|Runtime::block_on` = 0 all zero in src; all async code lives in tests/.

### unsafe
- N/A: seeds 1-3 (`\bunsafe {|\bunsafe fn|\bunsafe impl|\bunsafe extern`, `// SAFETY:`, `transmute|from_raw|MaybeUninit|mem::zeroed`) = 0/0/0; manifest sets `unsafe_code = "forbid"`.

### ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0 all zero.

### macro
- N/A: seeds `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0 all zero.

### test
- clean: seeds `#[test]|#[tokio::test]` = 5, `assert_eq!|assert_ne!|assert!` = 16, `proptest!|insta::assert|rstest` = 0, `#[ignore]` = 0; five tests with doc comments - declaration surface, duplicate refusal, dependency reads, supervisor-child spec asserted field by field (providerRef/template/processClass/executionRef/userRef/dependencies/ownerRef), and table-driven malformed-ref refusal; no test that cannot fail.

### Coverage
- idiom: clean (seeds: 0/0/0)
- own: clean (seeds: 0/1/0/0)
- type: clean (seeds: 0/0/0)
- api: clean (seeds: 8/1/1)
- err: clean (seeds: 0/0/0/0)
- serde: clean (seeds: 0/0/0/2)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency)
- docs: clean (seeds: 8/0/3)
- perf: clean (seeds: 1/0/0)
- conc: N/A (seeds: 0/0/0/0 all zero)
- async: N/A (seeds: 0/0/0/0 all zero in src)
- unsafe: N/A (seeds 1-3: 0/0/0; manifest forbid)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds: 5/16/0/0)
- supply: clean (X1 owns the lens; per-crate manifest check: all six deps used in src; async-trait dev-dep used in tests)

## d2b-provider-telemetry-binding

### idiom
- clean: seeds `for \w+ in 0\.\.` = 0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 1, `let mut \w+ = (String|Vec)::new\(\)` = 0; the one hand-written `impl Default for TelemetryBindingDriverFactory` (driver.rs:234) is required - `[ResourceTypeName; 1]` has no field-wise `Default` and `ResourceTypeName` does not implement it, so a derive is impossible.

### own
- clean: seeds `.clone()` = 20, `.to_owned()|.to_vec()|.to_string()` = 4, `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` = 0, `Cow<` = 0; every clone is explainable: the driver owns its `ResourceKey` and zone copies (driver.rs:269, 296, 421), watch targets and desired refs need owned values for the manager calls (321, 446, 454, 467), and the remaining 14 sit in `#[cfg(test)]` fixture code (702-844); the 4 `to_owned`/`to_vec` are owned strings at wire boundaries or test expectations (387, 967, 1144-1145).

### type
- tail-4#4 sev=low blast=leaf effort=S verdict=actionable - `TelemetryBindingStatus.phase: &'static str` is stringly-typed state with exactly two valid spellings (`PHASE_PENDING`, `PHASE_DEGRADED`), while the sibling otel crate models the same concept as the `TelemetryBindingPhase` enum - fix: introduce a local `TelemetryBindingPhase` enum with `as_str()` preserving the "Pending"/"Degraded" spellings and use it for the `phase` field, deleting the two `PHASE_*` consts - [packages/d2b-provider-telemetry-binding/src/driver.rs:168, packages/d2b-provider-telemetry-binding/src/driver.rs:170, packages/d2b-provider-observability-otel/src/controller.rs:288]
  evidence: seeds `fn validate_\w+|fn check_\w+` = 2 (test fn names only), `is_\w+: bool|\w+_flag: bool` = 0, `(mode|kind|state): String` = 0; finding from read: `phase: &'static str` (driver.rs:170) set only from the two consts (400-401, 477-480); sibling enum at d2b-provider-observability-otel/src/controller.rs:288-290 (`pub phase: TelemetryBindingPhase`); status is in-memory only (R11, README.md:77-78), so the spellings survive via `as_str()`.
- clean: `DeviceNodePath`-class parse-once newtypes absent here but the driver's `TelemetryBindingSpecEnvelope` and `TelemetryBindingDriverError` (private fields, const constructor) follow the model; no boolean-flag soup (`fenced`/`converged` are independent status facts).

### api
- clean: seeds `\bpub (fn|struct|enum|trait|type|const|mod) ` = 20, `pub .*\b(Arc|Rc|Box|RefCell)<` = 1, `^\s*pub use ` = 1; the `Arc<dyn SpecDecoder>` return (driver.rs:205) is required by the `DriverDescriptor.decoder` field; the re-export list is explicit (lib.rs:35-41); `TelemetryBindingSpecEnvelope` stays un-exported (pub in a private module), and `TelemetryBindingDriver` is an opaque pub type with a private constructor - deliberate.

### err
- clean: seeds `.unwrap()|.expect()` = 17, `let _ = |.ok();` = 1, `panic!|unreachable!|todo!|unimplemented!` = 3, `enum \w*Error` = 1; all 17 unwrap/expect and all 3 `panic!` sit in `#[cfg(test)]`; the one `let _ =` (driver.rs:509) is the deliberate validate-the-envelope-decodes pattern that still propagates errors with `?`; `TelemetryBindingDriverErrorKind` splits by caller action with stable wire codes via `as_str()` (driver.rs:130-136) - the taxonomy this lens recommends.

### serde
- clean: seeds `derive)...Serialize` = 0, `serde)...)` = 0, `impl .*Deserialize.*for` = 0, `serde_json::from_|serde_json::to_` = 7; the wire boundary is the `ResourceSpec` decode inside `telemetry_binding_spec_decoder` (driver.rs:205-211) and canonical-JSON round-trips of child payloads (199, 374, 388-390); no derives needed because the envelope wraps `CanonicalJsonObject`; every parse failure maps to a typed error kind.

### obs
- N/A: seeds `println!|eprintln!` = 0, `(info|debug|warn|error|trace)!\("` = 0, `.instrument(|#[instrument` = 0, `tracing::|log::` = 0 all zero and the manifest lists no tracing/log dependency.

### docs
- clean: seeds `^\s*pub (fn|struct|enum|trait|const|type)` = 20, `/// # (Examples|Errors|Panics|Safety)` = 0, `-> Result<` = 17; every pub item carries a doc comment under `#![deny(missing_docs)]` (lib.rs:14), including the contract-flag note on the Degraded projection (driver.rs:473-476); the Result-returning `ResourceDriver` impls (validate/recover/reconcile/delete) carry prose docs and their error contract lives on the trait in d2b-resource-runtime.

### perf
- clean: seeds `format!(` = 5, `Vec::new()|VecDeque::new()|HashMap::new()|BTreeMap::new()` = 5, `.to_string()` = 0; all 5 `format!` (driver.rs:764, 796, 1027, 1166, 1173) and 3 of the `Vec::new()` (684-686) are `#[cfg(test)]` fixtures; the production `Vec::new()` sites (270, 404, 909) are the empty-common case; static (unmeasured).

### conc
- clean: seeds `std::thread::|thread::spawn|thread::scope` = 0, `\bMutex<|\bRwLock<` = 4, `Atomic\w+|Ordering::` = 0, `thread_local!|unsafe impl (Send|Sync) for` = 0; all 4 `Mutex` hits (driver.rs:675-677, 839) are `tokio::sync::Mutex` inside `#[cfg(test)]` fixture doubles - test-only synchronization, no production shared state.

### async
- clean: seeds `async fn|async move|.await` = 20, `tokio::spawn|spawn_blocking|JoinSet|select!|join!` = 0, `tokio::sync::(Mutex|RwLock|Notify)` = 1 (cfg(test) import), `#[tokio::(main|test)]|Runtime::block_on` = 10 (cfg(test)); the production async surface is the `ResourceDriver` trait impls - no blocking calls, no guards held across `.await`, no spawn/select, and each pass is idempotent so cancellation at any await leaves a re-runnable state; `watch_once` (driver.rs:317-323) is documented best-effort with the requeue schedule as the recovery path.

### unsafe
- N/A: seeds 1-3 (`\bunsafe {|\bunsafe fn|\bunsafe impl|\bunsafe extern`, `// SAFETY:`, `transmute|from_raw|MaybeUninit|mem::zeroed`) = 0/0/0; manifest sets `unsafe_code = "forbid"`.

### ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` = 0, `catch_unwind` = 0, `repr\(C\)|repr\(transparent\)` = 0, `CStr|CString|c_char` = 0 all zero.

### macro
- N/A: seeds `macro_rules!` = 0, `proc_macro|syn::|quote!` = 0, `\$crate` = 0, `to_compile_error|new_spanned` = 0 all zero.

### test
- clean: seeds `#[test]|#[tokio::test]` = 13, `assert_eq!|assert_ne!|assert!` = 40, `proptest!|insta::assert|rstest` = 0, `#[ignore]` = 0; ten unit tests over a recording manager endpoint assert the observable contracts - child ensure order (collector Process before ingest Endpoint), second-pass convergence with no resync, fence on dangling dependency and foreign Provider, endpoint-first/process-last teardown order, one watch registration per target, recover adopt only on a current child set, and delete performing no effect past the manager cascade - plus the three documented registration tests; no test that cannot fail.

### Coverage
- idiom: clean (seeds: 0/1/0)
- own: clean (seeds: 20/4/0/0, all clones explainable)
- type: 1 finding(s)
- api: clean (seeds: 20/1/1)
- err: clean (seeds: 17/1/3/1, all panic sites in cfg(test))
- serde: clean (seeds: 0/0/0/7)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency)
- docs: clean (seeds: 20/0/17)
- perf: clean (seeds: 5/5/0)
- conc: clean (seeds: 0/4/0/0, test-only)
- async: clean (seeds: 20/0/1/10, production surface sound)
- unsafe: N/A (seeds 1-3: 0/0/0; manifest forbid)
- ffi: N/A (seeds: 0/0/0/0 all zero)
- macro: N/A (seeds: 0/0/0/0 all zero)
- test: clean (seeds: 13/40/0/0)
- supply: clean (X1 owns the lens; per-crate manifest check: all eight deps used in src; tokio dev-dep used in tests)