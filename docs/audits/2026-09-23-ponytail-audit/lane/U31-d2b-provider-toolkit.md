# U31 d2b-provider-toolkit
Lean already. Ship.

No new findings. The complete live surface of the shared Provider authoring
framework - `src/lib.rs` (framework doc + full re-export arms), `src/service.rs`,
`src/shared_provider.rs`, `src/credential.rs` (the shared Credential realization
keyed on `CredentialProviderKind`), `src/audit/{mod,redaction}.rs`, `src/declaration/{mod,schema,manifest}.rs`,
`src/credential.rs`, and the workspace consumers of every seeded item - was re-read
in full in this pass. Every public item in the live half has a production caller or a
ledger-applied twin:
- `credential::authorized_service_record` / `credential_frame` / `dispatch_blocking` /
  `block_on` are the one audit record, telemetry frame, and dispatch seam the three
  Credential binaries share (U53 #PR1/#PR2, toolkit-owned since both the broker and
  the daemon host the same contract).
- `EffectService` / `EffectServiceFactory` / `ServiceInvocation` - the daemon hosts
  one actor per declared service and dispatches the envelope's canonical payload
  (R8); the realizer crates' `service.rs` implement it.
- `SharedProviderSpecEnvelope` / `shared_provider_spec_decoder` / `emit_canonical` /
  `verify_canonical` - consumed by the CLI (`src/bin/d2b-provider-toolkit.rs`) and by
  the installation/verify commands.
- `Redacted` - the structural redaction wrapper for values that must never render,
  exercised by its own ring tests and by the Credential providers' audit records.

The refused surface (U1 packet #B3, "toolkit's unconsumed framework half");
`src/server/`, `src/operations/`, `src/plane/`, `src/base/`, `src/testing/` - the
declared-but-unwired framework half the Provider lanes instantiate - stays per the
refusal ledger. No new evidence suggests any piece of that half got a caller since
the refusal was recorded; the 5x identical read warnings confirm the framework half
files are unchanged at HEAD. The `normalize_integral_numbers` / `structured` JSON
normalizers and the `declaration`/`audit` modules are live; the schema-
canonicalization vectors are tested and consumed by the manifest emitters.

No zero-caller claim is made, so no caller verification was waived; the workspace-wide
consumer census for the toolkit's live items (the daemon's envelope dispatch, the
credential realizer crates' dispatch seams, the CLI manifest commands) all have
in-tree callers at HEAD.
