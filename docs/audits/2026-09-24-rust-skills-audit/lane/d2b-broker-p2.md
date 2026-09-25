# d2b-broker-p2 - d2b-broker - part 2/7
Baseline: 6ebdd4cec | LOC audited: 11074 (runtime.rs:1-10300; 10300 lines; src/ops/gpu.rs (463); src/ops/modprobe.rs (311); none carries src/generated/**). | modules: runtime (part 1/2 of the item split), ops::gpu, ops::modprobe
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: runtime.rs:1-10300, src/ops/gpu.rs, src/ops/modprobe.rs

## idiom
- d2b-broker-p2#1 sev=low blast=leaf effort=S verdict=actionable - active_locked_usbip_bind_intents builds out with a let-mut push loop over the resolver's intent-id iterator, where a filter_map().collect() pipeline would carry the same filtering - fix: replace the loop at runtime.rs:10132-10147 with `resolver.usbip_bind_intent_ids().filter_map(|id| resolver.find_usbip_bind_intent(id).map)...))).collect::<Vec<_>>()`, keeping the two continue conditions as filter predicates - [packages/d2b-broker/src/runtime.rs:10132]
  evidence: seeds: `for w+ in 0..` = 2;`impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for` = 0;`let mut w+ = (String|Vec)::new()` = 5. The other index-loop hit (runtime.rs:1004) has a per-iteration thread-spawn side effect, so the plain-for rule is the right shape; the while-let async fs-entry loops (5663, 8529, 10113) collect over a fallible async stream, not an iterator-pipeline cleanup.




## own
- clean: seeds ran: 144/320/0/0. clone seed: 144 hits (142 runtime + 2 modprobe). to_owned/to_vec/to_string seed: 320 hits (311 runtime + 2 gpu + 7 modprobe). Rc/RefCell/Arc<Mutex>/Arc<RwLock> seed: 0; Cow seed:  0. Read all 144 clone hits plus a deterministic sample of the to_* hits (every 7th =45 of 320). Every clone/to_owned inspected constructs an owned wire response, audit record, registration, or intent struct whose value must outlive a borrow, or is an accepted Arc at a spawn/static boundary; the two gpu/modprobe to_owned hits are test-fixture argv lists; no explainable-but-avoidable copy found.


## type
- clean: seeds ran:  22/0/0 (`fn validate_w+|fn check_w+` = 22 hits: 18 runtime + 4 gpu;`is_w+: bool|w+_flag: bool` = 0;`(mode|kind|state): String` = 0). The validate fns are wire-admission and-domain-shape gates over contract-pinned &str ids and SpawnRunnerPlan/GpuLaunchRequest values; no boolean-flag soup, Option-pair smell, or string-typed state exists in the three files; a parsed-newtype alternative would be a wire/contract change, not a lane-local refactor.



## api
- d2b-broker-p2#2 sev=medium blast=leaf effort=M verdict=actionable - the GPU and modprobe op modules rise to the crate's public surface through pub mod ops + pub mod gpu/pub mod modprobe (ops/mod.rs:47-48), yet the GPU types (GpuBrokerRole, GpuDeviceClass, GpuOpaqueIdentity, GpuLaunchRequest, GpuProcessObservation, GpuBrokerError)and modprobe's(ModprobeAuditRecord, ModprobeDecision, AllowlistRow, ModprobeBackend, RecordingBackend, dispatch, live_modprobe_if_allowed)have zero consumers outside d2b-broker itself:live_handlers uses only the two gpu validate fns and runtime uses only live_modprobe_if_allowed, all same-crate - fix: narrow the module decls to pub(crate)(ops/mod.rs:47-48), or make the item-level pub to pub(crate)in gpu.rs and modprobe.rs; same-crate call sites are unaffected - [packages/d2b-broker/src/ops/gpu.rs:13, packages/d2b-broker/src/ops/gpu.rs:24, packages/d2b-broker/src/ops/gpu.rs:41, packages/d2b-broker/src/ops/gpu.rs:63, packages/d2b-broker/src/ops/gpu.rs:177, packages/d2b-broker/src/ops/gpu.rs:190, packages/d2b-broker/src/ops/modprobe.rs:32, packages/d2b-broker/src/ops/modprobe.rs:42, packages/d2b-broker/src/ops/modprobe.rs:61, packages/d2b-broker/src/ops/modprobe.rs:67, packages/d2b-broker/src/ops/modprobe.rs:76, packages/d2b-broker/src/ops/modprobe.rs:99, packages/d2b-broker/src/ops/modprobe.rs:165, packages/d2b-broker/src/ops/mod.rs:47, packages/d2b-broker/src/ops/mod.rs:48]
  evidence: census:`GpuLaunchRequest|GpuBrokerError|GpuOpaqueIdentity|GpuDeviceClass|GpuBrokerRole|GpuProcessObservation|ModprobeBackend|ModprobeAuditRecord|ModprobeDecision|AllowlistRow|RecordingBackend|ops::gpu|ops::modprobe` over packages, tests, nixos-modules, docs/reference, labs:code hits outside the defining files = 3 (live_handlers.rs:2903, live_handlers.rs:2957, runtime.rs:3722), all inside the same crate; cross-crate code users = 0; d2b-core's/d2b-host's/docs-references are prose only.



## err
- d2b-broker-p2#3 sev=high blast=wide effort=S verdict=actionable - DispatchAuditContext::from_request panics the broker on a malformed authoritative audit join: both CanonicalAuditDigest::parse(zone_id.expect)...) at runtime.rs:2419-2422 parse data that came straight out the wire (request.authoritative_audit_join() returns the strings unchecked), while the sibling from_request_with_join (runtime.rs:2440-2444) converts the same parse failure to BrokerError::Protocol - a remote caller can crash the daemon - fix: replace both expect("authoritative ... digest") calls with `.map_err(|_| BrokerError::Protocol("audit zone identity invalid".to_owned()))?;`, mirroring runtime.rs:2442-2444, keeping the panic out of the wire path - [packages/d2b-broker/src/runtime.rs:2419, packages/d2b-broker/src/runtime.rs:2422]
  evidence: seed `\.unwrap\(\)|\.expect\(` = 13 over the lane (11 runtime + 2 gpu; both gpu hits are in mod tests), of which 2419/2422 are the only inputs crossing a caller-controlled boundary; the other unwrap/expect hits are startup/runtime-build/catalog invariants (runtime.rs:1023, 3169, 5112-5118, 7073, 7093, 7173) or best-effort ignores (`let _ =` at runtime.rs:1038, 1200,1421,1461,1579,1613,1641,6432-6434)and the 16 `let _ = |\.ok\(\);` seed hits are all deliberate best-effort audit/cleanup/param sites; gpu's unreachable! at gpu.rs:321 is a closed-enum invariant panic, correct per the panic policy;`enum w*Error` = 3 (RunError, GpuBrokerError; BrokerError is pub(crate)


## serde
- clean: seeds ran: 3/2/0/8. The modprobe wire shapes (ModprobeAuditRecord, ModprobeDecision, AllowlistRow)use serde(rename_all = "camelCase") or "kebab-case" on the type, not per field; no hand-written Deserialize impl exists; the 8 serde_json::from_/to_ sites either surface errors as typed BrokerErrors (runtime.rs:1481, 2551, 4976, 9752)or are best-effort optional audit fields that explicitly fall back (to_value().ok() at runtime.rs:3334, 3364, 3725; from_slice::<Value>().ok() at 3563 for a qemu dump probe), none silently swallowing wire input the caller must know about.



## obs
- d2b-broker-p2#4 sev=low blast=leaf effort=M verdict=actionable - three message-only tracing events carry no named fields:the child-reap buffer-busy warnings at runtime.rs:6023 and runtime.rs:6036 (a dropped notification / an abandoned drain), and the dispatch-pool panic error at runtime.rs:1016 (request body panicked), each has dynamic identity it could expose (the dropped ChildReapedNotification, or which operation the panicking job covered)- fix: convert to events with named fields:tracing::warn!(dropped = ?notif, "child_reap_buffer busy"), include buffer = "child_reap_buffer" on the empty-drain warn,and propagate an operation span or captured job context into the pool's tracing::error! - [packages/d2b-broker/src/runtime.rs:6023, packages/d2b-broker/src/runtime.rs:6036, packages/d2b-broker/src/runtime.rs:1016]
  evidence: seed `(info|debug|warn|error|trace)!\("` = 2 hits (runtime.rs:6023, 6036); the no-fields multiline tracing::error! at runtime.rs:1016 sits inside the tracing::|log:: count = 33; all other tracing events in the lane carry named fields(error, notify_result, load_outcome, runner_id, etc.); the `//!`-documented use tracing::info/warn at 45-46 undermines neither finding.



## docs
- d2b-broker-p2#5 sev=medium blast=family effort=S verdict=actionable - the broker's central runtime module has no //! module doc,and its five process-entry public items (ServerConfig, BrokerMode, RunError, parse_command, run)lack any doc comment, even though they form the public API the composition binary (d2b-broker-composition/src/main.rs:3, 47-60)and integration tests (tests/profile_separation.rs:1, 22-33)match on - fix: add a one-line module doc atop runtime.rs(serve/socket/audit contract)and one-line first-sentence doc comments to ServerConfig, BrokerMode, RunError, with # Errors on run/parse_command, which return Result),keeping the comments caller-contracts, not implementation narration - [packages/d2b-broker/src/runtime.rs:1, packages/d2b-broker/src/runtime.rs:324, packages/d2b-broker/src/runtime.rs:375, packages/d2b-broker/src/runtime.rs:398, packages/d2b-broker/src/runtime.rs:565, packages/d2b-broker/src/runtime.rs:799]
  evidence: docs seeds:`^\s*pub (fn|struct|enum|trait|const|type)` = 34 (10 runtime + 19 gpu + 5 modprobe), documentation state checked per hit;`/// # (Examples|Errors|Panics|Safety)` = 0;`-> Result<` = 104. The gpu/modprobe public items largely do carry docs; the runtime entry set listed is the documented-by-fields-only surface (ServerConfig's fields each have ///,but the struct/enum and the four entry fns none)



## perf
- clean: seeds ran: 124/33/91, plus gpu/modprobe 3 sites (2/0/1)=total 251 hits. Sampled: read 42 of 251 hits (every 6th). Every sampled format!/to_string/Vec::new site is an error path (BrokerError::LiveHandler(format!)...)), an audit-record/rendering field (requested:/resolved: rows, display().to_string(), serde_json fallbacks), a one-shot process/startup diagnostic (sd_notify msg, nft table render),or an empty-case/common struct allocation(Vec::new(), HashMap::new()in registries, response_fds)-none sits in a per-request hot loop exceeding a single small allocation; no perf-budget claim is made (static, unmeasured.



## conc
- clean: seeds ran: 2/12/3/0. The two std::thread hits (runtime.rs:1008, 1069)are the sanctioned dedicated bounded-worker DispatchPool (threads spawned once at startup, blocking_recv on the worker's own thread per plan R4)and available_parallelism() sizing; the 12 Mutex< hits are all tokio::sync::Mutex registries/limiters with the documented try_lock-on-sync-worker posture(`// Non-blocking try-lock (plan U8)` at 7540/7556/6022/6035,orthe rate-limiter lock_sync comment at 1663-1666); the 3 Atomic/Ordering hitsform one AtomicUsize round-robin counter with Ordering::Relaxed(a counter nobody synchronises on),the weakest correct ordering; no thread_local!,no unsafe impl Send/Sync.



## async
- d2b-broker-p2#6 sev=medium blast=wide effort=S verdict=policy-confirmed - the USB-audit serial HMAC key path runs blocking filesystem syscalls inside async fns on broker executor threads:usb_audit_serial_hmac_keyring calls the sync ensure_usb_audit_serial_hmac_key_dir (two path_safe::ensure_dir stat/mkdir chains)per call,and every bind op with a device serial loads each key file through a sync nix::fcntl::open plus tokio::fs::File::from_std read,and the create leg performs sync create_file_at_safe/fchmod/rustix::fs::fsync(dir_fd) at runtime.rs:7722-7729; none of these raw calls sits on the disallowed-methods list,so the sync-in-async class escapes the existing gate - fix: extend the already-used tokio::fs::File::from_std)..).sync_all().await pattern(orthe bounded-worker shape per plan R4)to the dir-fd fsync and the key-dir ensure/open legs, per U1 ledger 2,which names tokio::fs asthe sanctioned replacement for these blocking calls,so the verdict is policy-confirmed - [packages/d2b-broker/src/runtime.rs:7729, packages/d2b-broker/src/runtime.rs:7654, packages/d2b-broker/src/runtime.rs:7584, packages/d2b-broker/src/runtime.rs:7696, packages/d2b-broker/src/runtime.rs:7722]
  evidence: async seeds:`async fn|async move|\.await` = 286 (runtime 267 + modprobe 19);`tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(` = 1 (tokio::spawn at runtime.rs:1398,a spawn-bound connection task);`tokio::sync::(Mutex|RwLock|Notify)` = 20,all with the documented plan-U8 try_lock/Notify usage;`#\[tokio::(main|test)\]|Runtime::block_on` = 12 (gpu/modprobe test fns; runtime's Runtime::block_on sites at 1221, 1528, 1541 carry inline allows with sanctioned reasons, not re-flagged



## unsafe
- clean: seeds ran: 0/0/5/0 (`unsafe \{|unsafe fn|unsafe impl|unsafe extern` = 0;`// SAFETY:` = 0;`transmute|from_raw|MaybeUninit|mem::zeroed` = 5;`unsafe_code` = 0). The five from_raw-pattern hits (runtime.rs:31, 5141, 6045, 7657, 7729)all name crate::sys::owned_fd_from_raw (the sanctioned syscall-boundary helper in ledger (d) 8)or io::Error::from_raw_os_error (safe std); no unsafe block, fn, impl, or extern exists anywhere inthe three files,consistent with the enumerated exception set (p5's sys.rs/p4's disk_init.rs hold the crate's unsafe sites, not here).



## ffi
- clean: seeds ran: 0/1/0/0. The single catch_unwind hit (runtime.rs:1015)isthe dispatch-pool's process-supervision panic boundary, not an FFI surface:each pool job is caught so a panicking handler costs its own connection andthe worker keeps serving (documented at runtime.rs:1012-1018),the card's "is it a boundary?" question resolves to yes for that purpose; no extern "C", no repr(C)/transparent, no CStr/c_char in the lane.



## macro
- clean: seeds ran: 2/0/0/0. The two macro_rules! definitions (runtime.rs:3087, 3092)are write_decision_op_record! and write_success_op_record!,variadic arg-forwarders that append the contextual audit_context tothe impl fns for dozens of callsites; the genuine variadic-interface case the skill names; the $($args:tt)* fragment isthe narrowest that can forward arbitrary trailing argument lists tothe *_impl fns; no proc-macro, no $crate, no to_compile_error/new_spanned machinery exists inthe lane.



## test
- clean: seeds ran: 67/240/0/0 (`#\[test\]|#\[tokio::test\]` = 67:gpu 3 + modprobe 6 + tests 58; runtime's 1 hit is a comment mention;`assert_eq!\(|assert_ne!\(|assert!\(` = 240:runtime/gpu/modprobe 1/8/12 + tests 219;`proptest!|insta::assert|rstest` = 0;`#\[ignore\]` = 0). Sampled: read 44 of 219 assert rows in tests/** (every 5th) plus both in-file test modules in full. The gpu/modprobe tests assert on error variants (GpuBrokerError::WrongPrincipal,ModprobeDecision::*)and recorded backend effects, not Display strings/implementation; the sampled tests/** asserts target wire payloads (json["kind"], PROTOCOL_VERSION, retired-variant lookup),cross-process state(host.pid() != guest.pid(),reconciler taps/pidfds),and error kinds(STALE_WIRE_VERSION,error_kind,w3-pending-typed-wire)-behavior-level assertions,mostly with failure-message context where tables loop; no test-that-cannot-fail observed.



## Coverage
- idiom: 1 finding(s)
- own: clean (seeds ran: 144/320/0/0; sampled: all 144 clone hits + 45 of 320 to_* hits)
- type: clean (seeds ran: 22/0/0)
- api: 1 finding(s)
- err: 1 finding(s)
- serde: clean(seeds ran: 3/2/0/8)
- obs: 1 finding(s)
- docs: 1 finding(s)
- perf: clean(seeds ran: 124/33/91; sampled: read 42 of 251 hits)
- conc: clean(seeds ran: 2/12/3/0)
- async: 1 finding(s)
- unsafe: clean(seeds ran: 0/0/5/0)
- ffi: clean(seeds ran: 0/1/0/0)
- macro: clean(seeds ran: 2/0/0/0)
- test: clean(seeds ran: 67/240/0/0; sampled: read 44 of 219 tests/** asserts)
