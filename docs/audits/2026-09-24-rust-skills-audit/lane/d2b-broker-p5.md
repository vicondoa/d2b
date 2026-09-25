# d2b-broker-p5 - d2b-broker - part 5/7
Baseline: 6ebdd4cec | LOC audited: 10950 (excl. src/generated/**) | modules: sys, ops::swtpm_dir, ops::nft, forwarding, ops::cgroup, ops::device, ops::hosts, fd_passing, lib
Lenses: idiom own type api err serde obs docs perf conc async unsafe ffi macro test | Partitions: src/sys.rs, src/ops/swtpm_dir.rs, src/ops/nft.rs, src/forwarding.rs, src/ops/cgroup.rs, src/ops/device.rs, src/ops/hosts.rs, src/fd_passing.rs, src/lib.rs

## idiom
- d2b-broker-p5#1 sev=low blast=leaf effort=S verdict=actionable - `format_errno` reverses `tmp` into `buf` by hand with an index loop (`for i in 0..len`), the exact case an iterator form reads as idiomatic - fix: replace the loop with `buf[..len].copy_from_slice(&tmp[..len])` plus `buf[..len].reverse()`, or fill via `buf.iter_mut().zip(tmp[..len].iter().rev())`; both stay allocation- and panic-free so the async-signal-safe contract is preserved - [packages/d2b-broker/src/sys.rs:2816-2820]
  evidence: seed `for \w+ in 0\.\.` = 5 hits (sys.rs:755, 850, 889, 2817; fd_passing.rs:321); the other four are bounded retry loops with side-effecting bodies where the plain `for` is the right shape, the reversal loop has a clean iterator equivalent.
- clean: seeds ran 5/0/8; no hand-written `Default/From/PartialEq/...` impls (0), statement-style `let mut x = String/Vec::new()` hits (8) are all read-into-buffer or parse-state-machine accumulators with real control flow; the single index-loop finding above was the only shape deviation.

## own
- clean: seeds ran 38/156/0; every clone read in context is explainable - owned map keys built from paths (sys.rs:2279, 2285, 2293), struct fields that must outlive a borrow (swtpm_dir.rs:584-586, 605-606), fd/binary values moved into argv or fixture closures (sys.rs:3905, 4184; forwarding.rs:466), and test fixtures; `Rc/RefCell/Arc<Mutex>/Arc<RwLock>/Cow` = 0 throughout; `to_owned/to_vec/to_string` mass is dominated by error-detail and audit rendering, not clone-to-please-the-borrow-checker.

## type
- clean: seeds ran 5/0/0; the five `validate_*`/`check_*` helpers (sys.rs:526 `validate_target_name`, swtpm_dir.rs:482 `check_resource_backed_state_dir`, nft.rs:534 `validate_projection_marker`, device.rs:335 `validate_opened_device`, hosts.rs:197 `validate_marker_ownership`) are single-boundary validators over genuinely untrusted fs/wire input with real check classes - parse-once newtypes would not remove a bug class here (stopping rule); no boolean-flag soup and no stringly-typed state (`is_/flag: bool` 0, `(mode|kind|state): String` 0).

## api
- d2b-broker-p5#2 sev=low blast=leaf effort=S verdict=actionable - `CgroupBundleContext::slice_path()` returns an owned `PathBuf` (cloning `parent_slice`) when a `&Path` return serves every in-crate call, forcing a clone per call at the `is_under_slice` check and duplicating the value - fix: change the return to `&Path` (`pub fn slice_path(&self) -> &Path`); the builder methods `vm_interior_path`/`vm_leaf_path`/`vm_role_leaf_path` keep their owned joins and `fields.slice_path`/tuple sites keep their one owned conversion - [packages/d2b-broker/src/ops/cgroup.rs:126-129, packages/d2b-broker/src/ops/cgroup.rs:343]
  evidence: census `slice_path\(` over packages/, nixos-modules/, tests/, docs/reference/, labs/ = 9 hits (8 in-crate, 1 unrelated `d2b_host::cgroup::d2b_slice_path`); call sites that would stop cloning: cgroup.rs:249 (owns once), 323, 343 (read-only).
- clean remainder: seeds ran 181/0/4; the wide `pub` surface is deliberate for a daemon-internal crate (all 4 `pub use` arms are the house single-surface pattern: lib.rs:56, forwarding.rs:42); the one smart-pointer-in-signature (`ForwardFuture` = `Pin<Box<dyn Future + Send>>`, forwarding.rs:114) is a documented trait-object seam for `dyn OperationForwarder` - not a hidden internals leak.

## err
- d2b-broker-p5#3 sev=medium blast=leaf effort=S verdict=actionable - `WriteMarkerBlockError::Io(String)` (hosts.rs:122) flattens `io::Error` into a Display-only string at three `map_err` sites, destroying the error kind/source so a caller cannot classify NotFound vs permission vs other without string-matching - fix: switch the variant to `Io(#[source] io::Error)` (thiserror or a hand-written `#[source]` accessor) and render the same "update-hosts marker splice: {err}" prefix so the visible message is unchanged - [packages/d2b-broker/src/ops/hosts.rs:122, packages/d2b-broker/src/ops/hosts.rs:149, packages/d2b-broker/src/ops/hosts.rs:153, packages/d2b-broker/src/ops/hosts.rs:180]
  evidence: seed `enum \w*Error` = 7; the siblings `CgroupOpError`, `ApplyWithCoexistenceError`, `ProjectionMutationError`, `FdPassingError`, `SwtpmHardenError` already carry typed fields, so `Io(String)` is the outlier; `\.unwrap\(\)|\.expect\(` = 260 hits, every non-test survivor is an `expect` on a literally-built C string (sys.rs:695, 714, 746, 974, 1860, 1928) or an unreachable-invariant expect (nft.rs:563 - the `"}"` line with `current.is_none()` is continued earlier, so `current.take()` cannot fail); `let _ =` = 34, all best-effort cleanup (fd close, child SIGKILL/reap, temp-dir removal); `panic!` = 10, all in `#[cfg(test)]`.

## serde
- clean: seeds ran 7/6/0/18; the derive set (swtpm_dir.rs `MarkerData`/`MarkerOrigin`, nft.rs `ApplyNftablesAudit`/`NftHashSidecar`, device.rs `PreOpenDecision`/`OpenAuditRecord`/`RoleDeviceClaim`) is consistently `rename_all`-conventioned, `MarkerData` correctly carries `deny_unknown_fields` as the one tamper-sensitive payload, no hand-written `Deserialize` (0), and no `try_from` is missing - every shape is an emit/parse pair of the broker's own audit/marker payloads, not untrusted service input; `deny_unknown_fields` absence on service-consumed audit messages is the recorded deliberate posture.

## obs
- clean: seeds ran 6/0/0/0; the six `println!`/`eprintln!` hits (sys.rs:1417, 1421, 3893, 4201, 4217; swtpm_dir.rs:1557) are all `#[cfg(test)]` skip messages, no interpolated-message events (0), no `instrument` spans (0), and no `tracing`/`log` use in this lane's files (0) - the lane's observability surface is the typed audit records (SwtpmDirAudit, OpenAuditRecord, ApplyNftablesAudit), which are data, not log lines.

## docs
- d2b-broker-p5#4 sev=medium blast=leaf effort=S verdict=actionable - the public fd-ownership API in fd_passing.rs is undocumented: `FdPassingError` (13), `FdRegistry::register`/`clear` (37, 41), and the whole `FdLease` surface (`new`/`raw`/`release`, 55-70) have no doc comments even though the load-bearing contract is non-obvious - `FdLease` closes the fd on drop and `release()` disarms that close, which is exactly what ADR 0034 fd-transfer callers must know - fix: add one-line docs to each item stating ownership (who closes, what `release` disarms) and the `recv_*`/`send_fds` error variants - [packages/d2b-broker/src/fd_passing.rs:13, packages/d2b-broker/src/fd_passing.rs:31-70]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 164; the documented peers in the same module (recv_fds, recv_fds_with_capacity, recv_one_fd, close_received_fds) show the house expectation, making the bare FdLease/FdRegistry group the gap.
- d2b-broker-p5#5 sev=medium blast=leaf effort=S verdict=actionable - pub syscall wrappers `peer_credentials` (121) and `tun_set_persist`/`tun_set_owner`/`tun_set_group` (185, 195, 210) carry no doc comments while their twins `peer_uid` and `tun_create_tap_fd` do; the contracts are non-obvious (peer uid/gid/pid triple semantics; TUNSETPERSIST/OWNER/GROUP ioctl semantics and the ifname binding) - fix: add one-line docs plus the `# Errors` conditions (ioctl failure, uid/gid out of `c_int` range) - [packages/d2b-broker/src/sys.rs:120-121, packages/d2b-broker/src/sys.rs:185, packages/d2b-broker/src/sys.rs:195, packages/d2b-broker/src/sys.rs:210]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 164 overall, these four are the undocumented pub items in the module's leading wrapper group (rest documented).
- d2b-broker-p5#6 sev=low blast=leaf effort=M verdict=actionable - the `path_safe` module's pub helpers (`refuse_symlink`, `refuse_world_writable_parent`, `refuse_non_root_parent`, `read_to_string_nofollow`, `write_nofollow`, `remove_nofollow`, `ensure_dir`, `ensure_dir_preserve_existing`) lack per-item doc comments; the contract lives only in the module-level doc, so rustdoc item pages are empty and the reader must open the module header for each helper's safety rule - fix: promote each module-doc bullet to a one-line `///` on its item (or add `#[doc = "..."]` links), keeping the module doc as the index - [packages/d2b-broker/src/sys.rs:258, packages/d2b-broker/src/sys.rs:268, packages/d2b-broker/src/sys.rs:329, packages/d2b-broker/src/sys.rs:398]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 164; the module doc at sys.rs:205-249 names every helper, confirming intent, and `/// # (Examples|Errors|Panics|Safety)` = 1 (only `getsockopt_int`'s `# Safety`), so the canonical-section convention is otherwise absent here.

## perf
- d2b-broker-p5#7 sev=low blast=leaf effort=S verdict=actionable - `projection_digest` builds the hex digest with `raw.iter().map(|byte| format!("{byte:02x}")).collect::<String>()`, allocating one `String` per byte (32 hex bytes) before the final collect - fix: `String::with_capacity(70)` and `write!`/`push_str` each byte, or a 16-entry hex lookup - [packages/d2b-broker/src/ops/nft.rs:784-790]
  evidence: seed `format!\(` = 108; this is the one site allocating inside a per-element `map` closure, all other `format!` are error/audit/text-artifact sites (cold per repo false positives).
- d2b-broker-p5#8 sev=low blast=leaf effort=S verdict=actionable - `handle_open_cgroup_dir` renders `canonical_path.display().to_string()` twice (the audit field at 341 and the outcome at 368) in the same call - fix: bind `let cgroup_id = canonical_path.display().to_string();` once and reuse for both the audit record and `OpenCgroupDirOutcome` - [packages/d2b-broker/src/ops/cgroup.rs:341, packages/d2b-broker/src/ops/cgroup.rs:368]
  evidence: seed `\.to_string\(\)` = 33; the duplicate is the same expression on the same value in one function body.
- clean remainder: seeds ran 108/27/33; all other `format!`/`Vec::new`/`to_string` sites are in error paths, audit rendering, nft-script text building, or fixture/test code (deliberate per card false positives); nothing else is a hot path with growth-by-push.

## conc
- clean: seeds ran 23; production concurrency use is a single `static TMP_NAME_COUNTER: AtomicU64` with `Ordering::Relaxed` (sys.rs:474, 541) - the weakest correct ordering for a globally-unique counter, exactly what the skill prescribes; every `Mutex`/`thread::spawn`/`Arc<AtomicUsize>` hit is test-only (forwarding.rs test peers, fd_passing.rs `fd_test_lock`, swtpm_dir.rs test thread, cgroup.rs `RecordingAuditSink` under `fake-backends`); no `unsafe impl Send/Sync` and no `thread_local!`/`static mut`.

## async
- d2b-broker-p5#9 sev=high blast=leaf effort=S verdict=actionable - the async `harden()` path (invoked from `live_handlers.rs:3142` on the runtime) calls `apply_ancestor_traverse_acl` -> `run_setfacl_op_on_fd` (sys.rs:1866-1893), which does a synchronous `fork` + `execv(setfacl)` + blocking `waitpid` loop with no `.await` and no `spawn_blocking`, stalling the executor worker for the duration of a subprocess spawn - fix: wrap the setfacl fork/exec/wait in `tokio::task::spawn_blocking` (moving the fd across as `OwnedFd`) and `.await` the join handle in `harden` - [packages/d2b-broker/src/ops/swtpm_dir.rs:770, packages/d2b-broker/src/sys.rs:1866-1893, packages/d2b-broker/src/live_handlers.rs:3142]
  evidence: seed `async fn|async move|\.await` = 134, `tokio::spawn|...|#\[tokio::(main|test)\]|Runtime::block_on` = 26; call chain shows the sync `waitpid` loop sits inside an async function with no yield point between entry and the blocking wait; no async-gate-allow marker or blocking-census entry covers this site (the scanner and disallowed-methods list target tokio sync forms and lock acquisition, not fork/exec/waitpid).
- clean remainder: the lane's other async sites follow the house patterns - `acquire_projection_lock` (nft.rs:809-832) polls non-blocking `F_OFD_SETLK` with 25ms async sleeps (R13, documented), `read_live_table_json_optional` uses `tokio::process::Command` (nft.rs:866-881), swtpm marker reads/writes use `tokio::fs` with a single quick `rustix::fs::fsync`; none block a worker and none keep a guard across `.await`.

## unsafe
- d2b-broker-p5#10 sev=medium blast=leaf effort=M verdict=actionable - most of `sys.rs`'s 101 `unsafe` blocks carry no `// SAFETY:` comment (only 25 SAFETY comments in the file, concentrated on the risky corner: clone3, fork, pre_exec, pidfd, mount); the raw wrapper layer - `openat2_raw`, `openat_raw`, `renameat2_raw`, `renameat_raw`, `mkdirat_raw`, `unlinkat_raw_with_flags`, `fstatat_raw`, `linkat_empty_path_raw` (593-699), the child-context helpers `mkdir_one`/`mknod_device_bind_target`/`install_pre_opened_fds` (2630, 2670, 2109) and the mount/mask helpers `apply_mount_actions(_debug)`/`apply_device_mask_and_binds` (2591, 2694) - call libc with only `#[allow(unsafe_code)]`, so a reader cannot distinguish audited from un-audited blocks in the sanctioned quarantine - fix: add a one-line SAFETY to each bare block stating the invariant it upholds (CString/pointer liveness and NUL-termination, dirfd validity, errno propagation, freshly-owned return fd), matching the existing clone3/fork comments - [packages/d2b-broker/src/sys.rs:593, packages/d2b-broker/src/sys.rs:615, packages/d2b-broker/src/sys.rs:630, packages/d2b-broker/src/sys.rs:653, packages/d2b-broker/src/sys.rs:676, packages/d2b-broker/src/sys.rs:685, packages/d2b-broker/src/sys.rs:697, packages/d2b-broker/src/sys.rs:2109, packages/d2b-broker/src/sys.rs:2591, packages/d2b-broker/src/sys.rs:2630, packages/d2b-broker/src/sys.rs:2670, packages/d2b-broker/src/sys.rs:2694]
  evidence: seed `\bunsafe \{|\bunsafe fn|...` = 103 hits; `// SAFETY:` = 25; `transmute|from_raw|MaybeUninit|mem::zeroed` = 39 (the `mem::zeroed` sites are int/struct-value kernels like `libc::stat`/`ifreq`/`clone_args` where zero is a valid bit pattern - sound, but the same bare-block comment gap applies). The allow pattern itself is the sanctioned boundary (U1 d8) and is not re-flagged; this finding targets only missing per-block justification on sound code.

## ffi
- clean: seeds ran 66; every hit is within the sanctioned libc syscall-wrapper surface and matches the card's repo false positives - `CString`/`c_char` in the `nix`-replacing raw syscall layer (sys.rs), one `#[repr(C)] OpenHow` kernel uapi shape (sys.rs:497), no `extern "C"` function, no `no_mangle`, no `catch_unwind`, and no foreign caller exists anywhere in the lane scope; nothing here crosses a non-Rust calling convention.

## macro
- N/A (seeds: 0/0/0/0 all zero; no macro_rules!, proc-macro, syn/quote, `$crate`, or compile-error machinery in any assigned file)

## test
- clean: seeds ran 109/91/0/0; the in-file `#[cfg(test)]` modules (sys.rs, swtpm_dir.rs, nft.rs, forwarding.rs, cgroup.rs, device.rs, hosts.rs, fd_passing.rs) assert behavior and error variants rather than Display strings - fd tests serialize via `fd_test_lock`, the forwarder tests cover refusal/fd-count-mismatch/budget-bound paths, swtpm tests assert fail-closed reasons and contents-preservation, and the sys.rs tests document skip conditions for privileged/unprivileged-userns cases; no `#[ignore]`, no proptest/insta/rstest (unit tests fit these pure-ish functions), and no test that cannot fail was observed. (Crate-level `tests/**` is shared across the broker's seven parts; this lane's `test` judgement covers its own modules.)

## Coverage
- idiom: 1 finding(s)
- own: clean (seeds ran: 38/156/0)
- type: clean (seeds ran: 5/0/0)
- api: 1 finding(s)
- err: 1 finding(s)
- serde: clean (seeds ran: 7/6/0/18)
- obs: clean (seeds ran: 6/0/0/0)
- docs: 3 finding(s)
- perf: 2 finding(s)
- conc: clean (seeds ran: 23)
- async: 1 finding(s)
- unsafe: 1 finding(s)
- ffi: clean (seeds ran: 66)
- macro: N/A (seeds: 0/0/0/0 all zero; no macro definitions in assigned files)
- test: clean (seeds ran: 109/91/0/0)
