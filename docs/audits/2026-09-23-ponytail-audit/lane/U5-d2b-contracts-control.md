# U5 d2b-contracts-control
net: -903 lines, -0 deps

## Findings (ranked biggest cut first)

1. [delete] local UsbSecurityKey*/UsbSk* wire-twin twin twin copy family - `public_wire.rs:3755-3951` (197 lines: the crate-local `UsbSecurityKeyStatusRequest/Response`/`UsbSecurityKeySessionsRequest/Response`/`UsbSecurityKeyCancelRequest/Response`/`UsbSecurityKeyTestRequest/Response` twin twin copy twins + `UsbSkPhysicalKey`/`UsbSecurityKeyTestCheck`/`UsbSecurityKeyCancelDryRunOutputV1`? - twin twin copy family) + `cli_output.rs:697-735` (39 lines local `UsbSk*` CLI-output family). The live security-key wire uses the canonical `d2b-contracts::security_key` types which are pinned by the contract enums (`PublicRequest::UsbSecurityKeyCancel(d2b_contracts::security_key::SecurityKeyCancelRequest)`, `PublicRequest::UsbSecurityKeyStatus`/`UsbSecurityKeySessions` unit variants, `PublicRequest::UsbSk*` unit variants, `PublicRequest::UsbSkSessions` - all canonical-unit) and the daemon emits them via resource-plane `d2b_contracts::security_key::SecurityKeyStatusResponse`. Zero in-tree Rust consumers for the local twin family (verified workspace-wide grep: only self + tests + crate's own generated daemon-api.md + daemon-api.md table rows pinned by the committed generated daemon-api.md table; zero `.into()/.from()` callers). Delete + regenerate. ≈ **-236 lines**. [packages/d2b-contracts-control/src/public_wire.rs:3755-3951, cli_output.rs:697-735] (leaf | delete)

2. [delete] `cli_output.rs` dead CLI-output Vm* families (VmExecCreateOutputV1+LaunchOutputV1+VmExecCreateOutputV1/VmExecListOutputV1/VmExecListEntryOutputV1/VmExecStatusOutputV1/VmExecLogsOutputV1/VmExecKillOutputV1 60-163 ≈104; VmDisplayListOutputV1/VmDisplaySessionOutputV1/VmDisplayIdentitySource/VmDisplayCapabilityPreflight+Status/VmDisplayCloseOutputV1 166-253 ≈88; VmAudioStatusOutputV1/VmAudioStatusEntryOutputV1/VmAudioErrorOutputV1/VmAudioSetOutputV1 620-694 ≈75) - zero Rust consumers, zero committed cli-output schema pins, zero ADR targets. ≈ **-267 lines**. [packages/d2b-contracts-control/src/cli_output.rs:60-163,166-253,620-694] (leaf | delete)

3. [delete] terminal_wire From conversions + unused terminal DTO family - byte-identical `From` conversions (`terminal_wire.rs:219-497`, 279 lines) + 7 unused terminal DTOs (`TerminalSignal` 80-97, `TerminalWait` 119-135, `TerminalClose` 137-146, `TerminalControlResult` 190-195, `TerminalStatus` 197-203, `TerminalWaitResult` 205-212, `TerminalCloseResult` 214-218, ≈71 lines) exercised only by the crate's own conversion tests (499-606, ≈108); zero production consumers. ≈ **-400 lines**. [packages/d2b-contracts-control/src/terminal_wire.rs:80-218,219-497,499-606] (leaf | delete)

## Consistency
- `AuditEntry` (public_wire.rs:2677-2681) - local, zero in-tree consumers; legacy v1 wire-protocol.json schema pins the name (committed legacy v1 schema; class "committed legacy schemas / pins") → **refused** (stays; same reasoning as U1-lane audit ledgers A6/C4).
- `AuditCursor`+ `validate_audit_page`? (public_wire.rs:2204-2214, 11 lines) - byte-identical duplicate of d2b-contracts/src/audit_wire.rs:52? verified → live admission gate → refused (stays). Not new; U1 lane #C4-type refusal class.
- `validate_audit_page` (public_wire.rs:2204-2214?)?? - local twin byte-identical to canonical `d2b-contracts/src/audit_wire.rs` admission; stays (live admission gate; refusal class: live live admission gates).
- Cli-output pinned families: `ListOutputV2`/`StatusOutputV2`/`UsbProbeOutputV1`/`OpInspectOutputV1`/`AuditOutputV2`/`AuthStatusOutputV2` + their unit/schema twins, plus `vm display output families`? - checked, all live via committed cli-output.schema.json + daemon-api.md + cli-contract.md.
- UsbSk in cli_output + UsbSecurityKey in cli_output counted as dead (finding 1).
- `terminal_wire` live types (`TerminalStream`, `TerminalSize`, `TerminalWriteStdin`, `TerminalReadOutput`, `TerminalResize`) stay.
- C4 [not applied] - honored per U1 ledger (still present).

## Checked
Full crate: 6 wire/DTO modules (public_wire.rs 3951 incl. tests, cli_output.rs 1352 incl. cli_output_tests + display_output_tests + display_output_test entity, terminal_wire.rs 606, terminal_wire unreliable? no - checked: public_wire.rs, terminal_wire.rs, cli_output.rs, cli_json_output_contract.rs, lib.rs, BUILD.bazel, BUILD.bazel, Cargo.toml, .bzl). Verification: workspace-wide grep for each family member name across packages/ + docs/ + xtask (workspace verify, workspace-wide grep): zero Rust consumers outside crate + tests + generated daemon-api.md + daemon-api.md table + daemon-api.md rows. deps unchanged.

## U4 execution (2026-09-24)
- F1 UsbSecurityKey*/UsbSk* wire-twin family (public_wire.rs tail + cli_output.rs UsbSk family): APPLIED. Zero external Rust consumers at HEAD; canonical wire = d2b_contracts::security_key (PublicRequest/PublicResponse arms unchanged). daemon-api.md regenerated.
- F2 cli_output Vm* families (VmExec/VmDisplay/VmAudio): APPLIED. Zero Rust consumers + zero committed schema pins at HEAD.
- F3 terminal_wire From conversions + 7 dead DTOs + conversion tests: APPLIED. Live types (TerminalStream/Size/WriteStdin/ReadOutput/Resize/WriteStdinResult/ReadOutputChunk) + their Debug redaction tests kept.
