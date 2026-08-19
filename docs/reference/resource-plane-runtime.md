# Zone resource-plane runtime

`d2bd` owns one [`ZoneResourceRuntime`](../../packages/d2bd/src/resource_runtime.rs)
for each configured Zone. A runtime is opened only from the broker's
`OpenZoneStore` response for the opaque `zone-store-<zone>` identifier. The
response must contain the matching store identity and exactly one
close-on-exec database descriptor; callers cannot provide a filesystem path.

## Startup and readiness

Opening a runtime consumes the descriptor with
`RedbResourceStore::provision_owned` or `RedbResourceStore::open_owned`,
rehydrates durable metadata, reconstructs the authority index, binds the
native Resource API, and starts the fixed system-core process when the
committed policy snapshot is available. The public readiness barrier requires
all of these conditions:

- the store is open and its identity is valid;
- the Resource API and local ComponentSession registration are ready;
- the trusted Provider path has been configured;
- durable Host-global authority recovery is complete; and
- system-core and its mandatory Host/User handlers report `Ready`.

`ZoneRuntimeReadiness::is_ready` and `ZoneResourceRuntime::require_ready`
enforce this conjunction. Opening a store deliberately leaves
`provider_path_ready` false because Provider catalog configuration is an
independent trusted-bundle step. A Zone is not published as ready before that
step completes.

The status projection is emitted by the fixed system-core emitter. It contains
one `system-core-host` and one `system-core-user` handler record; missing,
duplicate, or `ProviderLifecycle` substitutions are refused.

The Wave 6 accounting contract is checked by
`policy_wave6_manifest.rs`. It contains all 258 Provider and integration work
items from the 27 Provider dossiers, maps each item once to a canonical
foundation or Provider package, and requires named validation and removal
proof. Dossier `Planned` labels are retained as source history only; they
cannot make an incomplete accounting row pass.

## Requests and authorization

The daemon resolves a request's `zoneRef` against its authoritative resource
plane. The field is a route assertion, not an authority or a way to select a
different store. Route, service, method, and readiness failures are typed.
Public `Get` and `List` requests bind the admitted local peer's `SO_PEERCRED`
uid into a request-scoped authenticated ComponentSession subject before
calling the same Resource API client used by the registered session path.
The uid is never accepted from the request envelope; it is included in the
transport and transcript bindings checked by the authorizer.
There is no fixed daemon-owned Provider identity and no public fallback to a
static manifest, SSH, a raw broker request, a caller-supplied path, or a
provider override.

Typed shell requests are the separate authenticated Resource path and retain
the same Zone routing and admin checks. Other public Resource operations do not
become available merely because a store descriptor was opened.

## Provider lifecycle boundary

The daemon composes the shared `d2b_provider::ProviderRegistry` from the
trusted v3 catalog. A missing catalog is an explicit legacy compatibility
state; a present but malformed catalog is refused and never silently downgraded
to that state. A lifecycle request must resolve a registered Provider and a
published method before its typed effect port can run. Caller role, Zone,
capability, idempotency, and per-Guest ownership are checked before the effect.

The persistent dispatcher stores admitted lifecycle operations in the
daemon-owned `provider-lifecycle.json`. Replaying an applied idempotency key
returns `Duplicate`; a pending operation is reconciled against the real effect
boundary after restart before another effect can run. Authorization refusal
does not mutate the downstream state.

## Provider acceptance boundaries

The production controller and effect-port boundaries are covered without
mocking the layers they exercise:

| Resource | Production boundary exercised | Acceptance guarantees |
| --- | --- | --- |
| `Volume` | `VolumeLocalController` with a real temporary filesystem | activation, layout readiness, restart reconstruction, cleanup policy |
| `Network` | `NetworkReconciler` with a filesystem-backed network effect/resource boundary | dependency wait, policy refusal before effects, and ordered finalization |
| TPM `Device` | `TpmResourceController` with a real state directory and `swtpm`-shaped child process | state-volume creation, process/endpoint readiness, flush, and retained state on removal |
| Cloud Hypervisor `Guest` | `CloudHypervisorController` with `/proc` identity inspection, a real child process, pidfd, and persisted recovery state | dependency gating, readiness, restart adoption, and finalization |

These adapters persist or inspect real state; they are not call-recording
mocks. Host mutation and hardware/KVM prerequisites remain separate from this
hermetic acceptance contract.

## Store restart, backup, and restore

Normal daemon restart reopens the same broker-owned store row, validates its
immutable identity, and reconstructs durable policy, catalog, authority, and
controller metadata before the readiness barrier. Shutdown asks the store to
persist its clean-shutdown marker.

`RedbResourceStore::logical_backup` captures a validated MVCC image of logical
rows and store metadata. `restore_owned` requires an empty target descriptor and
an identity-matching provisioning marker, restores into a staged database, and
publishes only after current-schema and row-integrity validation. Restore
preserves each ResourceRef, UID, generation, canonical JSON, and payload
digest; the restored store keeps the same store identity and advances
`backup_generation`. Runtime adoption occurs only after that publication.

When a live legacy TPM adoption is requested, the production Device admission
path first captures this logical backup and refuses the adoption if capture
fails. Physical schema advancement uses the equivalent
`upgrade_owned_after_backup` boundary, which consumes an identity-validated
backup before staging.

Logical restore does not support schema downgrade. A backup whose physical
schema is not the current registered schema returns `upgrade-required` before
publication; there is no best-effort conversion or live adoption of an older
schema. See [resource-store-migration](./resource-store-migration.md) for the
staged publication and crash-recovery contract.
