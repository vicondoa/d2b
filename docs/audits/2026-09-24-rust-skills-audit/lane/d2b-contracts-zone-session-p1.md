# d2b-contracts-zone-session-p1 - d2b-contracts-zone-session - part 1/2
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 6284 (excl. src/generated/**) | modules: v3::component_session, v3::role, v3::resource_export, v3::zone_link, v3::resource_import, v3::mod, lib
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: src/v3/component_session.rs, src/v3/role.rs, src/v3/resource_export.rs, src/v3/zone_link.rs, src/v3/resource_import.rs, src/v3/mod.rs, src/lib.rs (part 1/2; wire-contract crate, U1 (d) 6-7 apply)

## idiom
- d2b-contracts-zone-session-p1#1 sev=low blast=leaf effort=S verdict=actionable - hand-written `impl Default for ReceiveSequence` and `SendSequence` duplicate what `#[derive(Default)]` generates field-for-field (u64 plus bool, both zero) - fix: add `Default` to the derive lists of `ReceiveSequence` and `SendSequence` and delete the two hand-written impls; keep `ZoneLinkLimits`' Default (zone_link.rs:126) which preserves the nonzero bounds invariant - [src/v3/component_session.rs:1935, src/v3/component_session.rs:1976]
  evidence: seed2 (`impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`) = 3 hits; the two Default bodies call `Self::new()` whose field values equal the derived zeros, so the derive is behavior-identical; ZoneLinkLimits::default() calls `default_values()` (256/32/10/300) and is correctly hand-written
- clean: seeds 1/2/3 = 0/3/0 (`for \w+ in 0..` / hand-written impls / `let mut x = String|Vec::new()`); no index loops, no statement-style accumulation, and the only hand-written impls are the two derivable Defaults above plus the invariant-preserving ZoneLinkLimits one

## own
- d2b-contracts-zone-session-p1#2 sev=low blast=family effort=S verdict=actionable - `HandshakeOffer::from(policy.clone())` at 7 sites across two crates: the by-value `impl From<EndpointPolicy> for HandshakeOffer` (component_session.rs:1172) forces a clone at every site that holds `&EndpointPolicy`, and every in-repo caller clones - fix: add `impl From<&EndpointPolicy> for HandshakeOffer` in component_session.rs and switch the 7 sites to borrow; then delete the by-value impl if the census stays clone-only - [src/v3/component_session.rs:456, src/v3/component_session.rs:473, src/v3/component_session.rs:1014, d2b-session/src/admission.rs:593, d2b-session/src/engine.rs:689, d2b-session/src/handshake.rs:112, d2b-session/src/handshake.rs:171]
  evidence: seed1 (`.clone()`) = 14 hits; census: `HandshakeOffer::from` over packages/ = 7 sites, every one passes a clone; the fix is additive so no caller breaks
- d2b-contracts-zone-session-p1#3 sev=low blast=family effort=S verdict=actionable - `ResourceExportSpec::validate_target` clones `target.resource_type()` and `target.metadata().name()` out of a borrowed `&ResourceEnvelope` only to rebuild the ref for comparison, because `ResourceRef::new` takes owned parts and the envelope exposes no borrowed accessor - fix: add `impl From<&ResourceEnvelope> for ResourceRef` (or a `resource_ref()` accessor) in d2b-contracts-resource and use it at the comparison site - [src/v3/resource_export.rs:545, src/v3/resource_export.rs:546]
  evidence: seed1 (`.clone()`) = 14 hits; `ResourceRef::new(resource_type: ResourceTypeName, metadata: ResourceMetadata, ...)` takes owned values (d2b-contracts-resource/src/v3/resource.rs:712) and no `resource_ref()` accessor exists on the envelope; the clones are cold-path but signature-forced
- clean: seeds 1/2/3/4 = 14/21/0/0 (`.clone()` / `.to_owned()|.to_vec()|.to_string()` / `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<` / `Cow<`); remaining clones are test fixtures, schema_name strings (cold), and return-owned-from-borrowed (resource_import.rs:295); no shared-state types anywhere

## type
- clean: seeds 1/2/3 = 14/0/0 (`fn validate_\w+|fn check_\w+` / `is_\w+: bool|\w+_flag: bool` / `(mode|kind|state): String`); the 14 validate fns enforce cross-field wire invariants (profile matrices, relay scope, quota fit) on parse-once types whose fields are private and constructors are the only entrance; the bool fields present (`credentials_allowed`, `writable`, `disabled`, `force_revoke`, `user_ns`, `connected`, `child_authorized`) are wire-pinned schema fields, not internal state flags; no stringly-typed state

## api
- d2b-contracts-zone-session-p1#4 sev=low blast=leaf effort=S verdict=actionable - four documented "compatibility" aliases give one type a second public path with zero in-repo callers: `ResourceExportError`, `Fairness`, `ResourceImportError`, `ZoneLinkStatus` - fix: delete the aliases (and the "Compatibility alias" doc lines); any future caller names the canonical type - [src/v3/resource_export.rs:97, src/v3/resource_export.rs:156, src/v3/resource_import.rs:86, src/v3/zone_link.rs:444]
  evidence: census: `ResourceExportError|ResourceImportError|Fairness|ZoneLinkStatus` over packages/, nixos-modules/, tests/, docs/reference/, labs/, BUILD.bazel = 0 uses, definitions only; crate is publish = false so removal is in-repo safe
- clean: seed1 (pub items) = 355 hits, seed2 (Arc/Rc/Box/RefCell in pub signatures) = 0, seed3 (pub use) = house re-export arms in mod.rs; the wide exported vocabulary is the deliberate contract-crate surface (U1 (c) api false positives), every struct keeps private fields with constructor validation, and no dependency type leaks into a signature

## err
- clean: seeds 1/2/3/4 = 96/0/0/10 (`.unwrap()|.expect(` / `let _ = |.ok();` / panic family / `enum \w*Error`); 14 non-test expects all name a checked invariant ("fixed slice" after explicit length checks, "bounded by u16 constant" at 2782); the remaining 82 unwraps sit in #[cfg(test)] modules; no panics, no swallowed Results, and the 10 error enums are closed per-contract sets whose Display strings are stable diagnostic labels (not the d2b_core wire catalog)

## serde
- clean: seeds 1/2/3/4 = 52/105/0/11 (derive / serde attrs / `impl .*Deserialize.*for` / serde_json); every hand-written Deserialize is a live admission gate (Wire struct with deny_unknown_fields plus the validated constructor, e.g. RoleRule role.rs:407, ShareQuota resource_export.rs:229, ZoneLinkLimits zone_link.rs:132, ResourceImportSpec resource_import.rs:405), BoundedVec's visitor enforces MIN/MAX at parse (component_session.rs:110), ChannelId validates its range in deserialize (component_session.rs:1627), rename_all is consistent camelCase/kebab-case/lowercase, and round-trip tests exist for the spec types

## obs
- clean: seeds 1/2/3/4 = 0/0/0/0 (`println!|eprintln!` / interpolated events / `instrument` / `tracing::|log::`); a pure contract crate emits no telemetry and needs none

## docs
- d2b-contracts-zone-session-p1#5 sev=medium blast=wide effort=S verdict=actionable - the 37 pub wire constants at the top of component_session.rs (canonical lengths, clock skew, queue and deadline bounds) carry no doc comments at all, leaving the "why" of each wire value unrecorded in the contract crate that pins them - fix: add one-line `///` docs naming the wire role of each constant, mirroring the documented constant blocks in role.rs:30-58 - [src/v3/component_session.rs:27, src/v3/component_session.rs:51, src/v3/component_session.rs:61]
  evidence: docs seed1 (pub items) = 355 hits; lines 27-63 verified doc-less against the same-file module header, while role.rs/resource_export.rs/zone_link.rs/resource_import.rs constant blocks all carry `///` lines
- d2b-contracts-zone-session-p1#6 sev=medium blast=wide effort=M verdict=actionable - the binary wire codec's pub methods carry no doc contract and no `# Errors` sections, even though their failure conditions are the wire contract: preface, canonical encode/decode, record and fragment headers, fragment and nonce sequences, deadline admission, and credit accounting - fix: add first-sentence docs plus `# Errors` naming the failure variants on the Result-returning methods (representative set: ComponentSessionPreface::new/encode/parse, EndpointPolicyIdentity::validate/encode_canonical/decode_canonical, HandshakeOffer::validate/encode_canonical/decode_canonical, HandshakeAccept::encode_canonical/decode_canonical, RecordHeader::validate/encode/decode, FragmentHeader::validate, FragmentSequence::begin/accept, ReceiveSequence::accept, SendSequence::take, RequestEnvelope::admit, AttachmentCredits::reserve, BoundedVec::new, ChannelId::named, CancelRequest::acknowledge, BootstrapIdentityBinding::validate) - [src/v3/component_session.rs:207, src/v3/component_session.rs:938, src/v3/component_session.rs:1053, src/v3/component_session.rs:1337, src/v3/component_session.rs:1684, src/v3/component_session.rs:1829, src/v3/component_session.rs:2445, src/v3/component_session.rs:2732]
  evidence: docs seed2 (`/// # (Examples|Errors|Panics|Safety)`) = 0 hits across the lane; seed3 (`-> Result<`) = 102 hits; the listed methods verified doc-less while the role.rs/resource_export.rs/zone_link.rs/resource_import.rs constructors are documented, so the gap is specific to the component_session codec

## perf
- clean: seeds 1/2/3 = 20/24/3 (`format!(` / collection `::new()` / `.to_string()`); format! is confined to cold `JsonSchema::schema_name` and #[cfg(test)] fixtures, collection news are test vectors, BinaryWriter is pre-sized with `with_capacity`, and the encode/decode paths are single-pass with no hot-loop allocation; static (unmeasured)

## conc
- N/A: seeds 0/0/0/0 all zero (`std::thread::|thread::spawn|thread::scope` / `Mutex<|RwLock<` / `Atomic\w+|Ordering::` / `thread_local!|unsafe impl (Send|Sync) for`); no threads, locks, or atomics in a pure wire contract

## async
- N/A: seeds 0/0/0/0 all zero (`async fn|async move|.await` / `tokio::spawn|spawn_blocking|JoinSet|select!|join!` / `tokio::sync::(Mutex|RwLock|Notify)` / `#[tokio::(main|test)]|Runtime::block_on`); no async code in the contract layer

## unsafe
- N/A: seeds 0/0/0 all zero (`\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern` / `// SAFETY:` / `transmute|from_raw|MaybeUninit|mem::zeroed`); seed 4 `unsafe_code` = forbid inherited through `[lints] workspace = true` (Cargo.toml), no local exceptions

## ffi
- N/A: seeds 0/0/0/0 all zero (`extern "C"|no_mangle|unsafe(link_section` / `catch_unwind` / `repr(C)|repr(transparent)` / `CStr|CString|c_char`); no FFI surface

## macro
- clean: seeds 1/2/3/4 = 3/0/0/0 (`macro_rules!` / `proc_macro|syn::|quote!` / `$crate` / `to_compile_error|new_spanned`); `closed_enum`, `wire_enum_values`, and `bounded_bytes` are impl-per-type generation (a genuine macro answer) with narrow ident/literal fragment specifiers, invoked only in the defining module where their unqualified `BinaryError` reference resolves, and no proc-macro machinery

## test
- d2b-contracts-zone-session-p1#7 sev=medium blast=leaf effort=M verdict=actionable - the security-relevant codec state machines have no tests: RequestEnvelope::admit deadline/skew/lifetime math, FragmentSequence ordering/duplicate/complete detection, ReceiveSequence replay and nonce exhaustion, SendSequence::take, AttachmentCredits::reserve and process_pool, and the canonical round-trips of RecordHeader/FragmentHeader/HandshakeAccept, while the suite covers only policy validation, redaction, and frozen enum vectors - fix: add unit tests asserting the error variants (InvalidDeadline, Reordered, Duplicate, Replay, NonceExhausted, CreditExceeded) for each state machine, plus encode/decode round-trips for the header types - [src/v3/component_session.rs:2445, src/v3/component_session.rs:1829, src/v3/component_session.rs:1916, src/v3/component_session.rs:2732]
  evidence: test seeds = 25 `#[test]` / 110 asserts / 0 proptest/insta/rstest / 0 `#[ignore]`; grep `FragmentSequence|ReceiveSequence|SendSequence|\.admit\(|process_pool` over tests/ = 0 hits, and no header round-trip test exists outside the enum-vector golden tests

## Coverage
- idiom: 1 finding(s)
- own: 2 finding(s)
- type: clean (seeds ran: 14/0/0)
- api: 1 finding(s)
- err: clean (seeds ran: 96/0/0/10)
- serde: clean (seeds ran: 52/105/0/11)
- obs: clean (seeds ran: 0/0/0/0)
- docs: 2 finding(s)
- perf: clean (seeds ran: 20/24/3)
- conc: N/A (seeds: 0/0/0/0 all zero; no threads/locks/atomics)
- async: N/A (seeds: 0/0/0/0 all zero; no async fns)
- unsafe: N/A (seeds: 0/0/0 all zero; manifest inherits unsafe_code = "forbid")
- ffi: N/A (seeds: 0/0/0/0 all zero; no FFI surface)
- macro: clean (seeds ran: 3/0/0/0)
- test: 1 finding(s)