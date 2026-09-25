# d2b-audit - d2b-audit
Baseline: 6ebdd4cec22e6537d1376e83ff7a82b00a8492c0 | LOC audited: 5221 (excl. src/generated/**) | modules: evidence_chain, export, hash_chain, lib, operation, rate_limit, reconcile, record_types, segment, sink
Lenses: idiom, own, type, api, err, serde, obs, docs, perf, conc, async, unsafe, ffi, macro, test | Partitions: whole crate

## idiom
- d2b-audit#1 sev=medium blast=leaf effort=S verdict=actionable - `read_bounded_line` is copy-pasted three times with only the error-code strings differing ("audit-export-line-*" / "audit-segment-line-*" / "audit-scan-line-*") - fix: extract one crate-private `read_bounded_line` (canonical home: a shared module or segment.rs) taking the line-limit/truncated error codes as parameters, and delete the two copies - [packages/d2b-audit/src/export.rs:255, packages/d2b-audit/src/segment.rs:980, packages/d2b-audit/src/sink.rs:421]
  evidence: idiom seed 3 `let mut \w+ = (String|Vec)::new\(\)` = 9 hits; manual read found 3 verbatim ~30-line copies of the same bounded reader differing only in error strings
- d2b-audit#2 sev=low blast=leaf effort=S verdict=actionable - `paths.retain)...)` in `export_segments_range` re-applies `is_segment_name` to every path the read_dir loop already filtered, a redundant pass over the directory listing - fix: delete the `paths.retain` block (export.rs:103-110); the push guard at export.rs:94-102 is the only filter needed - [packages/d2b-audit/src/export.rs:103]
  evidence: manual read of export_segments_range; both the loop guard (export.rs:94-102) and the retain (export.rs:103-110) apply the same `is_segment_name` predicate
- d2b-audit#3 sev=low blast=leaf effort=S verdict=actionable - `scan_chain_state` re-invokes `record.mutation_id()` and `record.zone_operation_key()` inside the block whose outer `if let` already bound them, re-deriving two SHA-256 identities per mutation record during the startup scan - fix: use the outer bindings for the `mutation_predecessors` insert, deleting the inner `if let` (sink.rs:403-410) - [packages/d2b-audit/src/sink.rs:393, packages/d2b-audit/src/sink.rs:403]
  evidence: manual read of scan_chain_state; inner if-let at sink.rs:403-410 shadows `mutation_id`/`key` bound at sink.rs:393-394, recomputing `zone_operation_key()` (two digest derivations)
- clean: seeds ran: 5/0/9; the 5 index loops are all test loops (evidence_chain.rs:282, rate_limit.rs:87, segment.rs:1250/1528, sink.rs:503), seed 2 is 0 (no hand-written Default/From/PartialEq/Eq/Clone/Hash impls; the redacting Debug impls use `core::fmt::Debug` paths), and the 9 `Vec::new()` sites are bounded readers/collectors of unknown size

## own
- d2b-audit#4 sev=low blast=leaf effort=S verdict=actionable - `OperationIdentity::parse` calls `AuditHash::parse(value.to_owned())`, allocating a String though `AuditHash::parse` takes `impl Into<String>` and `&str: Into<String>` holds - fix: pass `value` directly (`AuditHash::parse(value)`) - [packages/d2b-audit/src/operation.rs:79]
  evidence: own seed 2 `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)` = ~100 hits (mostly test fixtures); `AuditHash::parse(value: impl Into<String>)` at packages/d2b-telemetry/src/audit_hash.rs:23 accepts `&str` without allocation
- clean: seeds ran: 55/~100/0/0; every production `.clone()` is explainable (AuditHash is a `String`-wrapped newtype, not Copy - owned values require clone; `EvidenceChain::nested` clones to build the appended chain; sink/segment map and index clones are required by the owned key shapes), seeds 3-4 are 0 real hits (the 8 `Rc<` substring matches are `Arc<AtomicU8>` fields of the test-only `FailureInjector`)

## type
- d2b-audit#5 sev=medium blast=wide effort=S verdict=actionable - `EvidenceChain` derives `Deserialize` (evidence_chain.rs:50) while its accessors assume a non-empty identity list: `invoking_identity()` panics via `.last().expect)...)`, `initiating_identity()` indexes `&self.identities[0]`, and `depth()` underflows on `len() - 1`; the "identities never empty" invariant is enforced only by the constructors, so a wire payload with `"identities": []` deserializes into the illegal state - fix: hand-write `Deserialize` for `EvidenceChain` rejecting an empty `identities` (the crate's own admission-gate pattern in operation.rs), or make the accessors total - [packages/d2b-audit/src/evidence_chain.rs:50, packages/d2b-audit/src/evidence_chain.rs:115, packages/d2b-audit/src/evidence_chain.rs:121, packages/d2b-audit/src/evidence_chain.rs:102]
  evidence: manual read; census: `EvidenceChain` over packages/d2b-broker, packages/d2bd, packages/d2bd-runtime = 30 hits, all `root()`/`nested()` construction (e.g. d2b-broker/src/envelope/mod.rs:1152, d2bd/src/forward_rendezvous.rs:650), zero deserialize sites; panic is not reachable from current callers but the parse boundary admits the state
- clean: seeds ran: 7/0/0; the `validate_*` hits are filesystem-metadata and closed-domain checks at the wire boundary (validate_fields record_types.rs:1036 is the parse-once admission gate over the generated audit_catalog vocabulary; segment.rs validate_* are inode/ownership checks), `update_state`/`disruption` strings mirror the pinned wire schema, and `evidence_from_decision_result` string-matching is the wire-boundary parse into typed `DurabilityOutcome`

## api
- d2b-audit#6 sev=medium blast=leaf effort=M verdict=actionable - the crate re-exports a large surface with zero external consumers: `sink` (AuditSink/AuditSinkError/AuditWriteOutcome), `segment` (SegmentWriter/FailureInjector/FailurePoint/DEFAULT_MAX_SEGMENT_BYTES/DEFAULT_RETENTION_DAYS), `export` (ExportLine/export_segments/export_segments_range/MAX_EXPORT_*), `rate_limit` (AuditRateLimiter/AuditWriteClass/RateDecision/DEFAULT_AUDIT_WRITES_PER_SECOND), `record_types` (AuditRecord/AuditRecordFields/*Fields/AuditRecordError/AUDIT_SCHEMA_VERSION), and `reconcile`'s `reconcile`/`Reconciliation`/`DurabilityOutcome` are all exported from lib.rs:16-46 but no dependent crate references them; consumers (d2b-broker, d2bd, d2bd-runtime, d2b-session) use only evidence_chain, operation, hash_chain, and `evidence_from_decision_result`/`DurabilityEvidence` - fix: either wire the daemon-side audit writer (d2bd-runtime daemon_audit) to the sink/segment/export stack, or reduce the unwired modules to `pub(crate)` until a consumer exists (crate is `publish = false`) - [packages/d2b-audit/src/lib.rs:16, packages/d2b-audit/src/lib.rs:30, packages/d2b-audit/src/lib.rs:33, packages/d2b-audit/src/lib.rs:37, packages/d2b-audit/src/lib.rs:43]
  evidence: census: `d2b_audit::(AuditRecord|record_types|sink|segment|export|rate_limit|reconcile_durability|Reconciliation|DurabilityOutcome|AuditSink|SegmentWriter|FailureInjector|AuditRateLimiter|export_segments|MAX_EXPORT_*)` over the 4 dependent crates (packages/d2b-broker, packages/d2bd, packages/d2bd-runtime, packages/d2b-session) = 0 hits; consumed surface is d2b-broker/src/audit.rs:27, runtime.rs:7122, ops/audit_op.rs:10, d2bd/src/forward_rendezvous.rs:78
- d2b-audit#7 sev=low blast=leaf effort=S verdict=actionable - `pub use d2b_telemetry::TraceContext;` (lib.rs:16) re-exports a foreign type that nothing in the crate or any dependent references - fix: drop the re-export (or adopt TraceContext in the record envelope if it is meant to be the trace carrier) - [packages/d2b-audit/src/lib.rs:16]
  evidence: census: `d2b_audit::TraceContext` over packages/ = 0 hits; `TraceContext` appears nowhere in src/ except the re-export line (record_types uses `d2b_telemetry::canonical_export_id` directly)
- clean: seeds ran: ~145/0/12; every pub item carries a doc comment, no Arc/Rc/Box/RefCell appears in a public signature, and the `pub use` arms in lib.rs:16-46 are the house single-surface pattern (all consumed except the two findings above)

## err
- d2b-audit#8 sev=medium blast=leaf effort=S verdict=actionable - `export_segments_range` classifies a failed `AuditRecord` deserialize by string-matching the serde error's Display (`error.to_string().contains("audit-record-hash-mismatch")`) to pick the "hash-break" export error code; a reworded deserialize message silently reclassifies a chain break as "record-invalid" - fix: split parse from verification (deserialize into a wire shape, then `verify()` to surface `AuditRecordError::HashMismatch`), or have the `Deserialize` impl expose the failure class; the emitted `error_code` strings stay unchanged - [packages/d2b-audit/src/export.rs:230]
  evidence: err seed 1 `\.unwrap\(\)|\.expect\(` = 3 production hits (all invariant expects: evidence_chain.rs:121, record_types.rs:367/922) plus test unwraps; `AuditRecordError::HashMismatch` variant exists at record_types.rs:887 and is the type the string names
- d2b-audit#9 sev=medium blast=leaf effort=S verdict=actionable - `is_discardable_checkpoint_scratch_error` classifies `io::Error` by matching `error.to_string().as_str()` against three literal codes ("audit-retention-checkpoint-invalid" / "-limit" / "-unverifiable") produced by `io::Error::other` at the checkpoint read/validate sites; a reworded code silently changes the discard decision on restart - fix: introduce a private checkpoint-read error enum (or a sentinel error kind) and match on it, keeping the io::Error strings at the public boundary - [packages/d2b-audit/src/segment.rs:694, packages/d2b-audit/src/segment.rs:628, packages/d2b-audit/src/segment.rs:718]
  evidence: manual read; the three literal strings are created at segment.rs:628-630 and segment.rs:718-747 and matched verbatim at segment.rs:694-699
- clean: seeds ran: 3 production/40 test, 5 production `let _ =`, 1 test-only `unreachable!`, 4 error enums; the production expects name invariants (identities never empty, bounded identity, literally-built serde values), the `let _ =` sites are deliberate best-effort cleanups (rollback_append segment.rs:273, prune_old segment.rs:302, rotate cleanup segment.rs:470-471, checkpoint repair segment.rs:817), and the four error enums (OperationIdentityError, EvidenceError, AuditRecordError, AuditSinkError) are closed with stable Display codes

## serde
- clean: seeds ran: 18/26/4/11; all wire structs use `rename_all = "snake_case"` + `deny_unknown_fields`, optionality is correct (`#[serde(default, skip_serializing_if = "Option::is_none")]` on mutation_id/mutation_ordinal, `#[serde(default)]` on backward-compat checkpoint fields), and the four hand-written `Deserialize` impls (OperationIdentity, ZoneId, ZoneOperationKey, AuditRecord) are live admission gates - the recorded-refusal class (docs/explanation/over-engineering-audit-record.md, hand-written Deserialize admission gates); the AuditRecord gate re-verifies the record hash on every read

## obs
- clean: seeds ran: 0/0/0/5; the 5 `tracing::|log::` hits are `audit_catalog::` substrings (e.g. record_types.rs:1038, 1067), not telemetry; the crate has no println, no tracing/log macros, and no logging dependency - the crate is a library that returns errors instead of logging

## docs
- d2b-audit#10 sev=low blast=leaf effort=M verdict=actionable - no canonical `# Errors`/`# Examples` sections exist anywhere in the crate although ~50 `-> Result<` sites include the public API (export_segments_range, AuditRecord::new/verify/zone_operation_key, evidence_from_decision_result, AuditSink::open/append/prune_old, SegmentWriter::open/append, OperationIdentity::derive/parse); failure conditions are described in prose but not under the section a caller scans for - fix: add `# Errors` sections naming the AuditRecordError/AuditSinkError/EvidenceError variants on the Result-returning pub items and `# Examples` on the non-obvious constructors (AuditRecord::new, SegmentWriter::open) - [packages/d2b-audit/src/export.rs:74, packages/d2b-audit/src/record_types.rs:428, packages/d2b-audit/src/reconcile.rs:54, packages/d2b-audit/src/sink.rs:175]
  evidence: docs seed 2 `/// # (Examples|Errors|Panics|Safety)` = 0 hits; docs seed 3 `-> Result<` = 50 hits; every pub item has a first-sentence doc comment (seed 1 ~145 hits, none undocumented)
- clean: seeds ran: ~145/0/50; module docs present everywhere, every pub item documented with a strong first sentence, no `ignore`d doctests (none exist)

## perf
- d2b-audit#11 sev=low blast=leaf effort=S verdict=actionable - `scan_chain_state` converts each line with `String::from_utf8(bytes)` then `serde_json::from_str`, allocating a String per record during the open-time scan, while `segment_tail_hash` parses the same JSONL shape with `serde_json::from_slice(&line)`; use `from_slice` here too - fix: replace the from_utf8/from_str pair with `serde_json::from_slice(&bytes)` at sink.rs:386-388 - [packages/d2b-audit/src/sink.rs:386, packages/d2b-audit/src/segment.rs:970]
  evidence: perf seed 1/3; static (unmeasured); cold path (startup scan) but a per-record allocation the sibling function already avoids
- clean: seeds ran: 13 format!/9 Vec::new()/~20 to_string(); the production `format!` sites are cold segment-naming and error paths (segment.rs:1007/1028/1039, export.rs:57), the `Vec::new()` sites are bounded readers of unknown size, and the `to_string()` hits are test fixtures and wire rendering; no format! in any loop

## conc
- clean: seeds ran: 0/1/15/1; the single `Mutex<SinkState>` (sink.rs:66) guards a genuinely synchronous surface with policy-tracked `#[allow(clippy::disallowed_methods, reason = "synchronous path")]` allows, the `AtomicBool` retention_degraded uses the correct Acquire/Release pair (segment.rs:334/343), the `Arc<AtomicU8>` slots are the test-only FailureInjector, and the `thread_local!` (record_types.rs:658) is the test-support serialization counter; no threads, no unsafe Send/Sync

## async
- N/A (seeds: 0/0/0/0; no async fn, await, tokio, or spawn in the crate - the whole surface is synchronous by design)

## unsafe
- N/A (seeds: 0/0/0/1; seeds 1-3 are all zero and the single seed-4 hit is `#![forbid(unsafe_code)]` at lib.rs:3, which per the card does not make the lens applicable; manifest has no unsafe_code setting but the crate-level forbid covers it)

## ffi
- N/A (seeds: 0/0/0/0; no extern "C", no_mangle, repr(C), catch_unwind, or CStr anywhere; the libc/rustix uses are syscall wrappers inside the crate)

## macro
- clean: seeds ran: 1/0/0/0; the single `macro_rules!` (impl_redacted_debug, record_types.rs:286) is legitimate impl-per-type generation for the 9 redacting Debug impls (a derive would leak record fields), uses the narrowest fragment specifier `$type:ty`, references no crate paths (no hygiene issue), and is not a proc-macro

## test
- clean: seeds ran: 50/~200/0/0; no tests/ directory - all 50 tests are in-module `#[cfg(test)]` units covering behavior, not implementation: failure-injection loops over every FailurePoint (segment.rs:1265, sink.rs:561/605), restart/repair/rollback scenarios, idempotent replay, single-writer locking, wire-vector pins (operation.rs:325), redaction assertions (record_types.rs:1275), and a serialization-count behavior probe (sink.rs:516); deterministic timestamps, no network, no `#[ignore]`, no tautological assertions, and no property/snapshot tooling needed for this domain

## Coverage
- idiom: 3 finding(s)
- own: 1 finding(s)
- type: 1 finding(s)
- api: 2 finding(s)
- err: 2 finding(s)
- serde: clean (seeds ran: 18/26/4/11; admission-gate Deserialize impls are the recorded-refusal class)
- obs: clean (seeds ran: 0/0/0/5; the 5 hits are `audit_catalog::` substrings of `log::`)
- docs: 1 finding(s)
- perf: 1 finding(s)
- conc: clean (seeds ran: 0/1/15/1; synchronous-path Mutex with policy-tracked allows, correct atomic pair)
- async: N/A (seeds: 0/0/0/0; no async surface in the crate)
- unsafe: N/A (seeds: 0/0/0/1; seed 4 alone - `#![forbid(unsafe_code)]` lib.rs:3 - does not make the lens applicable)
- ffi: N/A (seeds: 0/0/0/0; no FFI surface)
- macro: clean (seeds ran: 1/0/0/0; impl_redacted_debug is legitimate impl-per-type generation)
- test: clean (seeds ran: 50/~200/0/0; no tests/ dir, behavior-focused unit mass, no ignored tests)
- supply-note: d2b-session declares `d2b-audit` (packages/d2b-session/Cargo.toml:17, BUILD.bazel:28) but no src/ or tests/ file references `d2b_audit` - directly evidenced unused dependency for lane X1 (census: `d2b_audit|d2b-audit` over packages/d2b-session = 4 hits, all in Cargo.toml/BUILD.bazel)