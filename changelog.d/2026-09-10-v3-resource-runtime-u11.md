### Added

- Land the manager-backed Resource API surface on the daemon (U8/U9 F1
  wiring): the per-Zone runtime resolves each plane's manager client and
  watch hub from the published `v3_planes` table, holds a second
  `NativeAuthorizer` whose policy mirrors the primary projection on every
  install, and routes public resource requests by type - the converted Phase
  A types ride the manager, everything else stays on the redb service. The
  per-type partition is now enforced by routing instead of by refusal.
- Project the new plane's views onto the public read contract: manager rows
  render the canonical envelope (Nix-materialized rows persist the compiled
  spec beside its authored metadata) with `type`, `metadata.uid`,
  `metadata.generation`/`revision`, and the actor's in-memory status
  (`phase`, `observedGeneration` = row generation, R11). A failed actor's
  status also carries the closed failure classification
  (`driverFailure: {operation, retryable}`): status is never persisted (R11),
  so this is the only way an operator or fixture can see why a resource
  failed.

### Changed

- Move the per-Zone SQLite spec store under the daemon's own state root
  (`<daemon-state>/zones/<zone>/spec-store.sqlite3`). The broker-provisioned
  `<state-root>/zones/<zone>` directory is owned by the zone-store principal,
  so the daemon could not create a file in it (`SQLITE_CANTOPEN`).
- `test-host-integration` builds vmChecks with the invoking user's identity
  (`--option build-users-group ""`) so the VM test driver keeps the
  privileges QEMU and its tap wiring need, without changing the host's
  global Nix configuration.

### Fixed

- The daemon/broker vmChecks that attach the `d2b-state.img` fixture disk
  boot again in a sandboxed `test-host-integration` build. The drive now
  carries QEMU `snapshot=on`, so QEMU opens the read-only store image and
  writes only to an ephemeral per-VM overlay. Without it, QEMU aborted at
  machine start ("Could not open '...state.img': Permission denied" against
  the sandbox's read-only `/nix/store`), which the test driver reported as
  `MachineError: Failed to start the following machines` /
  `Connection reset by peer` and which read as an intermittent boot flake.
- `D2B_VM_CHECK=resource-operator-activation make test-host-integration`
  passes: the daemon boots with the v3 plane, ingests the Zone bundle, and
  the fixture observes the Process slice through the manager-backed API
  (identity, owner reference, observedGeneration) plus PID continuity across
  a daemon restart.
- The Process driver now names its owning resource on the launch ticket.
  It reports the manager-resolved owner key when the manager has one, and
  falls back to the owner reference the row was authored with
  (`ResourceContext::metadata()`) for owners the manager does not manage -
  every unconverted owner, and the reason the controller Process could not
  form a launch ticket.
- The Process driver binds the launch ticket's resource revision to the
  row's generation. The new store has no zone-wide commit revision, and the
  conformance ticket rejects a zero revision.
- Process provider failures are logged where they are classified
  (`ProcessDriver`'s provider-error mapping and launch effect). The launch
  effect previously discarded the provider error outright, so a resource
  whose status is memory-only (R11) failed with no observable reason.
