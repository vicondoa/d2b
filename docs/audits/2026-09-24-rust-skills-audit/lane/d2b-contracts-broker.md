# d2b-contracts-broker - d2b-contracts-broker
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 5215 (excl. src/generated/**) | modules: whole crate (broker_wire, host_generation, kernel_client, lib, tests/wire.rs)
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate (single-part lane)

## idiom
- d2b-contracts-broker#1 sev=low blast=leaf effort=S verdict=actionable - `response.refusal.clone().unwrap_or_default()` runs inside an `if response.refusal.is_some()` branch, so the default is unreachable and the value is cloned twice - fix: restructure to `if let Some(code) = response.refusal.clone()` or match on the Option once, returning `KernelInvokeError::Refused` in the Some arm - [packages/d2b-contracts-broker/src/kernel_client.rs:225-227]
  evidence: seed 3 `let mut \w+ = (String|Vec)::new\(\)` = 1 hit; the is_some/unwrap_or_default pair read at kernel_client.rs:225-227
- d2b-contracts-broker#2 sev=low blast=leaf effort=S verdict=actionable - `ApplyHostGenerationHandoff::validate` carries a caller-role check that can never fire: `HandoffCallerRole` has exactly two variants and the `!matches!(Lifecycle | Admin)` guard is always false, so the `InvalidTransition` arm is dead code in a security-adjacent validation path - fix: delete the branch (or add the missing third role if one was intended) - [packages/d2b-contracts-broker/src/host_generation.rs:162-167, packages/d2b-contracts-broker/src/host_generation.rs:131-137]
  evidence: seed 3 = 1 hit; dead branch confirmed by reading the two-variant enum at host_generation.rs:131-137
- d2b-contracts-broker#3 sev=low blast=leaf effort=S verdict=actionable - `BrokerCallerRole::for_display()` returns the bare label `"RootUid"` for `RootUid` while every sibling arm returns a stable `d2b-*` audit label, and the value lands in the broker's `peer_role` audit records - fix: align the arm to the scheme, e.g. `"d2b-root"`, and pin it in the existing label test - [packages/d2b-contracts-broker/src/broker_wire.rs:2904-2912, packages/d2b-contracts-broker/src/broker_wire.rs:3149-3163]
  evidence: seed 3 = 1 hit; census: `for_display` over packages/ = 12 hits, consumed as `peer_role` audit field at packages/d2b-broker/src/runtime.rs:1654,1731,2456
- clean: seeds ran (0/0/1); the single `let mut received = Vec::new()` accumulation loop is a side-effect fd-collection drain where the plain loop is the skill's own preference; no index loops, no hand-written derives over derivable ones

## own
- clean: seeds ran (13/71/0/0); every clone/to_owned is explainable - audit-join string materialization (broker_wire.rs:643,651,698), request-field moves into the envelope (kernel_client.rs:139-145), and the refusal clone before the response is moved (kernel_client.rs:226-227); no Rc/RefCell/Arc<Mutex>/Cow anywhere

## type
- d2b-contracts-broker#4 sev=medium blast=leaf effort=S verdict=actionable - `HandoffCoordinator.source_remains_usable: bool` is fully derivable from `state` (false iff `Completed`, true in every other phase), so the pair `{ state: Completed, source_remains_usable: true }` is an illegal state constructible through durable-record deserialization and the two fields can desync - fix: drop the field, derive the accessor from `self.state != HandoffState::Completed`, keep `#[serde(default)]` for old broker records (the wire `ApplyHostGenerationHandoffResponse.source_remains_usable` field stays as-is) - [packages/d2b-contracts-broker/src/host_generation.rs:226-232, packages/d2b-contracts-broker/src/host_generation.rs:294-315]
  evidence: seed 1 `fn validate_\w+|fn check_\w+` = 2 hits (host_generation.rs:76,256); field/accessor/mutations read at host_generation.rs:226-316; census: `source_remains_usable` over packages/ = 11 hits, consumers use the accessor (d2b-broker/src/ops/host_generation_handoff.rs:390) or the response's own wire field (d2b-provider-activation-nixos driver.rs:1331)
- d2b-contracts-broker#5 sev=medium blast=leaf effort=S verdict=actionable - `CanonicalAuditDigest(pub String)` exposes a public field that bypasses the SHA-256-spelling invariant its `parse` constructor and hand-written `Deserialize` enforce, so a literal construction can mint an invalid digest - fix: make the tuple field private and keep `as_str()` (serde transparent and JsonSchema work with a private field; wire shape unchanged) - [packages/d2b-contracts-broker/src/broker_wire.rs:2805, packages/d2b-contracts-broker/src/broker_wire.rs:2807-2821]
  evidence: seed 3 `(mode|kind|state): String` = 3 hits, all wire `kind` code strings (broker_wire.rs:971,2994,3018) that are schema-pinned false positives; census: `CanonicalAuditDigest` over packages/ = 13 hits, every production construction goes through `parse` (d2b-broker/src/runtime.rs:2419,2442; d2bd-runtime/src/broker_transport.rs:63)
- clean: seeds ran (2/0/3); the three `kind: String` fields are wire code strings mirroring the pinned schema (false positive per card); the handoff state machine's runtime phase checks are the deliberate replay-safe design, not a typestate candidate under the stopping rule

## api
- d2b-contracts-broker#6 sev=medium blast=wide effort=S verdict=needs-contract - `BrokerRequestEnvelope.test_peer_uid: Option<u32>` is a test-only peer-uid override carried on the production wire envelope (serialized, schema-visible), honored only under the broker's `config.test_mode` gate - fix: move the override out of the wire type into the broker's test harness (e.g. a test-only envelope wrapper or a `#[cfg(test)]`-visible field) so the production contract carries no test seam - [packages/d2b-contracts-broker/src/broker_wire.rs:2862-2869, packages/d2b-broker/src/runtime.rs:1628-1632]
  evidence: seed 1 = 198 pub items (contract-crate wide vocabulary is the card's false positive), seed 2 = 0; census: `test_peer_uid` over packages/ = 26 hits across d2b-broker bootstrap probe helpers, five broker test files, d2bd, d2bd-runtime and kernel_client
- d2b-contracts-broker#7 sev=low blast=family effort=S verdict=actionable - `pub use d2b_contracts::audit_wire::{AuditExportCursor, AuditExportEntry, AuditExportErrorCode}` at broker_wire.rs:13 re-exports another crate's types into this crate's surface, making each item reachable at two paths (`d2b_contracts::audit_wire::*` and `d2b_contracts_broker::broker_wire::*`), off the house single-surface pattern which places re-export arms in lib.rs - fix: move the re-export to lib.rs or drop it and let consumers import from d2b_contracts (sibling d2b-contracts-control/src/public_wire.rs:1 repeats the pattern; X3 may merge the family) - [packages/d2b-contracts-broker/src/broker_wire.rs:13, packages/d2b-contracts-broker/src/lib.rs:7-11]
  evidence: seed 3 `^\s*pub use ` = 4 hits; census: `AuditExportEntry|AuditExportCursor|AuditExportErrorCode` over packages/ = 40 hits, consumers import via the broker_wire path (d2b-broker/src/audit.rs:29, d2b/src/dispatch.rs:22, d2bd-runtime/src/wire.rs:544)
- clean: seeds ran (198/0/4); the 198-item pub surface is the deliberate contract-crate wire vocabulary (card false positive); no Arc/Rc/Box/RefCell in signatures; `RunnerLaunchArgs` and `CanonicalAuditDigest` show the private-field-plus-accessor shape; lib.rs re-export arms follow the house pattern

## err
- clean: seeds ran (114/0/26/3); every unwrap/expect/panic/unreachable site (114 unwrap/expect, 26 panic/unreachable, all listed 3223-4282) sits inside `#[cfg(test)]` (broker_wire.rs tests module, tests/wire.rs) - no panic site reachable from caller input; the three error enums (HandoffError, RunnerLaunchArgsError, KernelInvokeError) are split by caller action with stable Display codes; no swallowed Results (`let _ =` = 0)

## serde
- d2b-contracts-broker#8 sev=medium blast=wide effort=S verdict=needs-contract - `OpenUnitPidfdRequest` and `StopUnitRequest` combine `#[serde(flatten)] pub unit: UnitRequest` with `deny_unknown_fields` on the containing struct, and serde ignores `deny_unknown_fields` on any type using flatten, so unknown fields in these two wire requests are silently accepted instead of refused - fix: drop the flatten (duplicate the UnitRequest fields or deserialize into a tagged wrapper) or accept-and-validate unknown fields explicitly; the wire admission change needs contract review - [packages/d2b-contracts-broker/src/broker_wire.rs:1836-1861, packages/d2b-contracts-broker/src/broker_wire.rs:1783-1834]
  evidence: seed 2 `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)` = 307 hits; the two flatten sites read at broker_wire.rs:1841,1852 with the outer deny_unknown_fields at 1838,1849
- clean: seeds ran (134/307/0/35); hand-written Deserialize impls (ExportBrokerAuditResponse:2200, RunnerLaunchArgs:2575, CanonicalAuditDigest:2831) are live admission gates in the recorded refusal class (over-engineering-audit-record.md) and validate real invariants; enum representations (adjacent on BrokerRequest/BrokerResponse, internal on ForwardOperationOutcome/BrokerNotification with `#[serde(other)]`) and the pervasive deny_unknown_fields are deliberate pinned wire shapes

## obs
- N/A: seeds 0/0/0/0 (case-sensitive run; the only case-insensitive `log::` match is the doc-prose word `AuditLog::write_entry` at broker_wire.rs:547) and the crate has no tracing/log dependency (packages/d2b-contracts-broker/Cargo.toml)

## docs
- d2b-contracts-broker#9 sev=medium blast=leaf effort=S verdict=actionable - every Result-returning public fn lacks the canonical `# Errors` section (seed 2 `/// # (Examples|Errors|Panics|Safety)` = 0 hits crate-wide), so the failure conditions of the handoff state machine, the launch-args bounds, and the kernel client are only recoverable from enum docs - fix: add `# Errors` sections naming the `HandoffError`/`RunnerLaunchArgsError`/`KernelInvokeError` conditions to `SourceGenerationCompatibilityFloorV1::new`, `begin_handoff`, the `HandoffCoordinator` transitions, `RunnerLaunchArgs::new`, and `envelope_invoke_kernel` - [packages/d2b-contracts-broker/src/host_generation.rs:47, packages/d2b-contracts-broker/src/kernel_client.rs:113, packages/d2b-contracts-broker/src/broker_wire.rs:2495]
  evidence: seed 3 `-> Result<` = 15 hits (12 public fns: host_generation.rs:50,80,95,153,260,277,286,295,305; kernel_client.rs:118; broker_wire.rs:2495,2809; the other 3 are Deserialize trait impls), seed 2 = 0 hits
- d2b-contracts-broker#10 sev=low blast=leaf effort=S verdict=actionable - four public fns have no doc comment at all: `BrokerCapabilities::w3`, `RunnerRole::as_str`, `BrokerCallerRole::is_admin_uid`, `BrokerCallerRole::for_display` - fix: one-line contract docs (for_display should document the stable audit-label promise) - [packages/d2b-contracts-broker/src/lib.rs:28, packages/d2b-contracts-broker/src/broker_wire.rs:2444, packages/d2b-contracts-broker/src/broker_wire.rs:2900-2904]
  evidence: seed 1 `^\s*pub (fn|struct|enum|trait|const|type)` = 195 hits; the four undocumented items confirmed by reading their neighborhoods
- d2b-contracts-broker#11 sev=low blast=leaf effort=S verdict=actionable - doc-comment polish defects in the FdKind/ForwardOperationRequest contract docs: `present.from` (missing space), CJK full-width periods (the FdKind variant docs end in a CJK period), and comma-adjacent spacing (`positions,in the ... list,of`) - fix: reword those doc lines - [packages/d2b-contracts-broker/src/broker_wire.rs:246, packages/d2b-contracts-broker/src/broker_wire.rs:254, packages/d2b-contracts-broker/src/broker_wire.rs:421-430]
  evidence: seed 1 = 195 hits; typos read at the cited lines
- clean: seeds ran (195/0/15); the crate's doc discipline is otherwise strong - nearly every pub item carries a contract doc with a load-bearing first sentence, module docs exist in all four modules, and magic values (MAX_FRAME_FDS, DEFAULT_CONTEXT_DEADLINE_MS, MAX_CONTEXT_DEADLINE_MS) document the why

## perf
- clean: seeds ran (48/2/30); all format!/to_string sites are audit-join rendering (broker_wire.rs:634-730, the card's audit-rendering false positive) or cold error paths (kernel_client.rs:125-223); the two Vec::new() sites are one-shot recvmsg fd collection and test scaffolding; no hot-loop allocation, no attacker-keyed hashing; static (unmeasured) per the card

## conc
- N/A: seeds 0/0/0/0 (case-sensitive run; the case-insensitive `Atomic\w+` matches were the prose word "atomically" in doc comments at broker_wire.rs:114,2018); no threads, locks, atomics, or unsafe Send/Sync in the crate

## async
- N/A: seeds 0/0/0/0; no async fn, no .await, no tokio usage anywhere in the crate (kernel_client is a synchronous rustix/socket2 client)

## unsafe
- N/A: seeds 0/0/0/0; the crate inherits `[lints] workspace = true` with `unsafe_code = "forbid"` (packages/d2b-contracts-broker/Cargo.toml), and no unsafe block, SAFETY comment, or transmute exists

## ffi
- N/A: seeds 0/0/0/0; no extern "C", no_mangle, repr(C), CStr/CString, or catch_unwind anywhere; the crate crosses no foreign boundary

## macro
- N/A: seeds 0/0/0/0; no macro_rules!, proc-macro, syn/quote, or $crate usage; the only include is the generated `broker_operation_profiles.rs` (X2's lane)

## test
- clean: seeds ran (51/127/0/0); 51 `#[test]` (49 in broker_wire.rs tests module, 2 in tests/wire.rs) and ~127 assertions cover wire round-trips, per-field legacy-authority rejection loops with failure messages naming the field, closed-enum matrices, and deliberate wire-constant pins (FD_LEG, STALE_CONTEXT, PROTOCOL_VERSION); no `#[ignore]`, no proptest/insta/rstest; every test can fail on a real regression (the constant pins carry `#[allow(clippy::assertions_on_constants)]` with written reasons)

## Coverage
- idiom: 3 finding(s)
- own: clean (seeds ran: 13/71/0/0)
- type: 2 finding(s)
- api: 2 finding(s)
- err: clean (seeds ran: 114/0/26/3)
- serde: 1 finding(s)
- obs: N/A (seeds: 0/0/0/0 all zero; no tracing/log dependency in Cargo.toml)
- docs: 3 finding(s)
- perf: clean (seeds ran: 48/2/30)
- conc: N/A (seeds: 0/0/0/0 all zero; no threads/locks/atomics)
- async: N/A (seeds: 0/0/0/0 all zero; no async code)
- unsafe: N/A (seeds: 0/0/0/0 all zero; workspace `unsafe_code = "forbid"` inherited)
- ffi: N/A (seeds: 0/0/0/0 all zero; no foreign boundary)
- macro: N/A (seeds: 0/0/0/0 all zero; no macros)
- test: clean (seeds ran: 51/127/0/0)