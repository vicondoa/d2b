# d2b-session-unix - d2b-session-unix
Baseline: 6ebdd4cec | LOC audited: 6663 (excl. src/generated/**) | modules: whole crate
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none (single-part lane)

## idiom
- clean: seeds ran (3/1/8): the index loops at socket.rs:561 (fairness budget) and systemd.rs:176/194 (index needed for `take_custom`/`take_raw_fd`) are legitimate; the hand-written `Clone`/`Default`/`From` impls in subject.rs:61-80 are cfg-gated capability-mutation machinery whose `unreachable!()` bodies are the point (declared in the manifest check-cfg list); the `Vec::new()` accumulation loops are statement-style with per-item error handling and multiple outputs.

## own
- clean: seeds ran (14/9/14/0): every clone is explainable - Arc clones at split/spawn boundaries (adapter.rs:523/524/627, credit.rs:69/88), cache stores (adapter.rs:234), owned returns (adapter.rs:209/281), CreditBundle copies for split reader/writer (adapter.rs:511/622), test fixtures (adapter.rs:1188, vsock.rs:547/553); the `same_descriptor_binding` clone (adapter.rs:997) is the chosen normalization for a two-field compare and the field-wise alternative is worse; `to_owned`/`to_string` hits are env-string conversions and test payloads; no `Rc`/`RefCell`/`Arc<Mutex>`/`Cow` in production.

## type
- clean: seeds ran (8/0/0): all `validate_*` hits are boundary parsers the skill endorses - `validate_socket` at construction (socket.rs:637), `validate_descriptor`/`validate_value` as the once-per-attachment admission with a cached result (adapter.rs:258/942), `validate_owned_file*` on received fds (descriptor.rs:569/577), `validate_environment_values` at the env boundary (systemd.rs:201); `VerifiedUnixPeer::validate_transport` (subject.rs:126) is a cheap cross-transport guard on a value that crosses into d2b-session; no boolean-flag soup or stringly-typed state (seeds 2/3 zero).

## api
- d2b-session-unix#1 sev=medium blast=family effort=S verdict=actionable - `SentPacket::acknowledge(self) {}` is a public no-op whose name promises an acknowledgment; its only behavior is dropping the packet (releasing the credit bundle via `Drop`), which callers cannot tell from the signature - fix: remove the method and let callers drop the packet, or document the drop-semantics contract on the method - [packages/d2b-session-unix/src/socket.rs:176]
  evidence: seed `\bpub (fn|struct|enum|trait|type|const|mod)` = 140 hits; census: `\.acknowledge\(\)` over packages/ = 6 hits (adapter.rs:595, d2b-provider-guest-cloud-hypervisor/src/controller_session.rs:145, d2b-provider-test-controller/src/main.rs:208, d2b-provider-toolkit/src/base/fd10.rs:1001/1725, tests/unix_session.rs:819)
- clean: the rest of the surface is deliberate - `pub use` re-export arms in lib.rs (house single-surface pattern), `Arc<dyn Fn>` callback aliases and `with_observer(Arc<dyn UnixTransportObserver>)` genuinely shared across split transports (adapter.rs:432/718, cited call sites adapter.rs:627), private fields on all transport/socket types, `UnixTransportObserver` with one required method plus a defaulted `record_errno`.

## err
- clean: seeds ran (22/5/3/4): production `expect` sites are invariants on internal state (`outbound was initialized`, adapter.rs:888, vsock.rs:158/272) or literally-built values (`compiled bootstrap Provider ref is valid`, zone_admission.rs:35 - the recorded false-positive class); `let _ =` sites are deliberate best-effort closes during shutdown/drain (adapter.rs:900, systemd.rs:196); `unreachable!()` in subject.rs is cfg-gated capability-mutation machinery; the four error enums are family-split with stable kebab-case wire renderings and caller-action mapping fns (`map_transport_error`, `map_validation_error`, `unix_failure_reason`).

## serde
- N/A: seeds 0/0/0/0 all zero; the crate crosses no wire - no serde dependency in the manifest and no `Deserialize`/`serde_json` anywhere in src.

## obs
- clean: seeds ran (0/0/0/2): zero `println!`/`eprintln!` in the library; the only two `tracing` events (zone_admission.rs:80/89) are `warn!` with named fields (`provider`, `expected_uid`, `observed_uid`) and no interpolation; no secrets in fields (identity types carry redacting `Debug` impls, e.g. subject.rs:135, pidfd.rs:31); no spans, which is not required at this granularity.

## docs
- d2b-session-unix#2 sev=medium blast=leaf effort=M verdict=actionable - the exported surface is largely undocumented: the transport, credit, descriptor, pidfd, systemd and vsock types and most of their pub methods carry no doc comment (only `VerifiedUnixPeer`, `ZoneBootstrapIdentity` and a handful of methods do) - fix: add first-sentence contract docs per pub item, starting with `SeqpacketSocket`, `UnixSeqpacketTransport`/`UnixStreamTransport`, `CreditPool`, `PidfdEvidence`, `PeerCredentials`, `ActivatedSeqpacketListener`, `FramedVsockTransport` - [packages/d2b-session-unix/src/socket.rs:190, packages/d2b-session-unix/src/adapter.rs:351, packages/d2b-session-unix/src/credit.rs:22, packages/d2b-session-unix/src/pidfd.rs:9, packages/d2b-session-unix/src/descriptor.rs:23, packages/d2b-session-unix/src/systemd.rs:44, packages/d2b-session-unix/src/vsock.rs:22]
  evidence: docs seed 1 (`^\s*pub (fn|struct|enum|trait|const|type)`) = 140 hits; seed 2 (`/// # (Examples|Errors|Panics|Safety)`) = 0 hits; `missing_docs` is not enabled - this is a proposal, not a gate failure
- d2b-session-unix#3 sev=low blast=leaf effort=S verdict=actionable - zero canonical `# Errors` sections exist despite roughly 60 pub `Result`-returning fns whose failure conditions are non-obvious (e.g. `SeqpacketSocket::from_owned` vs `from_parent_prearmed` vs `from_inherited_fd` fail differently) - fix: add `# Errors` sections naming the failing conditions to the pub `Result` fns - [packages/d2b-session-unix/src/socket.rs:202, packages/d2b-session-unix/src/socket.rs:210, packages/d2b-session-unix/src/socket.rs:222, packages/d2b-session-unix/src/adapter.rs:378]
  evidence: docs seed 2 = 0 hits vs seed 3 (`-> Result<`) = 60 hits

## perf
- d2b-session-unix#4 sev=low blast=leaf effort=S verdict=actionable - burst and collector `Vec`s grow from empty with an exact known upper bound, reallocating on the way - fix: `Vec::with_capacity(fairness_budget)` in `recv_burst`/`send_burst` and `Vec::with_capacity(attachments.len())` in `send_packet` - [packages/d2b-session-unix/src/socket.rs:256, packages/d2b-session-unix/src/socket.rs:306, packages/d2b-session-unix/src/adapter.rs:540]
  evidence: static (unmeasured); seed `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 15 hits; the remaining hits are empty-case defaults (receive_body, outbound None) or test fixtures
- clean: `format!` sites are cold (pidfd.rs:65 /proc path, zone_admission.rs:34 compiled ref) or tests; `to_string` sites are env-string conversions on cold paths; no hashing with attacker-controlled keys; no codegen-flag advice warranted.

## conc
- clean: seeds ran (0/3/20/0): the two production `std::sync::Mutex` sites (adapter.rs:158 `validation`, adapter.rs:297 `ReceivedPacketState.inner`) carry `#[allow(clippy::disallowed_methods, reason = "synchronous path")]` - sanctioned synchronous paths per the provider-crate-policy allow list, short critical sections with no suspension point; atomics are counters/flags with correct weakest orderings (CreditPool CAS AcqRel, `received_any` swap, `ACTIVATION_CONSUMED` compare_exchange); no threads, no `thread_local!`, no manual `Send`/`Sync`.

## async
- clean: seeds ran (130/2/1/9): all socket I/O goes through the `AsyncFd` wrappers named in clippy.toml's replacement vocabulary (`SeqpacketSocket::recv_burst`/`send_burst`, `StreamSocket::read_available`/`write_all`, systemd accept loop); no blocking calls on executor workers (the only std fs call is the sanctioned sync pidfd surface with an allow, pidfd.rs:71); no guards held across `.await` (`await_holding_lock` deny is clean); framing is cancellation-safe via persistent buffers + `mem::take` (adapter.rs:813, vsock.rs:127) with dedicated cancellation tests; `tokio::spawn`/`join!` appear only in tests.

## unsafe
- N/A: seeds 1-3 all zero (`\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` = 0, `// SAFETY:` = 0, `transmute|from_raw|MaybeUninit|mem::zeroed` = 0 - the `from_raw` matches are `FileType::from_raw_mode`/`Pid::from_raw`/`io::Error::from_raw_os_error`, not pointer casts); manifest declares `unsafe_code = "forbid"`; the crate is not on the enumerated exception set in U1 (d) 8, consistent with zero blocks.

## ffi
- N/A: seeds 0/0/0/0 all zero (`extern "C"`, `no_mangle`, `catch_unwind`, `repr(C)`, `CStr`/`CString`/`c_char` absent); the crate has no C boundary - libc/rustix calls are in-crate syscall wrappers.

## macro
- clean: seed 1 = 1 hit (`macro_rules!` at subject.rs:59, the cfg-gated `mutate_verified_unix_peer_trait!` capability-mutation machinery whose generated impls must never be reachable - declared in the manifest check-cfg list, the test-only-helper-macro false-positive class); seeds 2-4 zero; no proc-macro or `$crate` usage.

## test
- clean: seeds ran (32/75/0/0): the suite is behavior-asserting and deterministic - real kernel objects (pipes, pidfds, socketpairs), fd tests serialized through a global `LazyLock<Mutex<()>>` (unix_session.rs:21), error variants asserted rather than Display strings, adversarial sequences (recycled pidfd info, fd reuse, stale readiness, cancellation mid-frame), credit accounting verified at every scope, and `#[tokio::test(flavor = "current_thread")]` where fd state is shared; no `proptest`/`insta`/`rstest` and no `#[ignore]` (seeds 3/4 zero), which is appropriate for this kernel-surface suite.

## Coverage
- idiom: clean (seeds ran: 3/1/8)
- own: clean (seeds ran: 14/9/14/0)
- type: clean (seeds ran: 8/0/0)
- api: 1 finding(s)
- err: clean (seeds ran: 22/5/3/4)
- serde: N/A (seeds: 0/0/0/0 all zero; no serde dependency, crate crosses no wire)
- obs: clean (seeds ran: 0/0/0/2)
- docs: 2 finding(s)
- perf: 1 finding(s)
- conc: clean (seeds ran: 0/3/20/0)
- async: clean (seeds ran: 130/2/1/9)
- unsafe: N/A (seeds: 0/0/0/0 for seeds 1-3; manifest `unsafe_code = "forbid"`)
- ffi: N/A (seeds: 0/0/0/0 all zero; no C boundary)
- macro: clean (seeds ran: 1/0/0/0; cfg-gated test-only macro)
- test: clean (seeds ran: 32/75/0/0)