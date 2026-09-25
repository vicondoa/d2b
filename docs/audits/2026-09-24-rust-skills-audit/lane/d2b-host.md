# d2b-host - d2b-host
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 11,767 (excl. src/generated/**; src 11,316 + tests 451) | modules: bridge_port, cgroup, devices, hardlink_farm, host_generation, host_prep_dag, ifname, ioctl_policy, media, modules, netlink, nftables, ownership_matrix, routes, seccomp, bin/
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate

## idiom
- clean: seeds ran: 1/0/18. The single `for \w+ in 0..` hit is a test-fixture side-effect loop (tests/activation_helper_build_farm.rs:23; plain `for` is right per the skill); all 18 `let mut X::new()` sites are legitimate accumulation into owned buffers or error vectors with real side effects; zero hand-written `Default|From|Debug|Clone|Hash|Eq|PartialEq` impls; no `fn get_` names found.

## own
- clean: seeds ran:   66/335/3. Every `.clone()` (66) and `.to_owned()|.to_string()` (335) inspected: each is an explainable error-record payload, a multiple-step construction from one borrowed value (host_prep_dag bundle refs, devices rows, ownership_matrix drift records), a test fake (RefCell-backed ops logs, drift simulators), or a wire/serde boundary; the 3 shared-ownership hits are 2 test-fake `RefCell<Vec<_>>` fields (netlink.rs:357, nftables.rs:891,894) and 1 test-fake `Mutex<Inner>` (cgroup.rs:796); no Rc/Arc/Cow in production code.

## type
- d2b-host#1 sev=medium blast=family effort=M verdict=actionable - `BusId(pub String)` carries no lexical invariant: `BusId::new` accepts any string and the field is public, so the busid grammar is re-validated at every consumer instead of once at the type boundary - fix: make the field private, validate in `BusId::new` (reusing `media::validate_usb_busid`'s grammar) or add `TryFrom<&str>`, keep `#[serde(transparent)]`; then delete the three broker re-validation sites - [packages/d2b-host/src/nftables.rs:609, packages/d2b-broker/src/ops/media.rs:194]
  evidence: seed `fn validate_\w+|fn check_\w+` = 10 hits; census: `validate_usb_busid` over `packages/` = 4 hits (3 broker call sites + defined here); `BusId(` literal construction over `packages/` = 0 external hits.

## api
- d2b-host#2 sev=low blast=leaf effort=S verdict=actionable - `HostPrepStepId(pub String)` exposes the inner `String` on a `#[serde(transparent)]` newtype whose only constructor `new` is private, so literal construction is the only external path and the `{vm}:{kind}` id convention stays unenforced - fix: make the field private (serde transparent round-trips unchanged; `as_str()` already exists) and add a public constructor if integrators need one - [packages/d2b-host/src/host_prep_dag.rs:85]
  evidence: seed `\bpub (fn|struct|enum|trait|type|const|mod) ` = 311 hits; census: `HostPrepStepId(` over `packages/` = 0 external literal constructions (in-crate tests only).

## err
- clean: seeds ran:   290/23/15/14. All 290 unwrap/expect hits are in `#[cfg(test)]` modules or two justified non-test sites (`media.rs:322` after an explicit `len() != 1` guard; `host_prep_dag.rs:503` on a statically-built DAG with a named invariant); the 23 `let _ =` sites are deliberate best-effort fsync/cleanup shrugs; alla 15 panic! hits are test-arm `panic!("expected ...")` plus the documented invariant `assert!` at seccomp.rs:132 (with `# Panics` contract, and the ioctl matrix is bounded below 251 by construction); the 14 error enums are closed and wire-coded (`code()`, `as_kebab_case()`, serde-tagged`, no string-matching taxonomy.



## serde
- d2b-host#3 sev=low blast=family effort=S verdict=actionable - `NftBatch` derives `Deserialize` but its `&'static str` fields pin the generated impl to `'de: 'static` (serde's `impl<'de: 'a, 'a> Deserialize<'de> for &'a str`), so the type cannot be deserialized from any runtime input; the derive is dead and misleads (the broker only ever `NftBatch::parse`s text or constructs batches) - fix: drop `Deserialize` from the `NftBatch` derive (keep `Serialize`), or make `table_family`/`table_name` owned `String` if round-trip is ever intended - [packages/d2b-host/src/nftables.rs:229]
  evidence: seed `derive\([^)]*(De)?[Ss]erialize|serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)|impl .*Deserialize.*for|serde_json::(from|to)_` = 125 hits; census: no `serde_json::from_*` on `NftBatch` over `packages/` = 0 hits; `NftBatch::parse` call sites in d2b-broker = 4.



## obs
- clean: seeds ran:   28/0/0/0. Every println!/eprintln! hit lives in `bin/d2b-activation-helper.rs` and is CLI product output or the helper's JSON wire protocol on stdout (card false positive for `bin/**`); zero tracing/log, zero instrument spans, zero interpolated telemetry events (this crate deliberately carries no telemetry dependency).

## docs
- d2b-host#4 sev=low blast=leaf effort=M verdict=actionable - Result-returning pub functions document failure conditions in prose but carry no `# Errors` sections; the crate has only 2 canonical sections total (host_prep_dag.rs:316, seccomp.rs:112) against ~119 `-> Result<` signatures (validate_readback, parse_request, gunzip_inflate, ...) - fix: add `# Errors` sections naming the failure conditions to the pub Result-returning fns, starting with the wire-boundary parsers (host_generation.rs, media.rs, nftables.rs) - [packages/d2b-host/src/bridge_port.rs:127, packages/d2b-host/src/host_generation.rs:103, packages/d2b-host/src/media.rs:41]
  evidence: seeds `^\s*pub (fn|struct|enum|trait|const|type)` = 284, `/// # (Examples|Errors|Panics|Safety)` = 2, `-> Result<` = 119.
- d2b-host#5 sev=low blast=leaf effort=S verdict=actionable - Pub items in impl blocks lack doc comments: `Controller::REQUIRED`, `Controller::as_str`, `Controller::from_token`, `BusId::new`, `HostPrepStepId::as_str` - fix: one-line doc comments, with `from_token` documenting the token grammar it accepts - [packages/d2b-host/src/cgroup.rs:53, packages/d2b-host/src/cgroup.rs:71, packages/d2b-host/src/cgroup.rs:85, packages/d2b-host/src/nftables.rs:612, packages/d2b-host/src/host_prep_dag.rs:92]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 284 hits; the five anchors carry no preceding `///` line (verified by neighborhood read).

## perf
- d2b-host#6 sev=low blast=leaf effort=S verdict=actionable - Hex digests are built with `format!("{b:02x}")` per byte, allocating a fresh String per byte (32+ allocations per digest) in `Sha256::of` and `generation_id` - fix: write into one preallocated `String::with_capacity(64)` via `write!`/`fmt::Write`, or share a hex helper - [packages/d2b-host/src/nftables.rs:46, packages/d2b-host/src/hardlink_farm.rs:628]
  evidence: static (unmeasured); seed `format!\(` = 116 hits, of which these two are per-byte loops (cold paths: per batch apply / per generation build).

## conc
- clean: seeds ran:: 0/1/0/0. The single `Mutex<` hit is the test-only `FakeCgroupBackend.inner` (cgroup.rs:796); no threads, no atomics/Ordering, no thread_local, no manual Send/Sync claims in the crate (the fake's `Mutex` is exercised from single-thread tests and needs no ordering story).

## async
- clean: seeds ran::   318/1/0/18. The crate's async surface (hardlink_farm + bin)drove entirely by `tokio::fs` I/O with the runtime created once at the binary entry (`#[tokio::main(flavor = "current_thread")]`); the single spawn-family hit is a policy comment (hardlink_farm.rs:1554, "async-purity policy bans spawn_blocking"), zero `tokio::sync::*`, zero guards held across awaits; sync helpers (`current_boot_id`, `process_identity`, `safe_usb_block_candidates`, `mirror_metadata`'s chown(syscall) carry per-site `#[allow(clippy::disallowed_methods, reason = "synchronous path")]` sanctions tracked by the blocking census (baseline lists all d2b-host blocking APIs at 0) - policy-confirmed, not re-flagged.



## unsafe
- clean: seeds ran: 0/0/9/5. Zero actual `unsafe` blocks/fns/impls in the crate (crate-root `#![forbid(unsafe_code)]` at lib.rs:20); alla 9 `from_raw` hits are safe constructors (`rustix::fs::FileType::from_raw_mode`, `Uid::from_raw`, `Gid::from_raw`, `std::io::Error::from_raw_os_error`), the 5 `unsafe_code` hits are the forbid declaration and its rationale comments; the manifest-level `forbid` explains why `nix`'s safe fchown wrapper is used over rustix's unsafe one (Cargo.toml comment).

## ffi
- clean: seeds ran:  0/1/1/0. The `repr(C)` hit is the deliberate `BpfInstruction` layout matching `libc::sock_filter` for the broker's quarantined sys.rs (card false positive); the `catch_unwind` hit is a test-only `debug_assert!` exercise (cgroup.rs:1137); no extern "C", no no_mangle, no CStr/CString/c_char cross any boundary in this crate.



## macro
- clean: seeds ran: 1/0/0/0. The single `macro_rules!` (bridge_port.rs:133 `check!`) is a local impl-per-field generator over 5 bool fields of a Copy wire struct with the narrowest fragment specifier (`$field:ident`), defined and consumed inside one function - a genuine last-resort use, not a finding.



## test
- d2b-host#7 sev=medium blast=family effort=M verdict=actionable - `NftBatch::parse` (the ~200-line nft script dialect parser with 15+ error paths) has no tests: zero calls to `parse` exist in the crate's test modules, while the broker feeds it live script bodies on its nft apply path - fix: table-driven parse tests (valid script, malformed header, foreign family/table, missing hook priority, unterminated chain, trailing content) asserting `ParseNftScriptError` variants - [packages/d2b-host/src/nftables.rs:245]
  evidence: seeds `#\[test\]|#\[tokio::test\]` = 163, `assert_eq!\(|assert_ne!\(|assert!\(` = 387 hits over src+tests; census: `NftBatch::parse` over `packages/` = 5 hits (1 definition, 4 broker call sites at ops/nft.rs:302,356 and runtime.rs:8598,10007, 0 tests).
- d2b-host#8 sev=low blast=leaf effort=S verdict=actionable - `package_digest_includes_bytes_read_through_store_symlinks` writes fixtures to a CWD-relative `target/` directory (the cargo build dir), polluting build artifacts and failing under a read-only target; every sibling test uses `tempdir()` - fix: use `tempfile::tempdir()` like the sibling tests - [packages/d2b-host/src/bin/d2b-activation-helper.rs:792]
  evidence: seed `assert_eq!\(|assert_ne!\(|assert!\(` = 387 hits; the anchor is the only test in the crate using a CWD-relative path (`PathBuf::from("target")`, bin/d2b-activation-helper.rs:792-793) instead of a tempdir.



## Coverage
- idiom: clean (seeds ran: 1/0/18)
- own: clean (seeds ran: 66/335/3)
- type: 1 finding(s)
- api: 1 finding(s)
- err: clean (seeds ran: 290/23/15/14)
- serde: 1 finding(s)
- obs: clean (seeds ran: 28/0/0/0)
- docs: 2 finding(s)
- perf: 1 finding(s)
- conc: clean (seeds ran: 0/1/0/0)
- async: clean (seeds ran: 318/1/0/18)
- unsafe: clean (seeds ran: 0/0/9/5)
- ffi: clean (seeds ran: 0/1/1/0)
- macro: clean (seeds ran: 1/0/0/0)
- test: 2 finding(s)