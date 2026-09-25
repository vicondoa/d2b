# d2b-provider-observability-otel - d2b-provider-observability-otel
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 3766 (excl. src/generated/**) | modules: whole crate (agent, config, controller, emitter_socket, ingress_policy, lib, metric_policy, metrics; tests: binding_controller, ingress_metric_policy)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: none

## idiom
- clean: seeds ran:  2/3/0; two `for _ in 0..` loops are test harnesses (ingress_policy.rs:905,1080),and the three hand-written `Default` impls preserve invariants the field-wise derive would break (config.rs:137, controller.rs:364, ingress_policy.rs:268), per U1 lens-card false-positive class.



## own
- d2b-provider-observability-otel#1 sev=low blast=leaf effort=S verdict=actionable - provider-agent methods clone their input strings only to hand them to a token parser, though parse_closed_token could borrow - fix: change parse_token/parse_closed_token (agent.rs:224-244) to take `value: &str` (BoundedToken::parse takes `impl Into<String>`, so `&str` satisfies it),and drop the five `clone()` calls in session_connect/process_effect - [agent.rs:255, agent.rs:256, agent.rs:283, agent.rs:285, agent.rs:288]
  evidence: seed `\.clone\(\)` over src = 17 hits (7 in agent.rs;5 avoidable via the borrow-taking parse;2 at agent.rs:332-333 required for ownership transfer into `ToolkitAuditEvent::new`)


## type
- d2b-provider-observability-otel#2 sev=low blast=leaf effort=S verdict=actionable - ProviderAgentAuditEvent stores the four closed audit strings as `String`/`Option<String>` and immediately discards the validated `BoundedToken` (parse-then-copy-back at as_str().to_owned()) - fix: store `BoundedToken`/small enums in the event fields (agent.rs:59,66-68),and render through `BoundedToken::as_str` in the Serialize impl (wire output unchanged) - [agent.rs:56, agent.rs:265, agent.rs:297]
  evidence: seeds ran: `fn validate_|fn check_` = 3 hits (all boundary validators), `is_: bool|flag: bool` = 0, `(mode|kind|state): String` = 0; reading: event/authz_decision/provider/domain are parsed into `BoundedToken` (agent.rs:230-298) then converted back to String for storage


## api
- clean: seeds ran:  121 pub items/1 Arc-in-signature/6 re-export arms; `pub use` re-export arms in lib.rs are the house single-surface pattern (U1 lens-card FP),and the sole `Arc<dyn IngressClock>` signature (ingress_policy.rs:331)is justified shared ownership - tests create one ManualClock and clone the Arc into multiple gates (ingress_policy.rs:902,1208,1286)


## err
- d2b-provider-observability-otel#3 sev=low blast=leaf effort=S verdict=actionable - when the connection-tracking table is full, reject() reports `IngressErrorClass::Malformed` ("frame could not be decoded") though the frame may be valid, whereas the sibling capacity refusal reports `None` - fix: return `IngressOutcome::Rejected, IngressErrorClass::None)` on that branch(or a distinct class, if one is introduced for wire labeling),consistent with the capacity path at ingress_policy.rs:460 - [ingress_policy.rs:647]
  evidence: reading of reject() full-table branch; seed `let _ = |\.ok\(\);` = 7 hits (all deliberate best-effort cleanups or test drills),and wire-visible error classes are the `as_str` labels of IngressErrorClass (ingress_policy.rs:87-91)


## serde
- clean: seeds ran:  1/0/1/10; the hand-written `Deserialize` for ProviderConfig (config.rs:127)is a live strict admission gate over untrusted config (the recorded refusal class: hand-written Deserialize admission gates - do not re-flag),the hand-written `Serialize` for ProviderAgentAuditEvent (agent.rs:68)and ProviderConfig (config.rs:118) render redacted/canonical shapes deliberately,and all serde_json sites are round-trip tests or the deliberate canonical-size measurement


## obs
- clean: seeds ran:  0/0/0/3; zero println/format-interpolated events(only message-only events with named fields: provider, binding, ingress, outcome, error_class, connection),all diagnostic events carry `provider = "observability-otel"` as a field,andzone/source redaction lives in the Debug/Serialize overrides (deliberate; agent.rs:72-86,88-99) rather than in log calls





## docs
- d2b-provider-observability-otel#4 sev=medium blast=leaf effort=S verdict=actionable - the three crate-identity constants `PROVIDER_NAME`,`PROVIDER_REF`,`PROVIDER_API_MAJOR` in lib.rs lack doc comments while every sibling public item in the crate carries one - fix: add one-line doc comments naming each constant's role (mirroring the documented `OTEL_HOST_BRIDGE_ROLE` on the next line) - [lib.rs:13, lib.rs:14, lib.rs:15]
  evidence: seed `^\s*pub (fn|struct|enum|trait|const|type)` = 121 hits over src; the three constants are the only bare undocumented pub items found by reading lib.rs
- d2b-provider-observability-otel#5 sev=medium blast=leaf effort=M verdict=actionable - Result-returning pub API fns lack `# Errors` doc sections naming their failure conditions, leaving callers to infer variants from code - fix: add `# Errors` sections to at least the five representative fns (ProviderAgentProcess::new/session_connect/process_effect, ProviderConfig::from_json, TelemetryServiceController::reconcile, TelemetryComponentSession::open_stream,EmitterSocket::bind/drain_once, validate_resource_attributes),enumerating e.g. `ProviderAgentError::{SessionDenied,AuditBackpressure,InvalidInput}` - [agent.rs:216, config.rs:147, controller.rs:100, emitter_socket.rs:131, metric_policy.rs:21]
  evidence: seed `-> Result<` = 22 hits (11 of them pub fn signatures across 6 modules); none of the pub Result fns' doc comments contain a `# Errors` section (seed `/// # (Examples|Errors|Panics|Safety)` = 0 hits over src)


## perf
- d2b-provider-observability-otel#6 sev=low blast=leaf effort=S verdict=actionable - drain_once allocates a fresh 64KiB+1 scratch buffer per datagram inside the drain loop ( <= 256 iterations/call),when one buffer reused across recv calls would suffice - fix: hoist `let mut bytes = vec![0_u8; MAX_COMPACT_FRAME_BYTES + 1];` above the while loop,and `bytes.resize(MAX_COMPACT_FRAME_BYTES + 1, 0)` per iteration; the queued redacted frame remains its own owned Vec from redact_parsed_frame - [emitter_socket.rs:139]
  evidence: static (unmeasured); seed `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)` = 6 hits; he deep read of drain_once found the per-iteration allocation
- d2b-provider-observability-otel#7 sev=low blast=leaf effort=M verdict=actionable - admit_for_connection re-measures every frame by re-serializing the whole MetricFrame to JSON(allocating a Value tree plus a String per admission),though the wire-boundary paths already carry `encoded_bytes` - fix: thread the canonical measured size through from the decode boundary (admit_raw/admit_parsed/metric_frame_from_raw) instead of re-calling measured_encoded_bytes in admit_for_connection, preserving the documented trustless measurement at the boundary(ingress_policy.rs:202-203)rather than per admission - [ingress_policy.rs:203, ingress_policy.rs:368]
  evidence: static(unmeasured; seed `format!\(` = 1 hit(cold construction path))and `serde_json::to_` = 10 hits; reread of admit_for_connection (line 395+) shows measured_encoded_bytes called for every frame before policy evaluation
- d2b-provider-observability-otel#8 sev=low blast=leaf effort=S verdict=actionable - valid_resource_attribute_value allocates a lowercase copy of each attribute value(`to_ascii_lowercase()`)on the per-frame resource-attribute validation path,only to substring-test six words - fix: replace the allocation with a case-insensitive byte-scan helper(e.g. a local `contains_ignore_ascii_case(value, word)`)over the already-bounded( <= 256-byte)value - [metric_policy.rs:44]
  evidence: static(unmeasured; seed `\.to_string\(\)` = 20 hits(most are wire-map construction); the identified site allocates per attribute value per admission frame(validate_resource_attributes is called from admit_for_connection at ingress_policy.rs:411))


## conc
- clean: seeds ran:  0/0/20/0; zeroproduction threads/locks/atomics; all `AtomicU64`/`AtomicUsize` + `Ordering` hits are in `#[cfg(test)]` harnesses (ManualClock in ingress_policy.rs:769-775,and SOCKET_SEQUENCE in emitter_socket.rs:380-383),test-only synchronization per U1 lens-card FP; production shared state is the single `Arc<dyn IngressClock>` trait-object ownership already judged under api


## async
- N/A: seeds ran:  0/0/0/0; no async fn/.await/tokio::spawn/tokio::sync anywhere in src,andthe crate declares no tokio dependency - the whole crate is a synchronous library


## unsafe
- N/A: seeds ran:  0/0/0/1; seeds1-3 all zero(no unsafe blocks/fns/impls,no SAFETY comments,no transmute/from_raw/MaybeUninit/zeroed),and seed4 alone - the `#![forbid(unsafe_code)]` attribute(lib.rs:3)- does not make the lens applicable per U1 lens-card



## ffi
- N/A: seeds ran:  0/0/0/0; no extern "C"/no_mangle/link_section/catch_unwind/repr(C)/repr(transparent)/CStr/CString/c_char anywhere; the rustix fchmod/fstat call sites(emitter_socket.rs:86,282)are safe-wrapper syscall call sites that never cross a foreign caller(U1 lens-card FP)



## macro
- N/A: seeds ran:  0/0/0/0; no macro_rules!/proc_macro/syn/quote/$crate/to_compile_error/new_spanned anywhere; only std macros and derives exist,which are not definitions per U1 lens-card FP



## test
- d2b-provider-observability-otel#9 sev=medium blast=leaf effort=S verdict=actionable - the resource-attribute validation test asserts only `is_err()` for both failure shapes,so a regression swapping the two wire-visible variants(`NotAllowlisted` vs `Invalid`)would pass - fix: replace the two `is_err()` assertions in `resource_attributes_have_a_separate_allowlist` with `assert_eq!)..., Err(ResourceAttributeError::NotAllowlisted))` for the unknown-key case,and `assert_eq!)..., Err(ResourceAttributeError::Invalid))` for the credential-canary value case - [metric_policy.rs:145, metric_policy.rs:150]
  evidence: seed `assert_eq!\(|assert_ne!\(|assert!\(` over src+tests = 158 hits(assert! mass incl.the two variant-blind is_err sites at metric_policy.rs:145,150); the variants' Display codes differ("otel-resource-attribute-not-allowlisted" vs "otel-resource-attribute-invalid", metric_policy.rs:83-91),so a caller can match on them


## Coverage
- idiom: clean(seeds ran:  2/3/0; index loops in test harnesses; Default impls deliberate)
- own:  1 finding(s)
- type:  1 finding(s)
- api: clean(seeds ran:  121/1/6; re-exports house pattern; Arc sharing justified by tests)
- err:  1 finding(s)
- serde: clean(seeds ran:  1/0/1/10; hand-written gates deliberate per refusal class)
- obs: clean(seeds ran:  0/0/0/3; no println; named-field events only)
- docs:  2 finding(s)
- perf:  3 finding(s)
- conc: clean(seeds ran:  0/0/20/0; all atomic hits test-only)
- async: N/A(seeds:0/0/0/0; no async fn or tokio dep)
- unsafe: N/A(seeds:0/0/0/1; forbid(unsafe_code) attribute alone does not apply per U1)
- ffi: N/A(seeds:0/0/0/0; no FFI surface; rustix wrappers are not crossings)
- macro: N/A(seeds:0/0/0/0; no macro definitions)
- test:  1 finding(s)