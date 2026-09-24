# U64 — d2b-provider-process (+ shared toolkit family U31)

## Verdict
Lean already. Ship.

No NEW findings. Both in-scope live surfaces were audited in full and every
distinctive public item has workspace consumers or a ledger-applied home.

## Crate layout (in-scope)

The toolkit (`packages/d2b-provider-toolkit`, ~13.7K LOC) and the Process
family realizer (`packages/d2b-provider-process`, ~8.8K LOC) together own the
provider-neutral framework half the frozen Provider catalog composes. The
prior ledger for this family (U1 packet) holds the framework half as refused
rows (#B3 for the toolkit's `server/operations/plane/base/testing`; paired
rows for the process driver family in the plan's U49/U32 columns) - all in
the "declared-but-unwired, refused stays refused" refusal class. Honoring
those, in-scope for NEW findings was the **live** authoring surface only:

- toolkit `lib.rs` / `service.rs` / `shared_provider.rs` (61.8KB) /
  `credential.rs` - the shared envelope, capability object, and driver the
  realizer crates implement and the daemon consumes from the same contract;
- toolkit `declaration/{mod,schema,manifest}.rs` - the one canonical
  manifest + root-schema emitter used by the `d2b-provider-toolkit` CLI
  binary (`src/bin/d2b-provider-toolkit.rs`);
- toolkit `audit/{mod,redaction}.rs` - the bounded audit ring + `Redacted`
  redaction wrapper;
- toolkit `bin/d2b-provider-toolkit.rs` - the manifest emit/verify CLI;
- process `identity.rs` / `worker_launch.rs` / `facets.rs` /
  `effects_service.rs` / `driver.rs` / `backend.rs` / `execution.rs`.

## What was checked

### Caller census (workspace-wide, mandatory per U1 constraint 2)
Every distinctive live export was grepped across `packages/` for consumers:

- `shared_provider_spec_decoder`, `shared_provider_spec_decoder`,
  `dispatch_blocking`, `dispatch_async`, `credential_frame`,
  `authorized_service_record`, `allowed_service_record`, `resource_uid`,
  `owner_ref`, `key_ref`, `decode_metadata`, `emit_canonical`,
  `emit_command`, `verify_command` - all have in-crate or cross-crate
  production readers:
  - the three Credential realizer crates (`credential-secret-service`,
    `credential-managed-identity`, `credential-entra`) call
    `d2b_provider_toolkit::credential::dispatch_blocking`/`dispatch_async`
    from their `service.rs` dispatch arms (U53 #PR2 [partial] live surface);
  - `emit_canonical` / `verify_canonical` / `validate_for_installation` are
    reached from the toolkit's own `d2b-provider-toolkit` bin and from the
    manifest emitters in the realizer crates' `driver.rs` (A1/P1 applied
    rows);
  - `Redacted` / `credential_frame` / `authorized_service_record` are
    consumed by the credential realizer telemetry/audit halves (live);
  - `resource_uid` / `key_ref` / `owner_ref` in shared_provider are used by
    the Device/Process/Volume family drivers (deterministic
    `deterministic_resource_uid` derivation at `worker_launch.rs` +
    `device_worker.rs` + `resource_runtime.rs:3216,3616,4957`).

No zero-caller claim survives verification; every item named above has a
live caller.

### Process family live surface
`resolve_launch_identity`, `LaunchRow`, `ProcessResourceIdentity`,
`ProcessFamilySpec`, `ProviderAdoption`, `ProcessRequest`/
`ProcessLaunchRequest` / `ProcessStopClass`, `WorkerTemplate`, the
`DeviceWorkerLaunch`/`ServingWorkerLaunch` facet machinery, and the
`deterministic_resource_uid` + `teardown_rank` helpers are all exercised by
the family's own driver state machine and its effects service; the
uid-to-UUIDv4 renderer resolution rides the shared `ResourceRef`/
`ProcessResourceIdentity` path, not a hand-rolled copy (already-migrated
call sites per U49/#S1 rows).

### Stdlib / platform-dup scan
No hand-rolled UUIDv4 renderer, no hand-rolled base64 codec, no re-invented
bounded ring, and no `itoa`/`from_bytes` duplication remains in the live
surface; the shared `ResourceUid::from_bytes` at
`d2b-provider-toolkit/src/identity.rs` (U64/U49 #S1-applied) covers the
family's renderer.

## Refusal ledger honored
- #B3 [refused] toolkit unconsumed framework half (`server/operations/plane/base/testing`) - honored, no new evidence; the framework stays unwired as prior.
- #PR4/#PR12/#U31-#PR11/#G17/#G18 (applied rows named server/operations half in toolkit + process family driver halves) - honored as applied; no deletion target restated.
- #U49/#U32 process driver family [refused] rows - honored; the family driver stays instantiated by the daemon composition (d2bd/src/composition.rs), consistent with U90/U91 no-findings rows.

## Reopened refusals
None. No new evidence against any refused row; all refused surface stays.

## Findings
None new. Ship.
