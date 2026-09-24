# U17 d2b-session

net: -319 lines, -0 deps

- `delete:` `src/audit.rs` (319 lines: 232-line production `SessionAuditWriter`/`SessionAuditRecord`/`session_connect_record`/`session_connect_record_with_trace`/`session_process_effect`/`session_process_effect_with_trace`/`AuditFrameDecoder`/`AuditFrameEncoder`/`SessionAuditEventBuilder`/`KeepaliveAuditPayload` surface, public via `pub mod audit;` at `packages/d2b-session/src/lib.rs:10`, plus its 87-line `mod tests`) - the entire module is an unproduced cross-crate audit-vocabulary copy with zero workspace callers, and the non-trace function pairs duplicate the identical with/without-trace wrapper pattern.

## Consistency
Identical class to the already-applied cross-crate audit-vocabulary deletions in five elsewhere-ledger crates (#G98/#S1/#S2/#S4 and the audit.vocabulary cross-crate row), duplicated verbatim here. zero external callers workspace-wide for every public item (`grep -rl` scans listed in the report file); in-crate production files (engine/driver/server/admission/client/streams/scheduler/cancellation/deadline/lifecycle) contain zero `audit` references - the only production reference is the `pub mod audit;` arm in lib.rs. The crate's own live metrics surface is `metrics.rs` (record_establishment) - audit vocabulary has zero reachable production users.

## Checked
Read all 22 src files + tests/ (component_session.rs 2612, admission.rs 1958, noise_vectors.rs 303) + Cargo.toml, BUILD.bazel (d2b_session + d2b_session_test_support globs `src/**/*.rs`), lib.rs. Ran workspace-wide `grep -rl` caller sweeps for each of audit.rs's public items - zero external callers in every file outside the module itself.

Net: -319 lines, -0 deps
