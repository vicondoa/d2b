# d2b-session-p2 - d2b-session - part 2/2
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 5364 (excl. src/generated/**) | modules: admission.rs, engine.rs, error.rs, client.rs, transport.rs, lifecycle.rs, record.rs, bootstrap.rs, deadline.rs, lib.rs
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: part 2/2 per U1 (f); test lens adds tests/**

## idiom
- d2b-session-p2#1 sev=low blast=leaf effort=S verdict=actionable - decode_attachment_control walks the descriptor table with an index loop plus manual offset arithmetic where an iterator pipeline fits - fix: replace the `for _ in 0..count` loop with `bytes[3..].chunks_exact(ATTACHMENT_DESCRIPTOR_BYTES).take(usize::from(count)).map(decode_attachment_descriptor).collect::<Result<Vec<_>, _>>()` - [engine.rs:1802, engine.rs:1803, engine.rs:1807]
  evidence: seed `for \w+ in 0\.\.` = 1 hit; the loop at engine.rs:1803 is the only index loop in the part
- d2b-session-p2#2 sev=low blast=leaf effort=S verdict=actionable - send_authorized_ttrpc re-implements the exact verb allow-list check that validate_ttrpc_permit already encodes - fix: call `validate_ttrpc_permit(&permit, now_tick)?` instead of re-writing the matches! block - [admission.rs:1593, admission.rs:956, admission.rs:958]
  evidence: seed `fn validate_\w+` = 7 hits; the Invoke|AuditExport|SupportBundle matches! block appears verbatim at admission.rs:958-962 and admission.rs:1597-1601

## own
- d2b-session-p2#3 sev=low blast=family effort=S verdict=actionable - take_authentication and from_verified_adapter clone the whole EndpointPolicy just to build a comparison HandshakeOffer - fix: add `impl From<&EndpointPolicy> for HandshakeOffer` in d2b-contracts-zone-session and call HandshakeOffer::from(policy) - [engine.rs:689, admission.rs:593]
  evidence: seed `\.clone\(\)` = 51 hits; both sites are one-shot admission-path clones of a struct holding LimitProfile, TransportBinding, and AttachmentPolicy; all other clones in the part are explainable (spawn boundaries, snapshot copies, Arc clones)

## type
- d2b-session-p2#4 sev=low blast=leaf effort=S verdict=actionable - stream control is decoded as a raw u8 kind inside a (u8, StreamId, u32) tuple and matched against local constants, so an unknown tag stays representable until the runtime match - fix: introduce a closed StreamControlKind enum with tag()/from_tag() beside the existing AttachmentControl enum - [engine.rs:1702, engine.rs:1295, engine.rs:31]
  evidence: seed `fn validate_\w+|fn check_\w+` = 7 hits; STREAM_CLOSE/STREAM_CREDIT/STREAM_RESET constants at engine.rs:31-33 are matched in receive_stream_control at engine.rs:1297-1308; the validate_* hits are boundary checks on wire-derived values, not repeated validation

## api
- d2b-session-p2#5 sev=low blast=leaf effort=M verdict=actionable - SessionEngine exposes seven establishment constructors, the metrics-taking variants have no production callers, and a public with_metrics builder already exists - fix: drop establish_initiator_with_generation_discovery_and_metrics and establish_responder_with_metrics, keep one metrics-taking path per role, and give establish_responder_with_generation_floor a metrics twin instead of recording into a fresh NoopMetrics - [engine.rs:166, engine.rs:262, engine.rs:416, engine.rs:356, engine.rs:584]
  evidence: census: `establish_initiator_with_metrics` over packages/tests/labs = 1 hit (tests/component_session.rs:1243); `establish_responder_with_metrics` = 0 external hits; `establish_initiator_with_generation_discovery_and_metrics` = 0 external hits; seed `\bpub (fn|struct|enum|trait|type|const|mod) ` = 155 hits; the generation-floor variant records with Arc::new(NoopMetrics) at engine.rs:406

## err
- d2b-session-p2#6 sev=medium blast=leaf effort=S verdict=actionable - OwnedTransportHandle panics via expect on descriptor, into_owned_transport, and close after the handle is consumed, because the Option<Box<dyn OwnedTransport>> keeps the consumed state representable - fix: return Option or Result from into_owned_transport and close, or split the handle into a typestate so double-consume does not compile - [transport.rs:170, transport.rs:178, transport.rs:185, transport.rs:173]
  evidence: seed `\.unwrap\(\)|\.expect\(` = 23 hits; the three expects at transport.rs:173,181,188 are the only non-test panic sites on a public API in the part (the other hits are cfg(test-support) helpers and test code)
- d2b-session-p2#7 sev=medium blast=leaf effort=S verdict=actionable - SessionClientBridgeError's Display prints a fixed label and the Error impl has no source(), so the inner SessionError code is lost from the chain when the bridge logs it - fix: implement Error::source() returning Some(&SessionError) for the Session variant, and/or include the code in Display - [client.rs:216, client.rs:224, client.rs:237]
  evidence: seed `enum \w*Error` = 3 hits; bridge termination logs at client.rs:60-70 print only the fixed label via %error, so an operator cannot see the underlying SessionErrorCode

## serde
- N/A: seeds `derive(...Serialize|Deserialize)` / `serde(...)` / `impl .*Deserialize` / `serde_json::from_|to_` all 0 hits; this part crosses no serde boundary (crate-level matrix cell is 0)

## obs
- clean: seeds ran: 0/0/0/12 - no println/eprintln, no interpolated message-only events, no instrument spans; all tracing::warn!/debug! calls use named fields (error = %error, purpose, service, timeout_ms, minimum_generation), and the redacting Debug impls keep secrets out of fields

## docs
- d2b-session-p2#8 sev=medium blast=leaf effort=M verdict=actionable - SessionEngine and SessionEvent plus 24 of the engine's pub methods carry no doc comments, leaving the crate's central data plane undocumented - fix: add one-line docs, with # Errors on the Result-returning methods and # Panics where applicable - [engine.rs:38, engine.rs:84, engine.rs:584, engine.rs:677, engine.rs:681, engine.rs:702, engine.rs:722, engine.rs:737, engine.rs:763, engine.rs:772, engine.rs:776, engine.rs:780, engine.rs:784, engine.rs:788, engine.rs:803, engine.rs:812, engine.rs:941, engine.rs:956, engine.rs:971, engine.rs:1037, engine.rs:1056, engine.rs:1448]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 155 hits; awk scan of engine.rs counts 24 pub methods with no preceding /// (missing_docs is not enabled; this is a proposal only)
- d2b-session-p2#9 sev=medium blast=leaf effort=S verdict=actionable - the whole public surface of lifecycle.rs, record.rs, bootstrap.rs, and deadline.rs is undocumented, including non-obvious state machines such as SessionLifecycle::poll_keepalive and begin_reconnect - fix: add item docs with # Errors sections on the Result-returning methods - [lifecycle.rs:38, lifecycle.rs:81, lifecycle.rs:147, record.rs:62, record.rs:107, bootstrap.rs:87, deadline.rs:14]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 155 hits; awk scan counts 10 undocumented pub methods in lifecycle.rs, 5 in record.rs, 5 in bootstrap.rs, 5 in deadline.rs
- d2b-session-p2#10 sev=low blast=leaf effort=S verdict=actionable - the route-binding accessors on AuthenticatedSessionRouteBinding, the SessionError accessors, and the TransportPacket methods are undocumented while their siblings carry docs - fix: add one-line docs to the accessors at the anchors - [admission.rs:1182, admission.rs:1186, admission.rs:1190, admission.rs:1194, admission.rs:1198, admission.rs:1237, admission.rs:1241, admission.rs:1245, admission.rs:1249, admission.rs:1253, admission.rs:1257, error.rs:31, error.rs:47, error.rs:51, error.rs:55, error.rs:116, transport.rs:23, transport.rs:30, transport.rs:34, transport.rs:38, transport.rs:42]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 155 hits; awk scan confirms 11 admission.rs, 5 error.rs, and 5 transport.rs pub items without a preceding ///
- d2b-session-p2#11 sev=low blast=leaf effort=S verdict=actionable - serialized_transport_split's doc claims it exists for engine-only test transports, but production code in d2b-bus and the crate's own driver call it - fix: rewrite the doc to state the serialized-compatibility contract (halves must never be driven concurrently) - [transport.rs:208, transport.rs:212]
  evidence: census: `serialized_transport_split` over packages = 6 hits, including d2b-bus/src/session/zone_link.rs:634 and d2b-session/src/driver.rs:456

## perf
- d2b-session-p2#12 sev=low blast=leaf effort=S verdict=actionable - unprotect allocates a fresh plaintext buffer sized to the limit and copies the payload out with to_vec on every received record - fix: keep a reusable scratch buffer on RecordProtector and return plaintext.split_off(RECORD_HEADER_LEN) instead of payload.to_vec() - [record.rs:125, record.rs:147]
  evidence: static (unmeasured); seed `Vec::new\(\)` = 11 hits; unprotect runs once per protected record on the receive hot path
- d2b-session-p2#13 sev=low blast=leaf effort=S verdict=actionable - flush copies every dequeued logical frame with as_bytes().to_vec() because OutboundFrame only exposes a borrowed view - fix: add OutboundFrame::into_bytes() in scheduler.rs and consume it in flush - [engine.rs:1354, engine.rs:1362, scheduler.rs:72]
  evidence: static (unmeasured); seed `Vec::new\(\)` = 11 hits; one copy per logical frame on the send path before fragmentation and encryption
- d2b-session-p2#14 sev=low blast=leaf effort=S verdict=actionable - the replay cache is a VecDeque scanned linearly with contains() on every received record - fix: use a bounded HashSet<[u8; 32]> or document why the 1024-entry linear scan is acceptable - [record.rs:120, record.rs:146]
  evidence: static (unmeasured); REPLAY_CACHE_ENTRIES = 1024 at record.rs:12; contains() runs per record before sequence acceptance

## conc
- d2b-session-p2#15 sev=low blast=leaf effort=S verdict=actionable - AuthenticatedSessionDriver._owner is a std::sync::Mutex that is never locked, serving only as a Sync carrier for the ComponentSessionDriver: Send + Sync bound, with no comment saying so - fix: document the Sync-carrier intent on the field or replace it with a named wrapper type - [admission.rs:756, admission.rs:1666, driver.rs:33]
  evidence: seed `\bMutex<` = 3 hits; grep `_owner` over admission.rs = 2 hits (declaration and construction), no lock() or get_mut() call anywhere; the SessionLiveness AtomicBool uses the correct Acquire/Release pair and the transport split uses tokio::sync::Mutex

## async
- clean: seeds ran: 201/1/1/3 - all handshake and packet-send I/O is wrapped in tokio::time::timeout, the serialized split uses tokio::sync::Mutex (no std guard across awaits), the client bridge uses tokio::spawn plus select! with cancellation, and no blocking call or lock guard crosses an await point in the part

## unsafe
- N/A: seeds 1-3 all zero (no unsafe blocks, fns, impls, transmute, from_raw, MaybeUninit, or mem::zeroed); seed 4 = 1 hit, the `#![forbid(unsafe_code)]` attribute at lib.rs:6, which alone does not make the lens applicable

## ffi
- N/A: seeds `extern "C"|no_mangle|unsafe\(link_section` / `catch_unwind` / `repr\(C\)|repr\(transparent\)` / `CStr|CString|c_char` all 0 hits; no FFI surface in the part

## macro
- d2b-session-p2#16 sev=low blast=leaf effort=S verdict=actionable - the local admit_try! macro exists only to fuse an early return with a metric record, which a plain helper function plus ? expresses - fix: replace each invocation with `let result = <expr>; admit_or_record(&mut engine, result)?` where admit_or_record records the failure metric and returns the error - [admission.rs:625, admission.rs:641, admission.rs:642, admission.rs:643, admission.rs:648, admission.rs:661]
  evidence: seed `macro_rules!` = 3 hits; the macro is invoked 5 times inside admit; the other two macro_rules! sites (mutate_session_acceptor_trait!, mutate_authenticated_session_trait!) are cfg-gated impl-generation scaffolding for compile-fail verification and are justified

## test
- d2b-session-p2#17 sev=low blast=leaf effort=S verdict=actionable - unpolled_cancellation_on_real_driver_reclaims_request_for_reuse spins up to 64 yield_now iterations waiting for the cancellation task to reclaim the request, while its sibling test waits on a Notify - fix: wait on a Notify (or a tokio::time::timeout around a Notify) instead of the fixed spin cap - [tests/admission.rs:78, tests/admission.rs:96, tests/admission.rs:139]
  evidence: seed `#\[test\]|#\[tokio::test\]` = 5 hits in src plus 40 test fns across tests/; the sibling failed_cancellation_delivery test uses send_failure.entered.notified() with a 1s timeout at tests/admission.rs:139-142; no proptest/insta/rstest and no #[ignore] anywhere

## Coverage
- idiom: 2 finding(s)
- own: 1 finding(s)
- type: 1 finding(s)
- api: 1 finding(s)
- err: 2 finding(s)
- serde: N/A (seeds: 0/0/0/0 all zero; no serde derives, attributes, hand-written Deserialize, or json crossing in this part)
- obs: clean (seeds ran: 0/0/0/12; no println, no interpolated message-only events, all events carry named fields)
- docs: 4 finding(s)
- perf: 3 finding(s)
- conc: 1 finding(s)
- async: clean (seeds ran: 201/1/1/3; timeouts wrap handshake and send I/O, tokio Mutex for the serialized split, no blocking calls or guards across awaits)
- unsafe: N/A (seeds: 0/0/0/1 all zero except the forbid attribute at lib.rs:6; no unsafe code in the part)
- ffi: N/A (seeds: 0/0/0/0 all zero; no FFI surface)
- macro: 1 finding(s)
- test: 1 finding(s)