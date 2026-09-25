# d2b-broker-p7 - d2b-broker - part 7/7
Baseline: 6ebdd4cec | LOC audited: 10265 (excl. src/generated/**, non-Rust src/ops/state-posture-contract.json) | modules: ops::media, kernel_ops, ops::network, catalog, ops::state_dir, protocol, bootstrap
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 7/7 (src/ops/media.rs, src/kernel_ops.rs, src/ops/network.rs, src/catalog.rs, src/ops/state_dir.rs, src/protocol.rs, src/bootstrap.rs; src/ops/state-posture-contract.json excluded, non-Rust wire contract)

## idiom
- clean: seeds `for \w+ in 0\.\.`=0, `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`=0, `let mut \w+ = (String|Vec)::new\(\)`=14; all 14 accumulation sites are async `read_dir`/`fill_buf` loops with early returns, test line readers, and hash-input string building - no iterator-pipeline conversions warranted.

## own
- d2b-broker-p7#1 sev=low blast=leaf effort=S verdict=actionable - enroll's registry-read fallback clones the just-written record although it is never used again - fix: replace `unwrap_or_else(|_| vec![record.clone()])` with `unwrap_or_else(|_| vec![record])` in `enroll` - [packages/d2b-broker/src/ops/media.rs:220]
  evidence: `\.clone\(\)` seed = 85 hits; site read confirms `record` is only borrowed (`write_registry_record(resolver,&record)`) and then moved into the fallback `vec!` - no use after line 220; combined own seed mass 358 (>200): sampled:  50 of 358 hits (all seed-1 hit lines read in full; seed-2 hits are owned-build string materialization and wire-render clones - none avoidable; the sole `RefCell<` (network.rs:984)is a `#[cfg(test)]` fake).

## type
- d2b-broker-p7#2 sev=medium blast=leaf effort=S verdict=actionable - QmpAttachCleanup models its four-step rollback as four bools (16 states, ~5 valid)with a fixed teardown order - fix: replace `device_added/raw_added/file_added/fdset_added: bool` with an ordered step list or enum so rollback order cannot drift from the attach order - [packages/d2b-broker/src/ops/media.rs:889-892]
  evidence: `is_\w+: bool|\w+_flag: bool` seed =  1 hit (the four fields at media.rs:889-892; rollback order fixed at 898-943(`device_del`->`blockdev-del` raw->file->`remove-fd`); the valid step set lives only in the attach flow - a future fifth step would silently bypass teardown).

## api
- d2b-broker-p7#3 sev=low blast=leaf effort=S verdict=actionable - kernel_table clones the whole KernelConfig into an Arc because it takes `&KernelConfig` - fix: take `config: KernelConfig` by value (or `Arc<KernelConfig>`) so the one-time clone disappears; the per-handler Arc clones stay - [packages/d2b-broker/src/kernel_ops.rs:99-100]
  evidence: `pub .*\b(Arc|Rc|Box|RefCell)<` seed =  0 hits (no managed types in signatures; the Arc appears only in the fn body); census: `kernel_table` over packages/,nixos-modules/,tests/,docs/reference/,labs/ = 7 hits (kernel_ops.rs:99 + 6 in-crate call sites in runtime.rs:7145,14485,14798,15227,15545,17597, all passing a locally-built `&KernelConfig`).

## err
- clean: seeds `\.unwrap\(\)|\.expect\(`=289, `let _ = |\.ok\(\);`=28, `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(`=1, `enum \w*Error`=3 (sampled:  50 of 321 hits; every production unwrap/expect site read: all are invariant expects (`BrokerOperationRow` committed rows at kernel_ops.rs:667,670,673,699,702,705; validated fdset id at media.rs:705;`[u8;4]` prefix try_into at protocol.rs:98,144 - the rest are in-file `#[cfg(test)]` assertions; the 28 `let _ =`/`.ok();` sites are best-effort rollback/server patterns; the three error enums (`MediaOpError`, `NetworkOpError`, `PrepareStateDirError`) carry stable wire codes - no panic-policy breach found)

## serde
- clean: seeds `derive\([^)]*(De)?[Ss]erialize`=13, `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)`=46, `impl .*Deserialize.*for`=0, `serde_json::from_|serde_json::to_`=36; all wire types (bootstrap.rs wire module, media registry records, network.rs `PersistentTapRealization`, protocol framing values)use `rename_all`/`deny_unknown_fields`/internal enum tags, deliberate `#[serde(default)]` on optional fields; no hand-written deserializers, no parse-validation gap found.

## obs
- clean: seeds `\bprintln!\(|\beprintln!\(`=0, `(info|debug|warn|error|trace)!\(`=1, `\.instrument\(|#\[instrument`=0, `tracing::|log::`=4 (1 real: media.rs:431 structured `tracing::warn!` with named fields `vm_id`/`media_ref`/`slot`; 3 false positives: `catalog::` x2 and `Backlog::` match the `log::` alternation); no secret/PII-in-log paths found.



## docs
- d2b-broker-p7#4 sev=medium blast=leaf effort=M verdict=actionable - media.rs's public ops surface carries no doc comments - fix: add doc blocks to each: MediaOpError variants, the four outcome structs (esp. BootOutcome's four booleans),and the pub ops fns (`enroll`/`refresh_registry`/`boot`/`system_powerdown`/`query_status`/`quit`/`attach`/`detach`),covering preconditions, outcome semantics,and `# Errors` on the `Result` fns - [packages/d2b-broker/src/ops/media.rs:35, packages/d2b-broker/src/ops/media.rs:167-186, packages/d2b-broker/src/ops/media.rs:188, packages/d2b-broker/src/ops/media.rs:238, packages/d2b-broker/src/ops/media.rs:254, packages/d2b-broker/src/ops/media.rs:270, packages/d2b-broker/src/ops/media.rs:281, packages/d2b-broker/src/ops/media.rs:322, packages/d2b-broker/src/ops/media.rs:331, packages/d2b-broker/src/ops/media.rs:339]
  evidence: docs seeds `^\s*pub (fn|struct|enum|trait|const|type)`=102, `/// # (Examples|Errors|Panics|Safety)`=0; item-by-item read of media.rs:35-56,167-186,188-339 confirms no preceding `///` blocks on any pub item in this module.
- d2b-broker-p7#5 sev=medium blast=leaf effort=S verdict=actionable - protocol.rs's framing surface (cap const + sync framing fns)carries no docs although it is the broker wire contract - fix: doc `MAX_FRAME_SIZE` and the six framing fns (length-prefix format, cap enforcement, `Option::None` on empty socket, fd-ancillary semantics), mirroring the async wrappers' existing docs - [packages/d2b-broker/src/protocol.rs:13, packages/d2b-broker/src/protocol.rs:16, packages/d2b-broker/src/protocol.rs:29, packages/d2b-broker/src/protocol.rs:43, packages/d2b-broker/src/protocol.rs:52, packages/d2b-broker/src/protocol.rs:85, packages/d2b-broker/src/protocol.rs:121]
  evidence: docs seeds 102/0 as above; the async twins (`AsyncSeqpacket`, `AsyncSeqpacketListener`, `connect_seqpacket_bounded`)do carry docs, so the gap is confined to the sync framing path; census: `connect_seqpacket|bind_seqpacket|send_json_frame|recv_json_frame|MAX_FRAME_SIZE` over packages/,nixos-modules/,tests/,docs/reference/,labs/ = heavy use (runtime.rs, forwarding.rs, envelope/, d2bd-runtime/, d2b-contracts/, tests/, docs/reference/tap-dag-contract.md).
- d2b-broker-p7#6 sev=medium blast=leaf effort=S verdict=actionable - state_dir.rs publishes nine undocmented pub items (types + fns)with no `# Errors` on the `io::Result` fns - fix: add item-level doc contracts to `DirKind`, `PrepareDirRequest`, `PrepareDirAudit`, `ReplaceOrCreateResult`, `prepare_dir`, `live_prepare_runtime_dir`, `PreparedStateDir`, `live_prepare_state_dir` (and `# Errors` where Result) - [packages/d2b-broker/src/ops/state_dir.rs:47, packages/d2b-broker/src/ops/state_dir.rs:53, packages/d2b-broker/src/ops/state_dir.rs:70, packages/d2b-broker/src/ops/state_dir.rs:83, packages/d2b-broker/src/ops/state_dir.rs:89, packages/d2b-broker/src/ops/state_dir.rs:161, packages/d2b-broker/src/ops/state_dir.rs:204, packages/d2b-broker/src/ops/state_dir.rs:211]
  evidence: docs seeds 102/0 as above; field-level `///` comments exist (e.g. mode units, vm_id_or_scope)but no item-level contract on any of the nine pub items, and `prepare_dir`'s root-ownership refusal guard is explained only in code comments.

## perf
- d2b-broker-p7#7 sev=medium blast=wide effort=M verdict=actionable - recv_json_frame allocates and zeroes a 1 MiB buffer (`MAX_FRAME_SIZE + 4`)per received frame on every broker/client envelope path - fix: peek the 4-byte length prefix (`recvmsg` with `MSG_PEEK`)then allocate `declared + 4` exactly,, or thread a reusable buffer through the receive path; apply the same size-exactness to `recv_json_frame_with_fds` - [packages/d2b-broker/src/protocol.rs:86, packages/d2b-broker/src/protocol.rs:125]
  evidence: targeted grep `vec!\[0_u8; MAX_FRAME_SIZE` =  1 hit (protocol.rs:86; the fd variant passes the same 1 MiB ceiling at 125);`Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` seed =  44 hits; the same 1 MiB per-receive pattern appears at sibling sites d2bd-runtime/src/unix_transport.rs:207,280 and d2b-contracts-broker/src/kernel_client.rs:202 (candidate for cross-crate consolidation); static (unmeasured) - no benchmark exists.



## conc
- clean: seeds `std::thread::|thread::spawn|thread::scope`=9 (all 9 are `std::thread::spawn` sites in `#[cfg(test)]` fake QMP servers), `\bMutex<|\bRwLock<`=0, `Atomic\w+|Ordering::`=0, `thread_local!|unsafe impl (Send|Sync) for`=0; no production threads,locks, or atomics in scope.



## async
- d2b-broker-p7#8 sev=high blast=wide effort=S verdict=actionable - write_redacted_registry_index_at_path resolves the fixed d2bd group via nss `Group::from_name` synchronously on an executor worker on every registry write (enroll/refresh/boot) - fix: resolve the gid once (lazy static or serve-time config injected into the ops context)and return `MediaOpError::Registry` on absence, so the nss lookup leaves the async hot path - [packages/d2b-broker/src/ops/media.rs:2100, packages/d2b-broker/src/ops/media.rs:2126]
  evidence: `Group::from_name` over the assigned files = 1 hit (media.rs:2126; nss lookup is not a clippy::disallowed_method (absent from clippy.toml's disallowed list, so not tracked by the blocking census - actionable per U1 (d) 2/4); static (unmeasured)per-write latency```

## unsafe
- clean: seeds `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern`=0, `// SAFETY:`=0, `transmute|from_raw|MaybeUninit|mem::zeroed`=7 (all 7 are safe std constructors `io::Error::from_raw_os_error` at protocol.rs:383/kernel_ops.rs:2073-2074 and `FileType::from_raw_mode` at media.rs:1798,1838), `unsafe_code`=0; no `unsafe` blocks in scope (consistent with U1 (d) 8's exception set).

## ffi
- N/A (seeds: `extern "C"|no_mangle|unsafe\(link_section`=0, `catch_unwind`=0, `repr\(C\)|repr\(transparent\)`=0, `CStr|CString|c_char`=0 all zero; no foreign-caller boundary in this part)

## macro
- clean: seeds `macro_rules!`=2, `proc_macro|syn::|quote!`=0, `\$crate`=0, `to_compile_error|new_spanned`=0; the two catalog macros (`wire_variants!` at catalog.rs:301,and `audit_fields!` at catalog.rs:373)are deliberate impl-per-enum generators with narrowest fragment specifiers and a documented completeness-gate purpose - not findings.



## test
- clean: seeds `#\[test\]|#\[tokio::test\]`=74 (src) + 58 (tests/), asserts=206 (src) + 219 (tests/), `#\[ignore\]`=0 (src+tests); reviewed suites are behavioral gates (catalog audit gates, QMP fake-server command-sequence tests, state_dir fs/posture regressions, broker protocol fd round-trips, profile/separation/retirement integration matrix) - no cannot-fail tests found.





## Coverage
- idiom: clean (seeds ran: 0/0/14)
- own: 1 finding(s)
- type: 1 finding(s)
- api: 1 finding(s)
- err: clean (seeds ran:  289/28/1/3; sampled: 50 of 321)
- serde: clean (seeds ran:  13/46/0/36)
- obs: clean (seeds ran:  0/1/0/4)
- docs: 3 finding(s)
- perf:  1 finding(s)
- conc: clean (seeds ran:  9/0/0/0)
- async:  1 finding(s)
- unsafe: clean (seeds ran:  0/0/7/0)
- ffi: N/A (seeds:  0/0/0/0 all zero; no foreign-caller boundary in this part)
- macro: clean (seeds ran:  2/0/0/0)
- test: clean (seeds ran: 74+58/206+219/0+0)