# ADR 0046 reset and cutover contract

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-reset-and-cutover` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | Zone runtime bootstrap owner, `d2b` CLI cutover owner, storage/broker integrator, `d2b-resource-store-redb` owner |
| Depends on | `ADR-046-decision-register`, `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-resource-store-redb`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-provider-model-and-packaging`, `ADR-046-provider-state`, `ADR-046-components-processes-and-sandbox`, `ADR-046-primitive-resource-composition`, `ADR-046-resources-volume`, `ADR-046-resources-device`, `ADR-046-resources-network`, `ADR-046-resources-host-guest-process-user`, `ADR-046-resources-credential`, `ADR-046-resources-zone-control`, `ADR-046-zone-routing`, `ADR-046-componentsession-and-bus`, `ADR-046-nix-configuration`, `ADR-046-cli-and-operations`, `ADR-046-telemetry-audit-and-support`, `ADR-046-core-controllers`, `ADR-046-current-code-migration-map` |
| Supersedes | [ADR 0034](../adr/0034-storage-lifecycle-restart-and-synchronization.md) "Migration decision" section for the d2b 3.0 cutover; the current `d2b host migrate-storage` verb (retired per `ADR-046-current-code-migration-map` §6 and `ADR-046-cli-and-operations` "v2 command surface removed at 3.0 clean break": it served the one-time v1→v2 storage layout cutover only and has no v3 successor) |

## Purpose

This spec is the exhaustive normative contract for taking one running d2b host
from its current pre-ADR-0046 state — the daemon-only/realm-native control
plane described by the repository `AGENTS.md` and evidenced at baseline
`b5ddbed6` — to a live ADR 0046 Zone/Provider control plane. d2b 3.0 has **no
v2 compatibility layer and no in-place protocol migration**: `Realm`,
`Workload`, `d2b-realm-router` PeerSession, the legacy public wire
(`WorkloadOp`, `RealmMethod`), and every v2 CLI verb stop working the moment
the new Zone runtime activates. This is a **destructive cutover**: the
control-plane data plane (redb Zone resource store, ResourceType rows,
revisions, watches) has no import path from any prior representation.

This spec is not a second implementation surface. It is the authoritative
sequencing, consent, inventory, disposition, safety, rollback-boundary, and
verification contract that every future cutover work item, CLI command, and
Provider dossier migration procedure must satisfy. Every other ADR 0046 spec's
"Data migration" column that reads "Full reset" or "Destructive v3 bootstrap"
is scoped to the **resource-store control-plane rows only**. This spec governs
what happens to the **durable host-owned bytes** — TPM identity, SSH keys,
disk images, durable Volumes, audit history, and credentials — that must
survive the cutover of the control plane sitting on top of them.

## Cross-reference and evidence corrections

Two existing ADR 0046 artifacts reference content that this spec supplies
canonically. Recording the correction here (rather than editing those files,
which are out of this work item's scope) keeps the set internally consistent
without an open decision:

- `ADR-046-resources-zone-control` §9.4 ("Recovery") reads "the out-of-band
  safety path (see §12.6)". That file's own §12 ends at §12.4; no §12.6
  exists in that document. **This spec is the out-of-band destructive-reset
  procedure §9.4 refers to.** See [Full Zone reset](#full-zone-reset-vs-provider-reset-vs-guest-reset)
  below, which is built directly on top of the exact primitive
  `ADR-046-resources-zone-control` §2.6 and §9.4 already define (`core.zone-drain`
  finalizer, `deletionRequestedAt`, reverse-dependency-order child deletion,
  final `phase=Deleted` event, store close, uid=0/local-OS-authenticated
  re-entry into bootstrap). The dangling `§12.6` citation should be corrected
  to point at this spec's Spec ID in a future non-content-bearing editorial
  pass; it is not a foundation decision and does not block acceptance of
  either document.
- `ADR-046-provider-volume-local` work item `ADR046-vl-005` ("TPM Volume")
  records "Data migration: None (full v3 reset; TPM NVRAM must be backed up
  by operator)". Read in isolation this appears to conflict with
  `ADR-046-provider-device-tpm` §17.3 ("Migration from current form"), which
  requires the existing `/var/lib/d2b/vms/<vm>/swtpm/` directory and its
  provisioning marker to be **migrated, not destroyed**, via a one-time
  migration `EphemeralProcess` that re-keys the marker. Both statements are
  correct and non-contradictory once scoped: `ADR046-vl-005`'s "full v3
  reset" describes the **volume-local Volume ResourceType implementation** —
  there is no row-level import of a prior `Volume` resource because no
  `Volume` resource existed before v3. "TPM NVRAM must be backed up by
  operator" is an **independent defense-in-depth recommendation**, not a
  license for this cutover to destroy TPM state. This spec is the binding
  authority: [Never wipe TPM identity or durable Volumes silently](#never-wipe-tpm-identity-or-durable-volumes-silently)
  makes host-file preservation of TPM NVRAM and its marker a **mandatory,
  automated, fail-closed** disposition of the cutover tool, on top of which
  an operator-performed backup remains good practice but is never assumed as
  the sole safety net.

## Terminology

| Term | Meaning |
| --- | --- |
| **Cutover** | The one-time, host-scoped, destructive procedure that replaces the pre-ADR-0046 control plane (daemon-only `d2bd`/`d2b-priv-broker`, Realm/Workload types, legacy Nix option namespace) with a live ADR 0046 Zone runtime. There is exactly one cutover per physical host (or per execution target that will host its own Zone runtime — see [Full Zone reset vs Provider reset vs Guest reset](#full-zone-reset-vs-provider-reset-vs-guest-reset)). |
| **Cutover snapshot** | The immutable, integrity-pinned, point-in-time capture of every current-baseline artifact this spec's [inventories](#authoritative-inventories) enumerate, taken at the start of [Preflight](#preflight-and-immutable-snapshot) before any mutation. Identified by a `checkpoint_id`. |
| **Disposition** | The single closed-set classification (`Adopt`, `Preserve`, or `Destroy`) this spec assigns to every current-baseline path, unit, process, and artifact. See [Disposition framework](#disposition-framework). |
| **Adopt** | The current artifact's durable bytes are moved, hardlinked, or re-keyed into a new v3-owned location (a Volume, a Zone bundle path, a re-rooted audit segment directory) while preserving identity, content, and any fail-closed provisioning marker. The old path is retired only after the new owner verifies adoption succeeded. |
| **Preserve** | The current artifact is left exactly where it is, unmodified, and continues to be read (never written) by legacy or transitional code until its owning ADR 0046 work item supplies a live successor. Nothing under Preserve is touched by cutover `apply`. |
| **Destroy** | The current artifact is deleted. Destroy is permitted **only** for regenerable, ephemeral, or cache-class data (§ [Old artifact/unit/schema removal gates](#old-artifactunitschema-removal-gates) enumerates the exact gate each Destroy candidate must clear). Destroy is never the default disposition; every row in the [migration/disposition matrix](#migrationdisposition-matrix) states its disposition explicitly — there is no implicit fallback to Destroy for an unlisted path. |
| **Full Zone reset** | The destructive re-initialization of one Zone's redb resource store: every non-`Zone` resource is deleted in reverse-dependency order, the store is closed, and Zone runtime re-enters compiled bootstrap authorization for a fresh initialization. Defined by `ADR-046-resources-zone-control` §2.6/§9.4; this spec is its authoritative out-of-band operational procedure (see [Full Zone reset vs Provider reset vs Guest reset](#full-zone-reset-vs-provider-reset-vs-guest-reset)). |
| **Provider reset** | Deletion and re-creation of exactly one `Provider/<name>` resource, its entire `ProviderStateSet` (every Volume it owns), and its component Process/EphemeralProcess children. Does not touch the Zone, other Providers, Hosts, Guests, or unrelated resources. |
| **Guest reset** | Deletion and re-creation of exactly one `Guest/<name>` resource and its owned children (its runtime Processes, its store-view/TPM Volumes unless explicitly retained). Does not touch the Zone, Providers, or other Guests. |
| **Cutover checkpoint** | One phase-boundary durable record in the [cutover journal](#crashpower-lossretryidempotency-journals), written with the ADR 0034 atomic-persistence sequence (temp file, `fsync`, rename, parent `fsync`). |
| **Incident hold (cutover-wide)** | An operator-set hold that blocks every destructive disposition step and the entire [old artifact/unit/schema removal gate](#old-artifactunitschema-removal-gates) sequence, built on the same `IncidentHold` condition semantics `ADR-046-provider-state` defines for a Volume, extended here to apply Zone-wide during a cutover window. |
| **Gateway Guest** | A Guest whose runtime Provider hosts a nested child Zone (a "gateway guest" in ADR 0032 terms) reachable only through a parent `ZoneLink`. The parent Zone never holds the gateway Guest's Credential, audit, or ZoneLink-internal session state — see [Gateway Guest credential/audit custody](#gateway-guest-credentialaudit-custody). |

Every term above composes with, and does not redefine, the shared vocabulary
in `ADR-046-terminology-and-identities` (Zone, ResourceRef, Provider, Host,
Guest, ExecutionPolicy, AuthenticatedSubjectContext) and the common resource
envelope in `ADR-046-resource-object-model` (metadata/spec/status,
`deletionRequestedAt`, finalizers, phase enum).

## Relationship to prior art

This spec is not a green-field design. It formalizes and generalizes an
existing, evidenced precedent:

| Prior art | What it establishes | What this spec adds |
| --- | --- | --- |
| [ADR 0034](../adr/0034-storage-lifecycle-restart-and-synchronization.md) "Migration decision" | One-time planned-downtime cutover permitted to move an existing host into a new storage layout; preserves swtpm NVRAM/markers, SSH keys, store-view state/gcroots, daemon adoption metadata, degraded/audit history, disk images; dry-run/apply modes; checkpoint id + rollback command printed before any apply step | Generalizes this exact preserve-list/checkpoint/dry-run model from "storage layout only" to the entire host control-plane cutover (daemon, wire protocol, Nix option namespace, resource plane, Providers, audit chain) |
| `packages/d2b/src/lib.rs` `cmd_host_migrate_storage`/`build_storage_migration_plan` (evidence class: `implemented-and-reachable`, retired per `ADR-046-cli-and-operations`) | Concrete `StorageMigrationPlan` JSON shape: `checkpointId`, `rollbackCommand`, `preflightRequirements`, `preserve`, `cutoverOnlyCleanup`, `failClosedHazards`, `applyStatus` | This spec's [CLI UX/exit codes/JSON plan](#cli-uxexit-codesjson-plan) reuses this exact JSON shape (renamed fields, extended scope) for `d2b host cutover plan` |
| `packages/d2b/src/lib.rs` `cmd_host_destroy` (evidence class: `implemented-and-reachable`) | Dry-run-first destructive host verb gated by `require_explicit_mutation_flag`, Tier-0-legacy refusal, daemon-down refusal | Reused verbatim as the `--apply` admission precondition pattern for `d2b host cutover apply` and `d2b host reset` |
| `ADR-046-resources-zone-control` §2.6, §9.4 | `core.zone-drain` finalizer; reverse-dependency-order child deletion; compiled bootstrap re-entry; uid=0/local-OS-authenticated out-of-band destructive reset | Adopted verbatim as the mechanism behind [Full Zone reset](#full-zone-reset-vs-provider-reset-vs-guest-reset) |
| `ADR-046-provider-state` "No bootstrap state Volume", "Cross-component migration coordination", "Incident hold", "Unclaimed Volume GC" | No bootstrap state-Volume exception (D086, superseded by D087); migration `EphemeralProcess` prepare/stage/commit/precommit-rollback/roll-forward-after-crash algorithm; per-Volume `IncidentHold` condition; unclaimed-Volume GC policy | Reused as the exact mechanical pattern for every Adopt-disposition row in the [migration/disposition matrix](#migrationdisposition-matrix) that moves bytes into a new Volume (TPM, store-view, disk images, unsafe-local scope state) |
| `ADR-046-nix-configuration` "Prior generation retention and pruning"; D066 | Per-Zone `retainedGenerations` (default 3, range 1..16), count-based (no TTL) pruning, eval-enforced bound | Reused verbatim for [Backup/retention count 1-16, no TTL](#backupretention-count-1-16-no-ttl) applied to cutover snapshots |
| `ADR-046-telemetry-audit-and-support` `d2b-audit` crate design | SHA-256 hash-chain JSONL, segment rotation, 30-day compaction, `AuditWriteClass`, `d2b zone audit export` | Reused for [Audit chain closure/opening](#audit-chain-closure-and-opening) |
| `tests/tools/preflight-disk-space.sh` (10 GiB floor, cited by `AGENTS.md` "Disk hygiene contract") | Fail-closed disk-space guard ordered before toolchain bootstrap | Reused as the model for [Disk-space/GC safety](#disk-spacegc-safety) |

## Cutover phases overview

```text
Phase 0  Preflight                    (read-only; §Preflight and immutable snapshot)
Phase 1  Consent + dry-run plan       (read-only; §Explicit operator consent and dry-run plan)
Phase 2  Authoritative inventories    (read-only; §Authoritative inventories)
Phase 3  Old daemon/unit/process drain(mutating: stop only; §Old daemon/unit/process drain)
Phase 4  Disposition execution        (mutating: Adopt/Preserve/Destroy; §Disposition framework)
Phase 5  Resource-store initialization(mutating: redb bootstrap; §Resource-store initialization)
Phase 6  Provider install/topological start (mutating; §Provider install/topological start)
Phase 7  Zone/ZoneLink cutover         (mutating; §Zone/ZoneLink cutover)
Phase 8  Guest/runtime/network/store view activation (mutating; §Guest/runtime/network/store view activation)
Phase 9  Post-cutover verification    (read-only; §Post-cutover verification)
Phase 10 Old artifact/unit/schema removal gate (mutating: Destroy only; §Old artifact/unit/schema removal gates)
```

Phases 0-2 are always read-only and safe to run repeatedly. Phase 3 is the
**point where the ADR 0034 continuation invariant is deliberately suspended**:
normal daemon/process restarts never sweep runtime state, but this one-time
cutover explicitly quiesces every legacy process before Phase 4 mutates
storage. Phase 4 is the last phase inside the [rollback boundary](#rollback-boundary):
`nixos-rebuild switch --rollback` remains a safe undo through the end of
Phase 4. From Phase 5 onward the resource-store bootstrap and Provider install
sequence commit durable state with no v2 representation to roll back to; the
only recovery path past that point is restoring from the [cutover
snapshot](#preflight-and-immutable-snapshot) via the [rollback boundary](#rollback-boundary)
contract. Phase 10 never runs automatically; it requires its own separate
consent and the [old artifact/unit/schema removal gates](#old-artifactunitschema-removal-gates)
to individually clear.

## Preflight and immutable snapshot

`d2b host cutover preflight` is read-only. It:

1. Refuses to run if a prior cutover snapshot for this host already has a
   `phase >= 5` checkpoint recorded (this host has already cut over; direct
   the operator to `d2b host reset` or `d2b host cutover doctor` instead).
2. Confirms baseline shape: `Realm`/`Workload` Nix option namespace present
   (`d2b.realms.*`), `d2bd.socket`/`d2bd.service`/`d2b-priv-broker.socket`/
   `d2b-priv-broker.service` units installed, no `d2b.zones.*` Zone runtime
   unit present yet.
3. Runs the [disk-space guard](#disk-spacegc-safety) before touching anything
   else, fail-closed on insufficient space, mirroring
   `tests/tools/preflight-disk-space.sh`'s ordering before toolchain
   bootstrap.
4. Builds the complete [authoritative inventories](#authoritative-inventories)
   in memory (read-only; no lock is taken yet).
5. Computes the cutover `checkpoint_id`:

   ```text
   checkpoint_id = "cutover-v1-" + hex(sha256(
       "cutover-v1\n"
     + sorted(guest_names).join("\n") + "\n"
     + sorted(storage_json_ids).join("\n") + "\n"
     + sorted(sync_json_ids).join("\n") + "\n"
     + sorted(swtpm_marker_ids).join("\n") + "\n"
     + sorted(framework_key_fingerprints).join("\n") + "\n"
   ))[..12]
   ```

   This follows the exact digest-of-sorted-names pattern
   `storage_migration_checkpoint_id` already uses in
   `packages/d2b/src/lib.rs`, extended to cover every inventory class in
   [Authoritative inventories](#authoritative-inventories) rather than VM
   names alone.
6. Writes the **immutable cutover snapshot**: a single JSON document at
   `/var/lib/d2b/cutover/<checkpoint_id>/snapshot.json`, created `0640
   root:d2bd`, containing every inventory row (path, owner, mode, size,
   content digest where the artifact is a regular file bounded under 4 MiB,
   or a directory-tree digest for larger trees such as disk images and the
   store-view farm), the resolved manifest digest, and the exact NixOS
   system-path being activated. The snapshot file is written with the ADR
   0034 atomic-persistence sequence (temp file in the same directory,
   `fsync`, rename over target, `fsync` parent directory) and is made
   immutable at the filesystem level (`chattr +i` where the filesystem
   supports it; otherwise `0440 root:root` with no writer principal granted
   write access) immediately after the atomic rename.
7. Prints the `checkpoint_id` and the exact snapshot path before returning.
   Nothing before this point mutates host state.

The snapshot is retained under the count-based retention policy in
[Backup/retention count 1-16, no TTL](#backupretention-count-1-16-no-ttl); it
is never referenced by content again except by the [rollback boundary](#rollback-boundary)
and [Failure/quarantine/manual recovery](#failurequarantinemanual-recovery)
procedures.

### Config/artifact/schema validation

Before the plan is offered to the operator, preflight independently validates
two configurations, because the cutover changes both the host's NixOS
configuration and the resource plane it activates:

1. **Legacy Nix configuration is currently green.** `nix flake check` (or the
   already-activated system's `bundle-<hash>.json` presence, whichever is
   available) must currently validate without error. Cutting over a host
   whose current configuration does not evaluate is refused with
   `cutover-precondition-failed` — this spec never repairs a pre-existing
   broken configuration.
2. **The candidate v3 Zone bundle validates independently.** The new
   `d2b.zones.<zone>.*` Nix configuration that will replace
   `d2b.realms.*`/`d2b.vms.*` is evaluated and its generation bundle built
   (per `ADR-046-nix-configuration` "Bundle and generation emission")
   *without activating it*. Schema validation includes: every
   `ResourceTypeSchema`/Provider settings schema referenced by the candidate
   bundle is present and its digest matches the signed catalog; every
   `Provider.spec.artifactId` resolves in `/etc/d2b/artifact-catalog.json`;
   every `Host`/`Guest`/`Volume`/`Network`/`Device` resource's cross-refs
   resolve inside the candidate bundle (no dangling `*Ref`); the canonical
   sorted rendered bundle's generation id is deterministic and reproducible
   (two evaluations of the same tree produce the same generation id, per
   `ADR-046-nix-configuration` build-time invariant). A bundle that fails any
   of these checks refuses preflight with `bundle-schema-mismatch` or
   `bundle-integrity-failure` and prints the exact offending field —
   preflight never proceeds on a bundle it cannot fully validate.
3. **Provider trust preflight.** Every Provider named by the candidate
   bundle's `Provider.spec.artifactId` values is checked against the trust
   rules in `ADR-046-provider-model-and-packaging` §"Trust": exact digest,
   trusted publisher/root epoch, valid signature, no emergency deny, accepted
   provenance/SBOM/license/vulnerability policy, exact package/API
   conformance attestation. A Provider that fails trust preflight blocks
   `cutover plan` from proceeding with that Provider installed; the operator
   may re-run preflight after removing or replacing the offending Provider
   from the candidate bundle.

## Explicit operator consent and dry-run plan

`d2b host cutover plan [--zone <zone>] [--json | --human]` is always a
dry-run: it never mutates state. It re-runs preflight (reusing an existing
unexpired `checkpoint_id` if inventories are unchanged, else computing a new
one) and renders the complete plan:

```json
{
  "command": "host cutover plan",
  "mode": "dry-run",
  "checkpointId": "cutover-v1-4f2a9c7b1e83",
  "snapshotPath": "/var/lib/d2b/cutover/cutover-v1-4f2a9c7b1e83/snapshot.json",
  "rollbackCommand": "d2b host cutover rollback --checkpoint cutover-v1-4f2a9c7b1e83",
  "consentPhrase": "I understand this destructively replaces the d2b control plane and accept the disposition table below",
  "guestCount": 2,
  "guests": ["corp-vm", "work-vm"],
  "preflightRequirements": [
    "legacy Nix configuration currently evaluates cleanly",
    "candidate v3 Zone bundle validates independently (schema, artifact catalog, cross-refs, deterministic generation id)",
    "every named Provider passes trust preflight",
    "10 GiB free under /var/lib/d2b and the store-view farm filesystem",
    "all Guests stopped",
    "d2bd.service and d2b-priv-broker.service stopped",
    "net VMs stopped; guest routing and dependent bridge traffic will be interrupted",
    "no incident hold currently active for this host"
  ],
  "disposition": {
    "adopt": ["... see migration/disposition matrix ..."],
    "preserve": ["... see migration/disposition matrix ..."],
    "destroy": ["... see migration/disposition matrix, Phase 10 only ..."]
  },
  "failClosedHazards": [
    "symlink or path traversal inside any adopted path",
    "foreign ownership markers on a d2b-managed path",
    "recursive operations traversing hardlink farms or mutating shared /nix/store inodes",
    "missing swtpm marker for a previously provisioned TPM Guest",
    "missing framework SSH host key for a previously provisioned Guest",
    "any candidate outside the generated storage/inventory root set",
    "any open d2b daemon, broker, runner, net VM, or Guest file descriptor",
    "any attempt to unlink a lock file rather than leaving it for reboot/tmpfs cleanup",
    "insufficient disk space for the largest disk-image or store-view adopt operation"
  ],
  "rollbackBoundary": "safe through end of Phase 4 (disposition execution); destructive and unrecoverable except via snapshot restore from Phase 5 onward",
  "applyStatus": "requires --consent matching consentPhrase and --apply"
}
```

`d2b host cutover apply --consent "<exact consentPhrase>" [--zone <zone>]
[--json | --human]` requires the operator to pass the **exact,
byte-for-byte** `consentPhrase` string printed by the preceding `plan`
invocation (bound to the plan's `checkpointId`, so a stale consent phrase from
an earlier plan is rejected with `cutover-consent-required` and a fresh
`checkpoint_id`). This mirrors `require_explicit_mutation_flag`'s existing
dry-run/apply precondition pattern in `packages/d2b/src/lib.rs`, extended
with a content-bound consent string because this operation is destructive
across an entire host rather than one flag-gated command. There is no
`--force`, no environment-variable bypass, and no non-interactive default
that supplies consent implicitly.

## Authoritative inventories

Phase 2 builds seven closed inventories. Every disposition row in the
[migration/disposition matrix](#migrationdisposition-matrix) is drawn from
exactly one of these enumerations; the matrix is not permitted to reference a
path this phase does not walk.

| Inventory | Source | Contents |
| --- | --- | --- |
| **Guest inventory** | Parsed manifest (`ManifestDocument::vms()`) | Every declared VM/net-VM name, env, `is_net_vm`, enabled component set (graphics/tpm/usbip/audio), `WorkloadProviderKind` |
| **Storage inventory** | `storage.json` (per ADR 0034, read via `BundleResolver.storage`) | Every declared storage id, path template, kind, owner/group/mode, persistence class, restart class |
| **Sync inventory** | `sync.json` (per ADR 0034, read via `BundleResolver.sync`) | Every declared lock id, path template, lock kind, allowed holders, acquisition order |
| **TPM inventory** | `/var/lib/d2b/vms/<vm>/swtpm/` + `/var/lib/d2b/swtpm-markers/<vm>` for every Guest with `tpm.enable = true` | Marker presence/content digest, NVRAM directory tree digest, `previously-provisioned` fail-closed state |
| **Key inventory** | `<keysDir>/<vm>_ed25519{,.pub}` for every Guest; `<stateDir>/vms/<vm>/host-keys/{host.pub,user-authorized-keys}` | Fingerprint, mode/owner, staged host-keys content |
| **Disk-image inventory** | Guest-declared disk images and writable store-overlay images under `/var/lib/d2b/vms/<vm>/` | Path, size, content digest (tree digest for large images uses a sampled/streaming digest, never a full read into memory) |
| **Network inventory** | Declared host bridges, TAP naming intent, nftables `inet d2b` table ownership markers, NetworkManager/`systemd-networkd` coexistence markers | Bridge names, ownership-marker `comment "d2b managed: <ownership-id>"` values, NM unmanaged-config presence |
| **Audit/degraded inventory** | `daemon-events-*.jsonl` (daemon), broker `audit.rs` segments, `d2b-gateway-runtime` `audit_jsonl.rs` segments (gateway-backed realms only), daemon degraded ledger | Segment file list, last `record_hash`, degraded-ledger entry count by class |
| **Credential/identity inventory** | `realm-controllers.json`, `realm-identity.json`, `~/.local/state/d2b/unsafe-local-scopes.json` per user | Realm controller placement records, identity/key-rotation metadata, unsafe-local scope ledger entries |

Every inventory entry that this spec's [migration/disposition
matrix](#migrationdisposition-matrix) does not explicitly list is treated as
**Preserve by default** for Phase 4 (never mutated by this cutover) and is
flagged in the plan output under a `unclassified` array so the operator sees
it before consenting — there is no silent Destroy for anything outside the
matrix.

## Old daemon/unit/process drain

Phase 3 is the only phase permitted to suspend the ADR 0034 continuation
invariant ("do not clear runtime state just because a daemon process
restarted"). It executes, in order, and refuses to proceed past any step that
does not reach the expected quiesced state within its bounded deadline
(default 120 s per step, operator-configurable, never silently extended):

1. **Stop every Guest and net VM.** Uses the existing graceful-shutdown path
   ([ADR 0040](../adr/0040-graceful-vm-shutdown.md)); a Guest that does not
   reach a stopped state within its shutdown deadline aborts the drain with
   `cutover-precondition-failed` naming the stuck Guest — cutover never force
   kills a Guest that might hold unflushed durable Volume state.
2. **Stop `d2bd.service`.** The daemon relinquishes `public.sock`.
3. **Stop `d2b-priv-broker.service`.** The broker relinquishes `broker.sock`
   and any adoptable-runner delegated cgroup leaves it still supervises.
4. **Stop any gateway-backed realm's local `d2bd`/broker pair** (ADR 0032
   gateway guests are themselves Guests and are already stopped by step 1;
   this step additionally confirms no residual `d2b-gateway-runtime` process
   remains attached to a stopped gateway Guest's vsock CID).
5. **Verify quiescence.** Re-read `/proc` for any process matching a
   `d2b-<vm>-*` cgroup leaf under `d2b.slice`; any live process here after
   steps 1-4 aborts the drain with `cutover-precondition-failed` rather than
   being killed — a live, unaccounted-for process is exactly the "any open
   d2b daemon, broker, runner, net VM, or Guest file descriptor" fail-closed
   hazard from the dry-run plan.
6. **Boot-scoped runtime cleanup, cutover-only.** Only after step 5 confirms
   quiescence: remove boot-scoped runtime socket files under `/run/d2b-gpu`,
   `/run/d2b-video`, `/run/d2b-wlproxy`, and `/var/lib/d2b/guest-control-<vm>`
   (these are the same `cutoverOnlyCleanup` candidates
   `build_storage_migration_plan` already names). Lock files under `/run/d2b`
   are **never** unlinked here — they are left for the normal reboot/tmpfs
   cleanup path, exactly as ADR 0034's `fail_closed_hazards` already
   requires.

After step 6, this host has no live d2b process. Phase 4 begins.

## Disposition framework

Every path, unit, process, and artifact this spec's inventories enumerate
receives exactly one of three dispositions. The framework — not any single
matrix row — is the normative contract; the [migration/disposition
matrix](#migrationdisposition-matrix) is its application to the concrete
baseline evidence.

### Adopt

Adopt moves durable bytes into a new v3-owned location while preserving
content, identity, and any fail-closed provisioning marker. Every Adopt
disposition:

1. is executed by a dedicated, idempotent migration `EphemeralProcess` using
   the exact prepare/stage/commit/precommit-rollback/roll-forward-after-crash
   algorithm `ADR-046-provider-state` "Cross-component migration coordination"
   already defines — this spec introduces no second migration state machine;
2. never deletes the source path until the destination Volume/artifact
   reports `phase: Ready` (or the equivalent terminal success state for a
   non-Volume artifact, such as an audit segment directory) **and** the
   migration `EphemeralProcess` itself reports `Succeeded`;
3. re-validates the source's provisioning marker (where one exists, such as
   the swtpm marker) before adoption and refuses the specific adopt step with
   a fail-closed condition rather than silently re-provisioning if the marker
   is absent, replaced, or fails identity verification — the marker check is
   identical to the existing `previously-provisioned-swtpm-state-missing`
   detection in `d2b-priv-broker/src/ops/swtpm_dir.rs`, generalized to every
   marker-bearing Adopt candidate;
4. writes the new marker/identity record at the destination **before**
   reporting the migration `EphemeralProcess` `Succeeded`, so a crash between
   destination-write and old-marker-removal always resolves toward "old path
   still present, new path present and valid" (safe to re-run) rather than
   "neither path has a valid marker" (data loss);
5. leaves the source path in place, unmodified, until the corresponding
   [old artifact/unit/schema removal gate](#old-artifactunitschema-removal-gates)
   clears — Adopt is never combined with an immediate Destroy of its own
   source in the same phase.

### Preserve

Preserve leaves an artifact exactly where it is. A Preserved path:

- is read (never written) by any transitional or legacy code that still
  needs it until its owning ADR 0046 work item supplies a live successor;
- is never included in Phase 4's mutation set;
- appears in the cutover snapshot for completeness and audit, but its content
  digest is not re-verified after cutover (it was never touched).

`nixos-modules/host-activation.nix`, `nixos-modules/host-users.nix`,
`/etc/d2b/privileges.json`, and every path the [migration/disposition
matrix](#migrationdisposition-matrix) marks `RETAIN`/`RETAIN-ADAPT` in
`ADR-046-current-code-migration-map` fall under Preserve for the duration of
this cutover: they continue to exist at their current path and are only
edited in place by a later Nix/Rust work item, never by this cutover's Phase 4.

### Destroy

Destroy deletes an artifact. Destroy is the disposition of last resort and is
gated as follows:

- Destroy is **never** assigned to any path this spec classifies as TPM
  identity state or `kind: durable` Volume-equivalent user data (see [Never
  wipe TPM identity or durable Volumes silently](#never-wipe-tpm-identity-or-durable-volumes-silently));
- every Destroy candidate must be regenerable (a cache, a stale lock file
  already covered by the reboot/tmpfs path, a boot-scoped runtime socket) or
  have a live, tested successor already adopted and verified (an old
  artifact/unit/schema that a new resource/Provider/systemd-unit has already
  replaced);
- Destroy for any current-baseline artifact other than the Phase 3
  boot-scoped runtime sockets is deferred to Phase 10 — see [Old
  artifact/unit/schema removal gates](#old-artifactunitschema-removal-gates)
  — and never runs inside Phase 4's initial disposition execution;
- every Destroy is logged to the cutover journal with the exact path/unit
  name, the gate that cleared it, and the checkpoint id, before the delete
  syscall executes.

## Never wipe TPM identity or durable Volumes silently

This is the single hardest invariant in this spec, and it is fail-closed in
both directions: cutover must neither destroy this data nor proceed as if it
had been safely handled when it was not.

1. **TPM identity.** Every Guest with `tpm.enable = true` at baseline has its
   swtpm NVRAM tree and provisioning marker (`/var/lib/d2b/vms/<vm>/swtpm/`,
   `/var/lib/d2b/swtpm-markers/<vm>`) **Adopted**, never Destroyed, using the
   exact one-time migration `EphemeralProcess` `ADR-046-provider-device-tpm`
   §17.3 already specifies: the swtpm directory is migrated to the
   controller-created TPM Volume path, and the existing provisioning marker
   is preserved and re-keyed by `Provider/volume-local` from its old basename
   to the new `device_uid`-based name. A missing or already-dropped marker at
   preflight time fails cutover `plan` closed with
   `cutover-precondition-failed` naming the affected Guest — cutover refuses
   to proceed rather than silently re-provisioning a fresh, identity-less TPM
   for that Guest. This is the binding resolution of the apparent
   `ADR046-vl-005` vs. device-tpm §17.3 tension recorded in [Cross-reference
   and evidence corrections](#cross-reference-and-evidence-corrections): the
   operator-backup recommendation in `ADR046-vl-005` is additional, not a
   substitute for this automated Adopt.
2. **Durable Volumes.** Every current-baseline path this spec's inventories
   classify as user-declared persistent data — Guest disk images (including
   writable store-overlay images), any path a future `Volume` resource will
   declare with `kind: durable` per `ADR-046-resources-volume` §"Kind" — is
   **Adopted**, never Destroyed. There is no disposition path in this spec
   that deletes a `kind: durable`-equivalent artifact; the [Disposition
   framework](#disposition-framework)'s Destroy gate explicitly excludes this
   class.
3. **Fail-closed default.** If preflight cannot classify a path with
   confidence as ephemeral/cache/regenerable, its default disposition is
   **Preserve**, never Destroy. Ambiguity resolves toward keeping data, not
   discarding it.
4. **Independent operator backup remains recommended, never assumed.** Every
   `plan` output includes an advisory line recommending an out-of-band backup
   of TPM NVRAM and any disk image before `apply`, consistent with
   `ADR046-vl-005`. Cutover's own automated Adopt behavior does not depend on
   the operator having taken that advice; it is defense-in-depth on top of,
   not a substitute for, the automated preservation this section mandates.
5. **Verification is mandatory, not advisory.** [Post-cutover
   verification](#post-cutover-verification) re-reads every adopted TPM
   marker and every adopted durable Volume's content digest against the
   cutover snapshot and fails the cutover (without touching the still-present
   Preserved source) if any digest does not match.
6. **Entra identity-Guest login state (D093).** The Entrablau-enabled identity
   Guest owns its machine credential, TPM binding, enrollment, and
   refresh-token/private login state (guest-local, secret/large/private). This
   state is treated exactly like TPM identity: it is **Preserved/Adopted**, never
   silently wiped. A Guest, Provider, or Zone reset either preserves the identity
   Guest's TPM/login state or **explicitly destroys** it under the operator's
   authorized destructive disposition — never as an implicit side effect. A
   subsequent login re-enrolls only if the operator explicitly destroyed that
   state.

## Gateway Guest credential/audit custody

ADR 0032 establishes that a gateway-backed realm's relay credentials, remote
node registry, provider configuration, and audit trail live **only** inside
its dedicated gateway guest VM; the host never holds them
(`CredentialCustody::None` for host-local vs.
`CredentialCustody::GatewayGuest` for relay-backed, per
`d2b-realm-router/src/service_v2.rs`, and `ADR-046-zone-routing` §3's
`CredentialCustody` evidence). This cutover preserves that boundary exactly,
translated into ZoneLink terms:

1. A pre-cutover gateway-backed realm becomes, post-cutover, a Guest whose
   runtime Provider hosts a **nested child Zone**, reachable from the parent
   only through a `ZoneLink/<name>` resource in the parent Zone (per
   `ADR-046-resources-zone-control` §3 and `ADR-046-terminology-and-identities`
   "Zone").
2. The cutover's [inventories](#authoritative-inventories) and [migration/
   disposition matrix](#migrationdisposition-matrix) **never** enumerate the
   gateway guest's internal relay credentials, remote node registry, or
   realm audit log as host-side artifacts to Adopt, Preserve, or Destroy from
   the parent host's perspective — those live inside the gateway Guest's own
   filesystem and are the gateway Guest's own (nested) cutover's
   responsibility when that Guest itself boots a v3 Zone runtime internally.
   The parent host's cutover only Adopts the parent-visible `ZoneLink`
   configuration (transport provider selection, `childZoneName`) as ordinary
   Nix-authored resource configuration — never as migrated credential bytes.
3. The parent Zone's ZoneLink handler never receives, stores, or logs the
   child Zone's Credential resources, matching
   `ADR-046-componentsession-and-bus` "Sensitive credential delivery": a
   Credential Provider may deliver raw token bytes only over a dedicated KK
   ComponentSession to an authorized consumer, and intermediaries (including
   a parent Zone's ZoneLink handler) forward opaque protected records without
   decrypting them.
4. If the gateway Guest itself is being cut over from a pre-ADR-0046
   `d2b-gateway-runtime` process to a nested v3 Zone runtime, that nested
   cutover is a **separate, independent invocation** of this same procedure,
   run *inside* the Guest (its own Host in ADR 0046 terms), with its own
   snapshot, checkpoint id, and consent — it is never folded into the parent
   host's single `checkpoint_id` and never shares a cutover journal with the
   parent.
5. Audit: the parent Zone's audit stream never records a gateway Guest's
   internal realm audit events; the parent only records its own ZoneLink
   connect/reconnect/disconnect/route events per
   `ADR-046-componentsession-and-bus` "Errors and telemetry".

## ProviderStateSet and state schema migration policy

Every Provider component's durable payload state is governed entirely by
`ADR-046-provider-state`; this spec does not define a second state model.
Provider state Volumes are optional and declared only under the storage-need
test — bounded non-secret operational state lives in resource `status` and the
core Operation ledger by default (D087). Two distinctions matter specifically
for cutover:

1. **First-bootstrap components never have a "from" schema.** For every
   Provider installed during [Provider install/topological start](#provider-installtopological-start),
   any *declared* component state Volume is created fresh by Core
   ProviderDeployment with `stateSchemaPhase: current` immediately (there is no
   `migration-required` phase to resolve, because `installedSchemaVersion`
   equals `spec.stateSchema.schemaVersion` at creation by construction).
   Cutover never invokes `ADR-046-provider-state` "Schema migration"
   §"Pre-launch migration"/"Online migration" for a freshly created Provider;
   those mechanisms exist for a **later** Provider software upgrade, not for the
   initial cutover.
2. **Cutover-adopted state Volumes.** The TPM Volume and store-view Volume
   created during [disposition execution](#disposition-framework) are created
   by the Adopt migration `EphemeralProcess` itself, which writes both the
   layout content (from the adopted source) and the identity marker before the
   Volume is marked `Ready`. Once created, they participate in the exact same
   ownership, quota, sealing, snapshot, retention, incident-hold, and
   unclaimed-GC machinery as any other declared Provider state Volume — cutover
   does not create a special-cased Volume subtype.

The optional `ProviderStateSet` query itself
(`ProviderStateSet(zone, provider-name) = { v : Volume | v.metadata.zone ==
zone && v.metadata.ownerRef == "Provider/<provider-name>" }`) requires no
cutover-specific extension: once Phase 6 installs a Provider, its
`ProviderStateSet` is exactly the set of *declared* Volumes Core
ProviderDeployment (or, for the Adopt cases above, the Adopt migration
`EphemeralProcess`) created for it — possibly empty — queryable the same way at
any later time.

## Resource-store initialization

Phase 5 creates the Zone's redb resource store from nothing, per
`ADR-046-resource-store-redb` "Backup, restore, and physical schema upgrade"
and work item `ADR046-store-003` ("Data migration: Destructive v3 bootstrap;
v3-to-v3 logical restore"). There is no logical import of any pre-cutover
daemon state, Realm/Workload representation, or legacy JSON artifact into the
redb store — the store's `store_meta` table is populated fresh:

```text
store_uuid                 = freshly generated
zone_name                  = <zone> (from --zone or the candidate bundle's sole Zone)
zone_uid                   = freshly generated
created_at                 = now() (RFC 3339 UTC)
schema_version              = current physical schema version
current_revision            = 0
compaction_floor             = 0
active_configuration_revision = 0 (set to 1 after Phase 7 activates the first generation)
policy_revision              = 0 (set after bootstrap RBAC publishes, per Phase 6)
api_catalog_revision          = 0 (set after the initial ResourceType catalog binds)
clean_shutdown               = true
backup_generation            = none
```

Phase 5's only input from the pre-cutover host is the **already-adopted**
bytes from Phase 4 (TPM/store-view/disk-image Volumes) and the **validated
candidate bundle** from preflight — never a raw read of `d2bd`'s in-memory
state, `realm-controllers.json`, or `realm-identity.json` (those remain
Preserved on disk per the [migration/disposition matrix](#migrationdisposition-matrix)
until their owning work item retires them, but their content is not imported
into redb rows).

Immediately after store creation, Zone runtime follows the exact 9-step
startup sequence `ADR-046-core-controllers` "Startup" already defines:

1. Zone runtime validates/opens the freshly created store.
2. Resource service and local d2b-bus/ComponentSession endpoint start.
3. Fixed `system-core` and `system-minijail` controller processes start and
   authenticate as their exact Provider subjects.
4. Compiled bootstrap authorization (per `ADR-046-resources-zone-control` §9)
   grants only the closed verb set to exactly `Provider/system-core` and
   `Provider/system-minijail`.
5. Handlers list/recover/checkpoint concurrently (trivially empty on first
   boot).
6. Configuration publishes the first candidate bundle as generation 1.
7. `system-core` reconciles the first `Host` and local `User` set.
8. Other Provider controllers/processes launch through resources (Phase 6).
9. Zone readiness publishes after mandatory handlers are current.

## volume-local reaches Ready without a state Volume

Step 8 above depends on `Provider/volume-local` being able to create the
*declared* state Volumes for other Providers — but `volume-local`'s own
controller instance declares no state Volume: its bounded non-secret
operational state lives in resource `status` and the core Operation ledger
(D087). Because the fixed bootstrap components (`volume-local`, `system-core`,
`system-minijail`) declare no state Volume, no component needs a Volume before
`volume-local` is Ready, so cutover relies on no bootstrap state-Volume cycle,
no per-execution-target local bootstrap storage mechanism, and no
bootstrap-storage exception (D086, superseded by D087; see "No bootstrap state
Volume" in `ADR-046-components-processes-and-sandbox`):

- the first `Provider/volume-local` controller instance on the Host reaches
  `Ready` from Host-local primitives and its own `status` alone, then begins
  creating the declared state Volumes of later Providers;
- a Guest boots its own `volume-local` instance from Guest-local primitives,
  without any parent-Host dirfd or resource handle, and that instance likewise
  reaches Ready from its own status;
- the TPM/store-view Adopt Volumes described above are created later, in
  Phase 4/8, by their owning migration `EphemeralProcess` and Provider
  (`device-tpm`, `runtime-cloud-hypervisor`), strictly after `volume-local`
  has already reached readiness — they are ordinary
  Core-ProviderDeployment-adjacent declared Volumes.

Cutover's only responsibility here is sequencing: Phase 5 step 8 (Provider
install) must not attempt to create any Provider's declared component state
Volume before `volume-local`'s own controller Process has reached `Ready` on
the target it will serve — this is naturally satisfied because
`Provider/volume-local` is itself the very first non-bootstrap Provider
installed in [Provider install/topological start](#provider-installtopological-start).

## Provider install/topological start

After Zone startup step 7 (`system-core` reconciles the first `Host` and
`User` set), step 8 installs every remaining Provider named by the candidate
bundle in dependency order. Provider dependencies are declared as manifest
aliases (`runtime`, `volume`, `network`, `credential`, `transport`, per
`ADR-046-provider-model-and-packaging` "Provider dependencies"); the
configuration-publication handler resolves this into a strict partial order
before any Process is created:

1. Build a dependency graph: one node per `Provider/<name>` in the candidate
   bundle, one edge `A -> B` for every alias in `A`'s manifest that Zone
   configuration binds to Provider `B`.
2. Reject the candidate bundle at Phase 5/6 boundary (before any Process
   exists) if this graph contains a cycle — `ADR-046-provider-model-and-packaging`
   "Provider dependencies" already requires synchronous dependency cycles to
   fail configuration; cutover enforces this before Phase 6 begins, not
   after a partial install.
3. Install Providers in reverse-topological order (dependencies before
   dependents), fixed at these staged priorities so that Providers without
   declared cross-dependencies still install deterministically:

   ```text
   Stage A (fixed bootstrap; already running before Phase 6 begins)
     Provider/system-core
     Provider/system-minijail

   Stage B (storage/process substrate; no Provider dependency of its own)
     Provider/volume-local
     Provider/system-systemd

   Stage C (attachment/fabric Providers; depend only on Stage A/B)
     Provider/volume-virtiofs
     Provider/network-local
     Provider/device-tpm
     Provider/device-usbip
     Provider/device-security-key
     Provider/device-gpu
     Provider/transport-unix
     Provider/transport-vsock
     Provider/transport-azure-relay
     Provider/credential-secret-service
     Provider/credential-entra
     Provider/credential-managed-identity

   Stage D (execution Providers; depend on Stage B/C for volume/network/device/credential/transport aliases)
     Provider/runtime-cloud-hypervisor
     Provider/runtime-qemu-media
     Provider/runtime-azure-container-apps
     Provider/runtime-azure-virtual-machine
     Provider/activation-nixos
     Provider/observability-otel

   Stage E (interaction Providers; depend on Stage D Guests/Hosts being reconcilable)
     Provider/display-wayland
     Provider/audio-pipewire
     Provider/clipboard-wayland
     Provider/notification-desktop
     Provider/shell-terminal
   ```

   Optional declared dependencies within a stage that are absent from the
   candidate bundle produce declared degraded behavior for the dependent
   Provider (per `ADR-046-provider-model-and-packaging` "Provider
   dependencies"), never an install failure — the fixed staging above is a
   deterministic default ordering for Providers with no explicit
   cross-dependency edge, not a hard requirement that every stage's Providers
   be present.
4. Within a stage, Providers install in parallel; installs across stages are
   strictly ordered by Provider lifecycle handler readiness (a Stage C
   Provider's `Ready` phase gates Stage D Providers that declare it as a
   dependency alias).
5. Each Provider install follows `ADR-046-core-controllers` "Provider
   lifecycle" handler algorithm unmodified: verify package/trust/config/
   conformance, validate controller/service/worker graph, create owned
   Volume/Process/EphemeralProcess children (subject to [ProviderStateSet
   and state schema migration policy](#providerstateset-and-state-schema-migration-policy)
   above), wait for required dependencies, publish exported ResourceTypes/
   services only after ready.

Cutover introduces no new Provider-install algorithm; it only fixes the
deterministic default stage order above so that a cutover `plan` can print a
complete, reproducible install sequence before `apply` runs.

## Zone/ZoneLink cutover

1. The candidate bundle's Zone becomes the `Zone/<zone-name>` self resource at
   Phase 5 step 1 (redb store creation). There is exactly one Zone per cutover
   invocation; a multi-Zone host runs one independent cutover invocation per
   Zone (this spec's `checkpoint_id` is always Zone-scoped, never host-wide
   when a host is declared to run more than one Zone).
2. Any pre-cutover realm that used `EntrypointMode::GatewayBacked` becomes a
   `ZoneLink/<name>` resource in the parent Zone, created during Phase 6/7 as
   ordinary Nix-authored configuration (per [Gateway Guest credential/audit
   custody](#gateway-guest-credentialaudit-custody)) — never as migrated
   session or credential state. The `ZoneLink`'s `transportProviderRef` must
   name an already-`Ready` Stage C transport Provider (per [Provider
   install/topological start](#provider-installtopological-start)); a
   `ZoneLink` whose transport Provider is not yet Ready enters `Degraded`
   with `ConfigurationCurrent: False` until that Provider reconciles, exactly
   as `ADR-046-nix-configuration` "Cross-Zone generation ordering" already
   specifies — Zone A's own activation is never blocked on Zone B/ZoneLink
   readiness.
3. Any pre-cutover realm that used `EntrypointMode::HostResident` becomes an
   ordinary `Guest` (VM/sandbox) or a user-only `Host` (unsafe-local, per
   D042) in the parent Zone — never a `ZoneLink`, because a host-resident
   realm never had a separate resource store to link to.
4. ZoneLink activation completes when the core `zone link/delegation` handler
   (`ADR-046-core-controllers` "Zone link/delegation") reports the link
   `Ready` — verified in [Post-cutover verification](#post-cutover-verification).

## Guest/runtime/network/store view activation

Phase 8 brings up every declared `Guest` in dependency order derived from its
own resource refs, following the same reverse-topological principle as
Provider install:

1. **Network first.** `Provider/network-local` reconciles every declared
   `Network` resource (bridges, DHCP/DNS/NAT/firewall) before any Guest that
   attaches to it is started.
2. **Volume/store-view/TPM adopt confirmation.** For every Guest whose
   store-view or TPM Volume was created in Phase 4's disposition execution,
   `Provider/volume-local` must report that Volume `Ready` (marker verified,
   quota enforced) before the owning runtime Provider is permitted to start
   that Guest's Process — a Guest never starts against a Volume still in
   `Pending`/`Degraded`.
3. **Device attachment.** `Provider/device-tpm`, `device-usbip`,
   `device-security-key`, `device-gpu` reconcile their declared attachments
   for each Guest, gated on the Volume readiness in step 2 where the device
   is TPM-backed.
4. **Guest runtime start.** The Guest's runtime Provider
   (`runtime-cloud-hypervisor`, `runtime-qemu-media`,
   `runtime-azure-container-apps`, or `runtime-azure-virtual-machine`) starts
   the Guest's Process(es) only after steps 1-3 report `Ready` for every
   resource that Guest's `ExecutionPolicy` attachment defaults name.
5. **Store view.** The per-Guest `/nix/store` hardlink farm (adopted from
   `/var/lib/d2b/vms/<vm>/store/` in Phase 4) is mounted read-only into the
   Guest by the owning virtiofsd Process, which `Provider/volume-local`
   supervises per `ADR-046-provider-volume-local` ADR046-vl-004 — unchanged
   from the current same-filesystem hardlink-farm contract in
   `nixos-modules/store.nix`/ADR 0027, only its activation path moves from
   Nix activation to the Volume controller.

Any Guest that fails to reach `Ready` within its per-Guest deadline during
Phase 8 does not block other Guests — per `ADR-046-resource-reconciliation`
"Process fast path", independent resources reconcile/start concurrently. That
Guest's `Degraded`/`Failed` status is surfaced individually in [Post-cutover
verification](#post-cutover-verification) and does not abort the cutover for
every other Guest.

## Post-cutover verification

`d2b host cutover verify [--zone <zone>] [--json | --human]` is read-only and
checks, in order, refusing to report success unless every check passes:

| # | Check | Failure condition |
| --- | --- | --- |
| 1 | Zone `status.phase == Ready` | Any mandatory core-controller handler not `Ready` |
| 2 | Every Stage A/B/C Provider from the candidate bundle reports `status.phase == Ready` | Any mandatory Provider `Degraded`/`Failed`/`Unknown` |
| 3 | Every declared `Guest` reports `status.phase == Ready` (or the operator-accepted subset, if `--allow-degraded-guests <list>` was explicitly passed to `apply`) | Any Guest not `Ready` outside the accepted subset |
| 4 | Every adopted TPM marker's content digest matches the cutover snapshot's recorded digest | Digest mismatch on any TPM Volume |
| 5 | Every adopted durable Volume's (disk image, store-view) content digest matches the cutover snapshot's recorded digest | Digest mismatch on any durable Volume |
| 6 | Every declared `ZoneLink` reports `Ready` or an accepted `Degraded/waiting-on-remote` (per Cross-Zone generation ordering) | `ZoneLink` in any other Degraded/Failed condition |
| 7 | Every declared `Network` reports `Ready` | Any Network Degraded/Failed |
| 8 | No orphaned pre-cutover process remains under any `d2b-<vm>-*`/`d2b.slice` cgroup leaf outside the current Zone's own cgroup partition | Any orphan found |
| 9 | The new audit chain's first record verifies against the closure record of the old chain (per [Audit chain closure and opening](#audit-chain-closure-and-opening)) | Hash-chain break or missing closure record |
| 10 | `d2b host cutover doctor` reports zero entries in the [degraded-state ledger](#failurequarantinemanual-recovery) tagged `cutover-quarantined` | Any such entry present |

`verify` failing any check does **not** automatically roll back — the
[rollback boundary](#rollback-boundary) has already closed by this point
(verification only runs after Phase 8). A failed `verify` transitions the
host into [Failure/quarantine/manual recovery](#failurequarantinemanual-recovery)
instead.

## Rollback boundary

Cutover has exactly one rollback boundary, and it is stated once here so no
other section needs to restate it:

| Point | What "rollback" means | Mechanism |
| --- | --- | --- |
| Before Phase 1 (`plan`) | No state changed; nothing to roll back | N/A |
| End of Phase 3 (drain complete) | Configuration/binary rollback: undo the NixOS activation that installed the new units, restart the old `d2bd`/`d2b-priv-broker` units, restart Guests | `nixos-rebuild switch --rollback` to the pre-cutover system generation, then `systemctl start d2bd.service d2b-priv-broker.service`, then normal Guest start |
| End of Phase 4 (disposition execution complete) | **Last safe rollback point.** Adopted bytes exist at both old and new locations (old path not yet removed by any gate); configuration/binary rollback is still possible because the old daemon/unit set has not been destroyed, only stopped | Same as above; the cutover journal (below) records that Phase 4 completed so a resumed/re-run cutover does not re-adopt already-adopted paths |
| Phase 5 onward | **No rollback.** The redb resource store now holds committed state with no v2 representation to revert to; Provider installs and Guest starts have created new process/cgroup/Volume state that a binary rollback cannot cleanly unwind | Recovery is **restore from the cutover snapshot**, not rollback: stop the new Zone runtime, `d2b host cutover rollback --checkpoint <id>` re-runs Phase 3's drain against the *new* control plane, restores the pre-cutover NixOS generation, and restarts the old daemon/broker against the still-Preserved and still-Adopted (never Destroyed, because Phase 10 has not run) durable paths |
| After Phase 10 clears any gate | **Fully irreversible** for whatever that gate destroyed. `d2b host cutover rollback` after Phase 10 refuses with `cutover-rollback-window-closed` and directs the operator to the cutover snapshot for forensic recovery only (the snapshot's content digests, not a live restore, since the artifact itself is gone) | N/A — this is why Phase 10 requires its own separate consent and never runs automatically |

`d2b host cutover rollback --checkpoint <checkpoint_id> [--json | --human]`
is only valid while the journal (below) shows the named checkpoint's last
completed phase is `<= 4`. It:

1. Re-drains the new Zone runtime using the same algorithm as [Old
   daemon/unit/process drain](#old-daemonunitprocess-drain), applied to the
   new units instead of the old;
2. Restores the pre-cutover NixOS system generation
   (`nixos-rebuild switch --rollback`, or the equivalent `activation-nixos`
   rollback once that Provider itself is live for a later, non-initial
   cutover);
3. Restarts `d2bd.service`/`d2b-priv-broker.service`;
4. Restarts every Guest from its Preserved/Adopted-but-not-yet-Destroyed
   state — since Phase 10 has not run, every old path this rollback needs is
   still present.

Rollback past Phase 4 (i.e., against a checkpoint whose last completed phase
is `>= 5`) is refused with `cutover-rollback-window-closed`; the typed error
message names the exact phase reached and points at the cutover snapshot path
for manual forensic recovery.

## Removed configuration async finalizer cleanup

The candidate v3 Zone bundle activated in Phase 5 step 6 is generation 1 for
this Zone — there is no "prior generation" to clean up on the very first
activation. This section exists to state precisely that the [generation-based
cleanup contract](../specs/ADR-046-nix-configuration.md#resource-cleanup-contract)
(`managedBy=configuration` diff-and-async-Delete, `PendingCleanup`/`Degraded`
status, finalizer-safe, non-blocking activation, count-based
`retainedGenerations` retention) applies to this cutover's Zone bundle
**exactly the same way it applies to every later configuration change** —
cutover does not special-case generation 1:

- generation 1's `managedBy=configuration` resource set is exactly the
  candidate bundle validated in preflight;
- there is no absent-resource cleanup to enqueue on generation 1's own
  activation (there is no generation 0 to diff against);
- the very next `nixos-rebuild switch` after cutover that changes the Zone's
  Nix configuration activates generation 2 and runs the ordinary cleanup
  contract unmodified;
- `retainedGenerations` (default 3, range 1..16, per D066) begins counting
  from generation 1 exactly as it would for any Zone.

This section is deliberately short: its entire content is "the existing
contract applies unmodified starting from generation 1," which is itself the
normative statement removing any ambiguity about whether cutover needs a
separate cleanup path for its own bundle. It does not.

## Crash/power-loss/retry/idempotency journals

Every phase boundary writes one durable **cutover journal checkpoint** using
the ADR 0034 atomic-persistence sequence, at
`/var/lib/d2b/cutover/<checkpoint_id>/journal.jsonl` (append-only, one JSON
object per line, `O_APPEND | O_CREAT`, `fsync` after every append):

```json
{"phase": 4, "step": "adopt-tpm-corp-vm", "outcome": "succeeded", "at": "2026-07-22T00:03:11Z"}
```

Idempotency rules, applied uniformly to every phase:

1. **Re-running `apply` after a crash resumes, never restarts.** On start,
   `apply` reads the journal for the named `checkpoint_id`, determines the
   highest phase/step with a recorded `succeeded` outcome, and resumes
   immediately after it. A step recorded `started` but never `succeeded`/
   `failed` (crash mid-step) is re-executed from its own beginning — every
   Adopt step's migration `EphemeralProcess` is required (per [Disposition
   framework](#disposition-framework)) to be idempotent and safe to re-run
   for exactly this reason, mirroring `ADR-046-provider-state` "Migration
   operator requirements": deterministic, idempotent, roll-forward only.
2. **Power loss during an Adopt step's atomic file-level operation** resolves
   the same way `ADR-046-provider-state` "Roll-forward after interrupted
   commit" already specifies: on restart, if the destination marker/content
   is already valid, the step is marked `succeeded` retroactively (the write
   completed; only the journal append did not) and the source is left in
   place for the normal removal gate; if the destination marker is absent or
   invalid, the step re-executes from its own beginning against the
   still-present source.
3. **Phase 5 (resource-store initialization) is the one phase that is not
   naturally idempotent** — creating a redb store twice would create two
   `store_uuid` values. Cutover's journal therefore records the
   `store_uuid`/`zone_uid` immediately after store creation; a resumed
   `apply` that finds Phase 5 already `succeeded` in the journal opens the
   existing store by that recorded identity rather than creating a second
   one, and fails closed with `cutover-precondition-failed` if the store at
   the expected path does not match the recorded identity (this is the same
   "previously provisioned database that is missing, replaced, bound to
   another Zone/UID... fails closed" invariant `ADR-046-resource-store-redb`
   "Store identity" already defines for the store in general).
4. **Retry budget.** Any individual step retries up to 3 times with bounded
   backoff before the whole `apply` invocation aborts with a typed failure
   naming the exact step, phase, and journal path — cutover never retries
   indefinitely or silently degrades a failed step into a skipped one.
5. **The journal is retained forever for a given checkpoint_id** (it is small
   and append-only); it is pruned only when its `checkpoint_id`'s snapshot is
   pruned under [Backup/retention count 1-16, no TTL](#backupretention-count-1-16-no-ttl).

## Old artifact/unit/schema removal gates

Phase 10 (`d2b host cutover finalize --consent "<phrase>"
[--json | --human]`) is the **only** phase permitted to Destroy anything
beyond the Phase 3 boot-scoped runtime sockets. It requires its own separate
consent phrase (bound to the same `checkpoint_id`, distinct wording from
`apply`'s), and each candidate must independently clear its own gate — there
is no bulk Destroy:

| Candidate | Gate that must clear before Destroy | Removal proof |
| --- | --- | --- |
| `d2bd.service`, `d2bd.socket`, `d2b-priv-broker.service`, `d2b-priv-broker.socket` unit files | New fixed Zone runtime units installed and `verify` check 1 passed at least once since the units were stopped | `tests/host-integration/cutover-unit-retirement.nix` boots with only new units present and passes `d2b host cutover verify` |
| `/etc/d2b/realm-controllers.json`, `/etc/d2b/realm-identity.json` | Zone self-resource and every `ZoneLink` derived from them report `Ready`/accepted-`Degraded` per `verify` check 6 | `ADR046-nix-008`/`ADR046-nix-009` parity tests pass against the live Zone |
| `nixos-modules/options-realms*.nix`, `nixos-modules/options-vms.nix` | Every VM/realm declaration has an equivalent `d2b.zones.<zone>.resources.*` declaration that produced a `Ready` Guest/Host in `verify` | `tests/unit/nix/cases/realm-to-zone-parity.nix` |
| `/var/lib/d2b/vms/<vm>/swtpm/`, `/var/lib/d2b/swtpm-markers/<vm>` (source side of an Adopt) | `verify` check 4 (TPM digest match) passed **and** the destination TPM Volume has survived at least one full Guest restart cycle post-cutover, proving the adopted marker is load-bearing in practice, not merely digest-equal | `integration/swtpm_marker.rs` adapted; `tests/host-integration/tpm-adopt-retirement.nix` |
| `/var/lib/d2b/vms/<vm>/store/` (source side of an Adopt) | `verify` check 5 (store-view digest match) **and** a live Guest boot successfully mounts the new store-view Volume path | `tests/store_view.rs`; `integration/store_view.rs` |
| Legacy `d2b-realm-router` PeerSession/MuxSession, `WorkloadOp`/`RealmMethod` wire types | Every v3 successor route (`d2b-bus`, `ResourceOp`, ZoneLink) is compiled in and `verify` passes with the new wire exclusively | `packages/d2b-contracts/tests/` wire-type removal compile check |
| Legacy CLI verbs (`d2b vm *`, `d2b realm *`, `d2b up/down/...`) | v3 successor verb implemented and dispatch-wired per `ADR-046-cli-and-operations` "Removal notes" | Compilation failure if any legacy `cmd_vm_*`/`cmd_realm_*` dispatch entry is reintroduced (policy lint, per existing `policy_*` test convention) |
| `d2b-unsafe-local-helper` binary, `DaemonToUnsafeLocalHelper` wire protocol | Process Provider supervisor ticket migration for the user-only Host is live and passes conformance | Per `ADR-046-current-code-migration-map` row for `d2b-unsafe-local-helper` |

Every row above stays at **Preserve** until its gate clears, independent of
every other row — clearing the TPM gate does not imply the store-view gate is
also clear. `d2b host cutover finalize` reports, per candidate, which gate is
outstanding and refuses to Destroy that candidate until it clears; it never
partially destroys a candidate (e.g., it never removes only the marker file
while leaving the swtpm directory, or vice versa).

## Data export/import where selected

Cutover itself imports no row-level data into the resource store (see
[Resource-store initialization](#resource-store-initialization)). Two
operator-selected export/import affordances exist, both optional and
explicitly requested — never automatic:

1. **Pre-cutover audit export.** `d2b audit export` (the current, retained
   verb) or, once the Zone is live, `d2b zone audit export` may be run by the
   operator *before* Phase 3 drain to produce an NDJSON export of the legacy
   hash chain for archival outside `/var/lib/d2b`. This is purely advisory
   and has no effect on cutover's own behavior — the legacy audit segments
   are Preserved regardless (see next section).
2. **Guest configuration export/import.** For a Guest whose declarative
   configuration cannot be mechanically derived from its pre-cutover Nix
   declaration (rare; only applies to hand-edited `/etc/nixos` drift outside
   the merged d2b PR, per the ADR 0034 "Repository landing remains separate
   from host adoption" precedent), the operator may explicitly pass
   `--export-guest-config <vm>` to `preflight`, which writes that Guest's
   resolved configuration to the cutover snapshot directory for manual
   reconciliation into the candidate v3 Zone bundle before `apply`. This is
   the only case where cutover reads configuration content for a purpose
   other than direct 1:1 Nix-option translation, and it is always
   operator-initiated per Guest, never automatic for the whole host.

No other export/import path exists. In particular, there is no bulk
"export the daemon's in-memory state" or "import realm-controllers.json rows
into the resource store" affordance — those artifacts are Preserved read-only
evidence, not import sources, consistent with [Resource-store
initialization](#resource-store-initialization).

## Audit chain closure and opening

The pre-cutover audit surface is a set of independent SHA-256 hash chains:
daemon (`daemon-events-*.jsonl`), broker (`AuditWriteClass`-rate-limited
segments), and, for any gateway-backed realm, `JsonlGatewayAudit` segments.
Cutover treats each chain identically:

1. **Close.** During Phase 3 drain, immediately before stopping the owning
   process, a terminal **closure record** is appended to each chain:
   `{"event": "chain-closed", "reason": "adr0046-cutover", "checkpoint_id": "<id>", "last_record_hash": "<hash>", "at": "<RFC3339>"}`,
   computed with that chain's own existing `record_hash`/`prev_hash`
   algorithm so the closure record is itself a verifiable final link.
2. **Preserve, never delete.** Every closed segment file is **Preserved**
   forever under its current path — it is authoritative historical evidence
   and is never a Destroy candidate at any Phase 10 gate. (This is
   consistent with `ADR-046-current-code-migration-map` and
   `ADR-046-telemetry-audit-and-support`'s `audit_segments_preserved_on_provider_delete`
   test convention: audit segments are never deleted as a side effect of
   anything this spec does.)
3. **Open.** The first write to the new Zone's `d2b-audit` chain (per
   `ADR-046-telemetry-audit-and-support` `ADR046-audit-*` work items) is a
   **genesis record**: `{"event": "chain-opened", "reason": "adr0046-cutover", "checkpoint_id": "<id>", "closed_chain_refs": [<path list of every closed chain's final closure-record hash>], "at": "<RFC3339>"}`.
   This is the only place a v3 audit record references pre-cutover evidence,
   and it references it only by path and closure hash — never by copying
   pre-cutover record content into the new chain.
4. **Verification.** [Post-cutover verification](#post-cutover-verification)
   check 9 confirms the new chain's genesis record's `closed_chain_refs`
   match the actual closure records written in step 1, byte for byte.
5. **No merge.** The old and new chains are never merged, re-hashed together,
   or presented as one continuous chain to `d2b zone audit export` — an
   operator who needs pre-cutover history reads the old segment files
   directly (still present, per step 2); `d2b zone audit export` only ever
   serves the new chain.

## Reset of stateless components and fresh state

The large majority of Provider components installed during cutover are
stateless: their bounded non-secret operational state lives in resource
`status` and the core Operation ledger (D087), so they declare no state Volume
and none is created. For the minority of components that declare an optional
state Volume under the storage-need test, a freshly installed Provider's
*declared* state Volume is created fresh by Core ProviderDeployment exactly as
`ADR-046-provider-state` "Volume creation and ownership" specifies, with a
valid `stateSchema`. This is worth naming explicitly here only to make clear
that cutover:

- creates a state Volume for a freshly installed Provider **only** for a
  declared namespace that passed the storage-need test — never an empty,
  identity-only Volume, and never a Volume for a stateless component (per the
  revised D076);
- never invents synthetic prior state for a freshly installed Provider —
  `stateSchemaPhase: current` and `installedSchemaVersion` equal to
  `spec.stateSchema.schemaVersion` are set at creation, not derived from any
  cutover-specific migration path;
- treats this identically for a component on a Host and a component on a
  Guest — the placement rules in `ADR-046-provider-state` "State placement
  under Host/Guest/user execution" apply unmodified during cutover.

## Incident hold (cutover-wide)

An operator may declare a cutover-wide incident hold at any point from the
end of Phase 2 (inventories built) through the end of Phase 9 (verification)
with `d2b host cutover hold --reason "<bounded operator text>"
[--json | --human]`. While active:

1. Phase 4 (disposition execution), Phase 8 (Guest/runtime activation past
   what has already started), and Phase 10 (removal gates) refuse to proceed
   — a hold blocks every destructive or state-creating step the same way a
   per-Volume `IncidentHold` condition blocks `deletionRequestedAt`
   processing and migration commit in `ADR-046-provider-state` "Incident
   hold";
2. read-only phases (Preflight, `plan`, `verify`, `doctor`) continue to work
   normally — a hold never blocks observation;
3. any Adopt migration `EphemeralProcess` already in flight when the hold is
   declared completes its current atomic step (it does not abort mid-write,
   which would violate the idempotency contract in [Crash/power-loss/retry/
   idempotency journals](#crashpower-lossretryidempotency-journals)) and then
   pauses before starting its next step;
4. the hold is recorded in the cutover journal as a phase-independent
   annotation (not tied to any single phase/step) and is cleared only by the
   same administrative Role that set it, via
   `d2b host cutover hold --clear --reason "<...>"`;
5. once every per-Volume `IncidentHold` condition this cutover created
   (Volumes it adopted state into) is individually cleared by their owning
   Provider controller's normal reconcile per `ADR-046-provider-state`, the
   cutover-wide hold is a separate, independent hold and must be cleared on
   its own — clearing one does not implicitly clear the other.

## Full Zone reset vs Provider reset vs Guest reset

These three destructive operations are related but strictly nested in scope.
None of them is the cutover this spec otherwise describes — they are the
post-cutover recovery levers this spec defines as the "full Zone reset,"
"Provider reset," and "Guest reset" scopes the rest of the ADR 0046 set
(system-minijail, volume-local, and other dossiers) already reference by name
without defining their mechanism.

### Not a reset: disruptive upgrade/recycle (D091)

An in-place disruptive **upgrade/recycle** (`d2b upgrade <ref> [--recursive]
--apply`, D091) is distinct from and never a substitute for these destructive
resets. An upgrade preserves the Resource UID and spec identity and preserves
durable/state/secret Volumes and TPM identity (`preserveState: true`), recycling
only the resource's realization and owned ephemeral Processes/endpoints, and its
dependency-aware planner drains dependents first. `Replace` of a resource-row
identity is used only when explicitly required and is planned with ownership and
state transfer so durable/state Volumes and TPM identity move to the replacement
rather than being wiped. Owned `Endpoint` resources (D092) recycle with their
producer (bumping `endpointGeneration` so consumers re-resolve) and are deleted
child-first with the producer/owner; they carry no raw locator, so recycling an
endpoint never leaks or persists a path/address/fd. A full factory reset (below)
is the separate, explicitly-destructive lever; upgrade/recycle never silently
deletes durable state, and reset never masquerades as an upgrade.

### Full Zone reset

`d2b host reset --scope zone --target Zone/<name> [--dry-run | --apply]
[--json | --human]` executes exactly the out-of-band destructive procedure
`ADR-046-resources-zone-control` §2.6 and §9.4 already define, and which this
spec is the authoritative operational owner of (see [Cross-reference and
evidence corrections](#cross-reference-and-evidence-corrections)):

1. Core adds the `core.zone-drain` finalizer to `Zone/<name>`.
2. `metadata.deletionRequestedAt` is set on the Zone; core stops admitting new
   resource/service requests.
3. Every non-`Zone` resource receives a delete request in reverse dependency
   order under normal finalizer protocol (Guests/Processes first, then their
   owning Providers, then Volumes, then Networks/Devices/Credentials, with
   authored qualified Bindings removed/retargeted before their import-owned
   projection Services, then ResourceImport/ResourceExport rows, ZoneLinks, and
   finally Role/RoleBinding/Quota/EmergencyPolicy).
4. After every other resource is deleted, `core.zone-drain` is cleared and a
   final transaction emits the Zone's own `phase=Deleted` event and closes
   the store.
5. Zone runtime re-enters compiled bootstrap authorization (§9 of
   `ADR-046-resources-zone-control`) for a fresh initialization — equivalent
   to re-running this spec's Phase 5 onward against a new, empty store at the
   same store path.
6. Authentication for this operation is OS-level (uid=0 or the local `d2b`
   group's `SO_PEERCRED` admission, matching the existing local lifecycle
   authorization surface) — it is never reachable remotely or through
   d2b-bus, exactly as `ADR-046-resources-zone-control` §9.4 requires.
7. **Full Zone reset destroys every Volume in the Zone, including
   `kind: durable` Volumes, unless `--preserve-durable-volumes` is passed**,
   in which case durable Volumes are relocated (per `ADR-046-provider-state`
   "Relocation") to a holding area outside the Zone before store deletion and
   are eligible for re-attachment to a freshly reconciled Guest/Provider
   after the reset completes. This flag defaults to **on** (durable Volumes
   are preserved by default) — a Full Zone reset that would destroy durable
   Volumes requires the operator to explicitly pass
   `--destroy-durable-volumes` and the same exact-consent-phrase pattern as
   cutover `apply`.

### Provider reset

`d2b host reset --scope provider --target Provider/<name>
[--destroy-volumes] [--dry-run | --apply] [--json | --human]`:

1. Deletes `Provider/<name>` through the normal resource API delete path
   (`deletionRequestedAt`, finalizer-ordered child deletion, `Deleted` event);
   the Provider's and its children's `status` — the default surface for bounded
   non-secret operational state (D087) — disappears with the resource row and
   its revision, requiring no separate state disposition;
2. Any declared Volume in that Provider's (possibly empty) `ProviderStateSet`
   is deleted **only** if `--destroy-volumes` is explicitly passed; by default,
   Volumes with `persistenceClass: persistent` are detached (ownerRef cleared
   to `null` pending operator disposition) rather than deleted, surfaced as
   `Unclaimed` per `ADR-046-provider-state` "Unclaimed Volume GC" — an
   operator must explicitly delete an unclaimed Volume; it is never
   automatically swept by a Provider reset;
3. Re-creating `Provider/<name>` afterward (a fresh `apply` of the same
   `artifactId`) goes through the normal Provider install algorithm and, if
   any Volume from the prior instance is still present and unclaimed with a
   matching component-state schema, the new instance's Core ProviderDeployment
   does **not** automatically re-adopt it — the operator must explicitly
   re-attach it, because an automatic re-adopt across a Provider identity
   change is exactly the kind of implicit ownership inference
   `ADR-046-resource-object-model` "Deletion" already forbids ("a
   deleted/recreated object with the same type/name has a different UID and
   does not silently inherit old ownership/operation state").
4. Scope: never touches the Zone, other Providers, or any Host/Guest that
   merely depends on this Provider (they transition to `Degraded/provider-unavailable`
   until the Provider is reinstalled or its dependency is satisfied
   otherwise).

### Guest reset

`d2b host reset --scope guest --target Guest/<name> [--destroy-volumes]
[--dry-run | --apply] [--json | --human]`:

1. Deletes `Guest/<name>` through the normal resource API delete path;
2. The Guest's owned Process/EphemeralProcess children are deleted
   child-first under normal finalizer protocol;
3. The Guest's store-view and TPM Volumes follow the exact same
   `--destroy-volumes`-gated preserve-by-default rule as Provider reset above
   — **never** destroyed by default, consistent with [Never wipe TPM
   identity or durable Volumes silently](#never-wipe-tpm-identity-or-durable-volumes-silently);
4. Re-creating `Guest/<name>` afterward re-attaches a preserved Volume only
   when the operator explicitly names it in the new Guest's declaration
   (never automatically);
5. Scope: never touches the Zone, the Guest's runtime Provider, or any other
   Guest.

### ResourceExport/ResourceImport reset behavior (D096)

All reset scopes tear down cross-Zone sharing child-first:

1. `ResourceExport` reset stops new advertisements, revokes every active lease
   through the Provider export adapter, waits for bounded revoke/deadline
   completion, and then drops the advertisement before the export row is
   deleted. It never deletes the owner Service or its local backing.
2. `ResourceImport` reset marks its same-qualified-type projection Service
   draining/revoked and refuses new sessions. Matching Binding controllers stop
   their owned Process/Endpoint children.
3. Binding is operator/Nix-owned desired consumer intent, never import-owned;
   observed realization belongs only in status. A scoped reset that includes
   the Binding deletes it normally; a narrower import reset waits with
   `BindingReferencesRemain` until each Binding is explicitly deleted or
   retargeted. The import controller never auto-deletes Binding.
4. After no Binding references remain, the import releases the remote lease,
   deletes only the core-owned projection Service
   (`ownerRef: ResourceImport/<name>`) and remaining provider-owned children,
   then deletes the import row.
5. ZoneLink loss during reset is treated as revoke/degrade, not as retained
   authority. Reconnect after a reset must revalidate generation and schema
   plus factory fingerprint before any new lease is admitted.
6. Full factory reset removes export/import rows, authored Bindings in scope,
   projection Services, internal leases/sessions/streams, and advertisements.
   No cross-Zone authority, capability grant, stream credit, or import session
   generation survives. Owner Service backing follows its own explicit reset
   disposition; no backing is silently wiped.

Reset/recreate preserves the qualified semantic Service/Binding type exactly;
it does not derive a type from the selected Provider or persist/copy a remote
`spec.provider` extension. A replacement Provider must accept the canonical
minimal base and bind the same semantic factory metadata. PipeWire, CTAPHID,
OTEL, and USBIP implementation state is disposed only under that Provider's
explicit reset policy and never becomes semantic base status or surviving
remote authority.

### Comparison table

| Property | Full Zone reset | Provider reset | Guest reset |
| --- | --- | --- | --- |
| Scope | Entire Zone (all resources) | One Provider + its ProviderStateSet | One Guest + its children |
| Authentication | OS-level (uid=0/local `d2b` group), never remote/d2b-bus | Normal resource API RBAC | Normal resource API RBAC |
| Durable Volume default | Preserved (relocated out of Zone) unless `--destroy-durable-volumes` | Detached/Unclaimed unless `--destroy-volumes` | Detached/Unclaimed unless `--destroy-volumes` |
| ResourceExport/ResourceImport | Revoke leases; remove authored Bindings in scope, then projection Services/imports/exports; preserve backing by explicit disposition | Revoke affected Service exports/imports; wait for Bindings to delete/retarget; never cascade Binding or backing | Guest-targeted Bindings and owned children delete first; imports wait on any remaining Bindings; sibling exports unaffected |
| Re-entry after reset | Compiled bootstrap authorization | Normal Provider install | Normal Guest declaration |
| Effect on siblings | None (other Zones on the host, if any, are unaffected) | Dependents degrade; not deleted | None |

## CLI UX/exit codes/JSON plan

### Command surface

```text
d2b host cutover preflight [--zone <zone>] [--json | --human]
d2b host cutover plan      [--zone <zone>] [--json | --human]
d2b host cutover apply     --consent "<phrase>" [--zone <zone>] [--allow-degraded-guests <name>[,<name>...]] [--json | --human]
d2b host cutover verify    [--zone <zone>] [--json | --human]
d2b host cutover hold      --reason "<text>" | --clear --reason "<text>" [--json | --human]
d2b host cutover finalize  --consent "<phrase>" [--zone <zone>] [--only <candidate>[,<candidate>...]] [--json | --human]
d2b host cutover rollback  --checkpoint <checkpoint_id> [--json | --human]
d2b host cutover doctor    [--zone <zone>] [--read-only] [--json | --human]

d2b host reset --scope zone     --target Zone/<name>     [--destroy-durable-volumes] [--dry-run | --apply] [--json | --human]
d2b host reset --scope provider --target Provider/<name> [--destroy-volumes]           [--dry-run | --apply] [--json | --human]
d2b host reset --scope guest    --target Guest/<name>     [--destroy-volumes]           [--dry-run | --apply] [--json | --human]
```

Every mutating verb (`apply`, `finalize`, `rollback`, `hold`, `reset
--apply`) follows the existing `require_explicit_mutation_flag` precondition
pattern from `packages/d2b/src/lib.rs`: a bare invocation with neither
`--dry-run` nor `--apply` (for `reset`) or without `--consent` (for `apply`/
`finalize`) refuses with exit code 2 and a usage message rather than silently
defaulting to either mode.

### New stable error classes

| Error class | Meaning |
| --- | --- |
| `cutover-precondition-failed` | A preflight/drain/adopt precondition (Guest not stopped, marker missing, bundle invalid, store identity mismatch) failed |
| `cutover-consent-required` | `--consent` absent, mismatched, or bound to a stale `checkpoint_id` |
| `cutover-already-complete` | `apply`/`preflight` invoked against a host whose journal already shows Phase 5+ complete |
| `cutover-checkpoint-not-found` | Named `checkpoint_id` has no snapshot/journal on disk |
| `cutover-rollback-window-closed` | `rollback` requested against a checkpoint whose last completed phase is `>= 5` |
| `cutover-incident-hold-active` | A mutating verb refused because a cutover-wide incident hold is active |
| `zone-reset-scope-invalid` | `--scope`/`--target` combination does not name a resolvable resource of the expected type |
| `zone-reset-consent-required` | `--apply` for `host reset --scope zone` without the required consent phrase |

### Exit codes

Every verb above uses the existing stable exit-code table in
`ADR-046-cli-and-operations` unmodified:

| Exit code | Used for |
| --- | --- |
| 0 | Success (dry-run plan rendered, apply/finalize/rollback/reset completed, verify all-clear) |
| 1 | `cutover-precondition-failed`, `zone-unavailable`, `provider-unavailable`, or any operational failure |
| 2 | Usage error (missing `--apply`/`--dry-run`/`--consent`, invalid `--scope`/`--target`) |
| 3 | Cancelled (SIGINT/SIGTERM during a long-running `apply`/`finalize`) |
| 78 | `not-implemented` (reserved; this spec's verbs are ADR-only until their implementation work items land, per D024) |

`verify` uses exit 0 only when every check in [Post-cutover
verification](#post-cutover-verification) passes; exit 1 with the first
failing check named otherwise (it does not require every check to run before
reporting the first failure, but it does run every independent check it can
before returning so a single invocation surfaces as many problems as
possible).

### JSON plan schema

`plan`'s JSON shape is given in full in [Explicit operator consent and
dry-run plan](#explicit-operator-consent-and-dry-run-plan) above. `apply`,
`finalize`, and `reset --apply` share one result envelope shape:

```json
{
  "command": "host cutover apply",
  "checkpointId": "cutover-v1-4f2a9c7b1e83",
  "phasesCompleted": [0, 1, 2, 3, 4, 5, 6, 7, 8],
  "verifyRecommended": true,
  "degradedGuests": [],
  "warnings": []
}
```

`--human` output mirrors the existing `host migrate-storage`/`host destroy`
text convention: one summary line, then a bulleted list per section
(preflight requirements, preserve, disposition, hazards), with no manual
`chmod`/`chown`/`setfacl` remediation text anywhere in the human or JSON
output — consistent with the existing `host migrate-storage` documentation
note that such instructions are never an acceptable recovery path.

## NixOS activation sequencing

Cutover spans two distinct NixOS activation contexts, and this spec is
explicit about which one applies where:

1. **The one-time bootstrap `nixos-rebuild switch`.** Before any Zone runtime
   exists, the operator runs a normal `nixos-rebuild switch` against a NixOS
   configuration that has replaced `nixos-modules/options-realms*.nix`/
   `options-vms.nix` with `d2b.zones.<zone>.*`. This activation:
   - installs the new fixed Zone runtime systemd unit set (the
     `Provider/system-core`/`system-minijail` bootstrap processes' owning
     unit, per `ADR-046-core-controllers` "Process model") **without
     starting it** — `system.activationScripts` orders the new unit's
     installation `Before=` the point where old units are stopped, but the
     unit itself is declared `wantedBy = []`/not auto-started at this
     activation, so the physical host boots with both the old units present
     (already stopped by a prior `d2b host cutover` Phase 3 drain, if this
     is a re-activation) and the new unit installed-but-dormant;
   - does **not** delete `d2bd.service`/`d2b-priv-broker.service` unit files
     yet — those remain Preserved until [Old artifact/unit/schema removal
     gates](#old-artifactunitschema-removal-gates) clears them in Phase 10;
   - is itself rollback-safe via the normal NixOS generation mechanism for as
     long as this spec's [rollback boundary](#rollback-boundary) remains
     open (through end of Phase 4).
2. **`d2b host cutover apply`** is then run interactively by the operator
   (never from an activation script — it requires the exact consent phrase
   from a human-read `plan` output) and is what actually starts the new Zone
   runtime unit and executes Phases 3-8.
3. **After cutover completes**, ordinary NixOS rebuilds resume their normal
   role: they change the Zone's `d2b.zones.<zone>.*` Nix configuration and
   activate new configuration generations through the ordinary
   `ADR-046-nix-configuration` "Bundle and generation emission" contract —
   the `activation-nixos` Provider (once installed, per [Provider
   install/topological start](#provider-installtopological-start) Stage D)
   takes over ordinary Host/Guest NixOS generation plan/apply/adopt/rollback
   from that point forward; it is never used for the initial bootstrap
   switch itself, because a Provider cannot exist before the Zone runtime
   that hosts it exists — this is the same chicken-and-egg boundary the
   [volume-local reaches Ready without a state Volume](#volume-local-reaches-ready-without-a-state-volume)
   section already establishes for Volume creation, applied here to NixOS
   activation instead.
4. **Ordering invariant.** The new fixed Zone runtime unit is declared
   `After=` and `Requires=` nothing that depends on Zone runtime already
   being up (it is the process that brings Zone runtime up); it is declared
   `Before=` nothing that the old `d2bd.service`/`d2b-priv-broker.service`
   units are declared `After=`, so the two unit sets never race for the same
   socket path during the window where both are installed but only one is
   running.

## Backup/retention count 1-16, no TTL

Cutover snapshots use the identical retention model
`ADR-046-nix-configuration` D066 already establishes for configuration
generations, applied to `/var/lib/d2b/cutover/<checkpoint_id>/` directories
instead of `bundle/generation-<N>.json` files:

```nix
d2b.site.cutoverSnapshotRetention = 3;   # default 3; range 1..16
```

- an eval assertion enforces `1 <= cutoverSnapshotRetention <= 16`, mirroring
  the existing `retainedGenerations` assertion exactly;
- retention is **count-based only** — there is no TTL/age-based expiry of a
  cutover snapshot, matching the existing "Generations within retention count
  retained (no TTL)" test invariant already proven for configuration
  generations;
- pruning removes only snapshot directories beyond the retention count,
  oldest first, and only for snapshots whose checkpoint shows Phase 10 fully
  cleared (a snapshot for an incomplete or rolled-back cutover is never
  auto-pruned, regardless of age, until the operator explicitly discards it
  with `d2b host cutover doctor --discard-checkpoint <id>`);
- pruning a snapshot never touches the live adopted Volumes/Providers/Guests
  it describes — only the snapshot's own JSON/journal files under
  `/var/lib/d2b/cutover/<checkpoint_id>/` are removed.

## Disk-space/GC safety

Cutover's disk-space guard runs **before** every other preflight step,
mirroring `tests/tools/preflight-disk-space.sh`'s placement "after the orphan
reapers but BEFORE the rust toolchain bootstrap so the fail-closed guard
cannot be bypassed by disk-consuming setup":

1. Compute required headroom: the sum of every Adopt candidate's size (TPM
   NVRAM trees, disk images, store-view farm) that will be **copied** rather
   than **hardlinked/renamed** in place, plus a fixed 10 GiB floor matching
   the existing project-wide disk-hygiene convention, plus the cutover
   snapshot's own estimated size (bounded; snapshot JSON never embeds full
   file contents, only digests and metadata, so this is small).
2. If free space under `/var/lib/d2b` (or wherever the target Volume root
   will live, if separately configured) is below this computed requirement,
   `preflight` fails closed with `cutover-precondition-failed` and prints the
   exact shortfall in bytes — it never proceeds with a partial Adopt that
   could run out of space mid-copy.
3. Adopt operations that use same-filesystem `rename`/hardlink (store-view
   farm, matching the existing ADR 0027 hardlink-farm contract) require zero
   additional headroom beyond directory-entry overhead and are excluded from
   the "copied" sum above — cutover's disk-space estimate distinguishes
   same-filesystem-cheap Adopt from cross-filesystem-expensive Adopt exactly
   as `ADR 0034` already requires for the hardlink-sensitive store-view path
   ("same-filesystem/cross-mount invariants... recursive chmod/chown/setfacl
   over a hardlink farm remains forbidden").
4. GC safety: cutover snapshot pruning (previous section) and the ordinary
   Zone generation GC (`d2b activation gc`, once `activation-nixos` is
   installed) are independent budgets — pruning a cutover snapshot never
   counts against or interferes with `retainedGenerations` pruning, and vice
   versa.

## Failure/quarantine/manual recovery

Cutover reuses the ADR 0034 degraded-state ledger model, extended with one
new closed reason class:

| Class | When it is set | Recovery |
| --- | --- | --- |
| `cutover-quarantined` | Any Phase 4-8 step whose ambiguity cannot be resolved automatically (e.g., a marker exists at the destination but does not match the source's recorded digest; a redb store exists at the target path but its `zone_uid` does not match the journal's recorded identity) | `d2b host cutover doctor` surfaces the exact quarantined step, affected path/resource, and a static remediation id; the privileged broker never repairs a quarantined path from trusted paths/owners/modes in the ledger — only from the trusted bundle/journal, matching the existing ADR 0034 invariant that "repairs never trust paths, owners, modes, ACLs, or commands from the ledger" |
| `adoption-quarantined` (reused from ADR 0034) | An Adopt migration `EphemeralProcess` finds multiple, ambiguous, or mismatched candidates for a marker-bearing source (e.g. two swtpm directories claim the same Guest identity) | Operator resolves the ambiguity manually (removing the stale candidate) and re-runs the specific Adopt step; cutover never guesses which candidate is authoritative |
| `restart-required` (reused) | A Provider/Guest process needs a restart to pick up newly adopted state (e.g., `volume-local` restarted mid-Adopt) | `d2b host cutover doctor` names the exact process; operator restarts it directly |
| `storage-drift` (reused) | A path's on-disk owner/mode/ACL does not match its declared storage contract entry after Adopt | Broker repair resolves only the trusted storage id from the bundle, never a raw path from the ledger |

`d2b host cutover doctor [--zone <zone>] [--read-only] [--json | --human]`
is read-only (its `--read-only` flag is accepted for symmetry with `d2b host
doctor` but is always effectively on for this verb — `doctor` never mutates).
It reports:

1. every open journal checkpoint and its last completed phase/step;
2. every degraded-ledger entry tagged with any class above;
3. the exact remediation command for each (never a raw chmod/chown/setfacl
   suggestion);
4. whether an incident hold is currently active and its reason text;
5. whether the [rollback boundary](#rollback-boundary) is still open for the
   named checkpoint.

Manual recovery for a `cutover-quarantined` entry always follows the same
shape as the existing ADR 0034 degraded-ledger recovery: the operator
resolves the *specific* ambiguity named by `doctor` (never a broad
chmod/chown/setfacl instruction), then re-runs the exact `apply`/`finalize`
invocation, which resumes from the journal per [Crash/power-loss/retry/
idempotency journals](#crashpower-lossretryidempotency-journals) rather than
restarting the whole cutover.

## Migration/disposition matrix

This is the exhaustive application of the [Disposition
framework](#disposition-framework) to every artifact this spec's
[inventories](#authoritative-inventories) enumerate. Every row is tied to its
`ADR-046-current-code-migration-map` evidence (§ and evidence class) and, for
Provider-owned destinations, the exact Provider dossier and work item that
owns the destination.

### Daemon, broker, and process substrate

| Current artifact | Evidence (migration-map) | Disposition | Target | Owning work item / dossier |
| --- | --- | --- | --- | --- |
| `d2bd.service`, `d2bd.socket` | §7, `production-reachable` | Preserve until Phase 10 gate clears (§ [Old artifact/unit/schema removal gates](#old-artifactunitschema-removal-gates)), then Destroy | Fixed Zone runtime unit | `ADR046-core-001` |
| `d2b-priv-broker.service`, `d2b-priv-broker.socket` | §7, `production-reachable` | Preserve until Phase 10 gate clears, then Destroy | Zone-local privileged broker (`ADR046-provider-003`) | `ADR046-provider-003` |
| `/run/d2b/d2bd.sock`, `/run/d2b/broker.sock` | §7 | Destroy (Phase 3, boot-scoped) | `/run/d2b/z-<zone-id>/...` fresh sockets | `ADR046-core-controllers` "Process model" |
| `/run/d2b/allocator.sock` | §7, config-ref/schema-only, engine not live | Destroy (Phase 3; never adopted — no live allocator process to quiesce) | No successor socket; provisioning integrates into fixed core controllers | `ADR046-core-001` |
| `d2b-realm-router` PeerSession/MuxSession/`WorkloadOp`/`RealmMethod` wire | multiple rows, `dead-reachable`/`production-reachable` | Preserve (compiled into binary; Destroy only when the binary itself is retired at Phase 10) | ComponentSession/d2b-bus/`ResourceOp` | `ADR046-session-001`, `ADR046-bus-001`, `ADR046-api-001` |
| `d2b-unsafe-local-helper` binary, `DaemonToUnsafeLocalHelper` protocol | §7, `production-reachable` | Preserve until Process Provider supervisor ticket migration lands, then Destroy | User-only `Host` Process supervisor | `ADR046-primitives-003` |
| `d2b-guest-shell-runner` | `production-reachable` | Preserve until user-only Host shell Process parity, then Destroy | `Process` child of user-only Host | `ADR046-primitives-003` |
| `~/.local/state/d2b/unsafe-local-scopes.json` | §7 evidence, per-user scope ledger | Adopt into a declared user-only Host state Volume — this ledger is real private per-user content that passes the storage-need test, not derivable from status/core ledger — via a per-user migration `EphemeralProcess` | User-only Host declared component state Volume | `ADR046-primitives-003` |

### Storage/restart/synchronization contract (ADR 0034)

| Current artifact | Evidence | Disposition | Target | Owning work item / dossier |
| --- | --- | --- | --- | --- |
| `storage.json` | ADR 0034 generated artifact | Preserve (read by legacy code until every storage id has a live Volume/resource successor), then Destroy per-id as each successor lands | Per-artifact `Volume`/resource storage declaration | `ADR046-store-003` |
| `sync.json`/`locks.json` | ADR 0034 generated artifact | Preserve, then Destroy per-id as each lock's successor (OFD-lock-owning resource/controller) lands | Internal controller/transaction lock mechanics (not a ResourceType, per `ADR-046-resource-object-model` "Folded implementation detail") | `ADR046-store-001` |
| Daemon degraded-state ledger | ADR 0034 | Preserve as historical evidence; new degraded conditions post-cutover use the Zone resource `status.conditions` model instead | `Resource.status.conditions` | `ADR046-core-001` |
| OFD lock files under `/run/d2b` | ADR 0034 | Destroy only via normal reboot/tmpfs cleanup — never unlinked directly by cutover (explicit fail-closed hazard) | N/A (mechanism, not a resource) | N/A |

### TPM (device-tpm Provider dossier)

| Current artifact | Evidence | Disposition | Target | Owning work item / dossier |
| --- | --- | --- | --- | --- |
| `/var/lib/d2b/vms/<vm>/swtpm/` | migration-map row (TPM/swtpm section), `production-reachable` | **Adopt** (never Destroy until Phase 10 gate; never silently re-provisioned) | Controller-created TPM `Volume` per Guest | `ADR-046-provider-device-tpm` §17.3; `ADR046-device-tpm-004`; `ADR046-vl-005` |
| `/var/lib/d2b/swtpm-markers/<vm>` | migration-map row | **Adopt**, re-keyed from `<vm>` basename to `device_uid`-based name by `volume-local` | TPM Volume identity marker (`broker-maintained` class) | `ADR-046-provider-device-tpm` §17.3 |
| `SwtpmArgvInput`/`SwtpmIoctlFlushInput` (`d2b-host/src/swtpm_argv.rs`) | `production-reachable` | Preserve until extracted, then Destroy at Phase 10 | `d2b-provider-device-tpm/src/` | `ADR046-device-tpm-001` |
| `PrepareSwtpmDir` broker op | `production-reachable` | Preserve (still invoked, now only by `volume-local`) | Same op, narrower caller | `ADR-046-provider-device-tpm` §18 |
| `components/tpm.nix` | `nix-emitted` | Preserve until Device Nix declaration (§17.1 of the dossier) is authored for every Guest, then Destroy | `d2b.zones.<zone>.resources.<name>` Device declaration | `ADR-046-provider-device-tpm` §17.3 |

### Store-view hardlink farm and disk images (volume-local Provider dossier)

| Current artifact | Evidence | Disposition | Target | Owning work item / dossier |
| --- | --- | --- | --- | --- |
| `/var/lib/d2b/vms/<vm>/store/` | migration-map §7, `production-reachable` | **Adopt** (same-filesystem hardlink-farm rename; zero-copy) | Store-view `Volume` per Guest | `ADR046-vl-004` |
| `/var/lib/d2b/vms/<vm>/store-meta/`, `store-view/` | migration-map §7, legacy-recovery-artifact (optional per `ownership_preflight.rs`) | Preserve if present (optional legacy recovery artifact; not required post-cutover) | Superseded by store-view Volume metadata | `ADR046-vl-004` |
| VM disk images, including writable store-overlay images | ADR 0034 "preserves critical persistent data" | **Adopt** (copy or same-filesystem move depending on the declared source policy) into a `kind: durable` block-image Volume | `Volume` with `source.settings.kind: block-image` | `ADR-046-resources-volume` §"Sources"; `ADR046-vl-006` |
| `nixos-modules/store.nix` (`share.source == "/nix/store"` sentinel) | `nix-emitted` | Preserve until store-view Volume controller is live and passes parity tests, then Destroy | `Provider/volume-virtiofs` reconciled virtiofsd Process | `ADR046-vl-004` |

### Networking

| Current artifact | Evidence | Disposition | Target | Owning work item / dossier |
| --- | --- | --- | --- | --- |
| Declared host bridges, TAP naming intent | `nixos-modules/network.nix`, `net.nix` | Preserve (bridges keep serving traffic through Phase 8 Network-Provider reconciliation; never torn down mid-cutover) | `Network` resource reconciled by `Provider/network-local` | `ADR-046-current-code-migration-map` §"nixos-modules/network.nix" row |
| `inet d2b` nftables table ownership markers (`comment "d2b managed: <ownership-id>"`) | ADR 0013 | Preserve; ownership markers re-validated, never re-created with a new marker scheme during cutover | Same marker scheme, now written by `network-local`'s broker effect adapter | `ADR-046-provider-network-local` |
| NetworkManager/`systemd-networkd` coexistence markers | ADR 0013 | Preserve | Unmodified | `ADR-046-provider-network-local` |

### Keys, credentials, and identity

| Current artifact | Evidence | Disposition | Target | Owning work item / dossier |
| --- | --- | --- | --- | --- |
| `<keysDir>/<vm>_ed25519{,.pub}` | `nixos-modules/host-keys.nix` | Preserve (framework SSH keys are never regenerated by cutover) | Unchanged; continues to be consumed the same way by Guest boot | N/A — out of ADR 0046 Provider scope per activation-nixos dossier §1.2 ("SSH key lifecycle... belongs to a separate identity Provider") |
| `<stateDir>/vms/<vm>/host-keys/{host.pub,user-authorized-keys}` | `nixos-modules/host-keys.nix` | Preserve | Unchanged | Same as above |
| `realm-controllers.json`, `realm-identity.json` | migration-map §"Current-code fit" rows, `implemented-and-reachable`/live | Preserve until every `ZoneLink`/Credential successor is `Ready` (Phase 10 gate), then Destroy | `Zone` self resource + `ZoneLink` bootstrap; Credential `scope`/`audience`/`allowedOperations` fields | `ADR046-nix-008`, `ADR046-nix-009` |
| Gateway guest realm relay credentials/audit (ADR 0032 `CredentialCustody::GatewayGuest`) | ADR 0032 evidence | **Never enumerated by the parent host's inventory** — see [Gateway Guest credential/audit custody](#gateway-guest-credentialaudit-custody) | Nested child Zone's own Credential resources | N/A — parent host cutover has no authority here |

### Audit and telemetry

| Current artifact | Evidence | Disposition | Target | Owning work item / dossier |
| --- | --- | --- | --- | --- |
| `daemon-events-*.jsonl` | `packages/d2bd/src/daemon_audit.rs`, `implemented-and-reachable` | Preserve forever (closed with a terminal record; never a Destroy candidate at any gate) | New chain opened per [Audit chain closure and opening](#audit-chain-closure-and-opening) | `ADR046-audit-*` (telemetry-audit-and-support work items) |
| Broker `audit.rs` segments | `implemented-and-reachable` | Preserve forever | Same | Same |
| `JsonlGatewayAudit` segments (gateway-backed realms only) | `implemented-and-reachable` (gateway paths) | Preserve forever, and never read by the parent Zone (custody boundary) | Nested child Zone's own audit chain | Same |
| `packages/d2bd/src/metrics.rs` hand-rolled Prometheus registry | `implemented-and-reachable` | Preserve until OTEL SDK metrics reach parity, then Destroy | `observability-otel` Provider native OTLP | `ADR-046-telemetry-audit-and-support` work items |

### CLI surface

| Current artifact | Evidence | Disposition | Target | Owning work item / dossier |
| --- | --- | --- | --- | --- |
| `d2b vm *`, `d2b realm *`, `d2b up/down/restart/list/status`, `d2b usb *`, `d2b keys *`, `d2b build/switch/boot/test/rollback/gc/migrate/config *` | `ADR-046-cli-and-operations` "v2 command surface removed at 3.0 clean break" | Preserve (compiled dispatch) until v3 successor is wired, then Destroy at Phase 10 (compile-time removal, verified by policy lint) | `d2b guest/zone/device/exec/shell/activation *` | `ADR-046-cli-and-operations` per-verb work items |
| `d2b host migrate-storage` | `ADR-046-cli-and-operations` "Removal notes" | Destroy at Phase 10 with **no v3 successor** — the layout cutover it served (v1→v2) is unrelated to this cutover and is not re-implemented | None (retired) | N/A |
| `d2b migrate-check` | `ADR-046-cli-and-operations` diagnostic | Preserve (retained diagnostic explaining v2→v3 verb replacements) | Same | N/A |

Any current-baseline path, unit, or artifact not named in any table above is,
per [Authoritative inventories](#authoritative-inventories), classified
**Preserve by default** and surfaced in the `plan` output's `unclassified`
array for explicit operator review before `apply` — this matrix is exhaustive
over every category this spec's inventories walk, but a future Provider
dossier revision that introduces a new current-code row must add a
corresponding row here before its disposition may be anything other than the
Preserve default.

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | ADR 0034 "Migration decision" (planned-downtime storage cutover, preserve list, checkpoint/rollback UX); `packages/d2b/src/lib.rs` `cmd_host_migrate_storage`/`build_storage_migration_plan`/`storage_migration_checkpoint_id` (retired verb, reused shape); `cmd_host_destroy`/`require_explicit_mutation_flag` (dry-run/apply precondition pattern); `packages/d2bd/src/storage_lifecycle.rs`/`ownership_preflight.rs` (bundle-versioned contract checks, legacy-recovery-artifact optionality); `ADR-046-resources-zone-control` §2.6/§9.4 (destructive reset primitive); `ADR-046-provider-state` (migration/incident-hold/unclaimed-GC machinery) |
| Evidence class | The CLI dry-run planning precedent (`host migrate-storage`, `host destroy`) is `implemented-and-reachable`; the ADR 0034 preserve-list/checkpoint contract is `implemented-and-reachable` as design but its `--apply`/`--rollback` are themselves `test-only-or-preview` (fail closed in the current build); the Zone/Provider/Volume destinations this spec adopts into are `ADR-only` |
| Behavior retained | Dry-run-before-apply with a printed checkpoint id and exact rollback command; preserve-list-first design (swtpm NVRAM/markers, SSH keys, store-view state/gcroots, disk images, audit/degraded history never silently destroyed); fail-closed hazards enumerated explicitly rather than left implicit; broker-mediated path-safe mutation (no manual chmod/chown/setfacl); atomic persistence sequence (temp file, fsync, rename, parent fsync) for every durable record this spec's own journal/snapshot writes |
| Required delta | The entire Zone/Provider/redb resource-store bootstrap this spec's Phase 5-8 execute has no current-baseline equivalent at all — ADR 0034's cutover only ever moved a storage *layout*, never replaced the daemon/wire protocol/resource model sitting on top of it; the Full/Provider/Guest reset scopes, the cutover-wide incident hold, and the gateway-custody-aware ZoneLink translation are new |
| Reuse path | Copy the exact `StorageMigrationPlan` JSON shape (renamed fields) for the new `plan`/`apply` JSON envelopes; copy `require_explicit_mutation_flag`'s precondition gate for every new mutating verb; copy `storage_migration_checkpoint_id`'s digest-of-sorted-names pattern extended over every inventory class; copy the ADR 0034 preserve list verbatim as the seed of the [migration/disposition matrix](#migrationdisposition-matrix)'s TPM/keys/disk-image/audit rows; copy `ADR-046-provider-state`'s prepare/stage/commit/rollback/roll-forward algorithm unmodified as every Adopt step's mechanism |
| Replacement/deletion | `d2b host migrate-storage` is retired with no successor (it served an unrelated v1→v2 cutover); every other current-baseline artifact this spec's matrix names is Preserved until its own [Old artifact/unit/schema removal gate](#old-artifactunitschema-removal-gates) clears — nothing is removed by this spec's own authoring, only by its future implementation work items after their gates clear |
| Feasibility proof | A disposable end-to-end cutover rehearsal fixture (single-Guest, single-TPM, single-store-view host) proving: preflight snapshot digest reproducibility; drain quiescence detection; Adopt idempotency across an injected crash at every step boundary; Phase 5 redb bootstrap against the rehearsal fixture's adopted Volumes; Provider install topological order determinism; `verify` catching an injected digest mismatch; rollback within the boundary and refusal past it; Full/Provider/Guest reset scope isolation (resetting one does not affect siblings) |
| Future owner | Work items below |

## Tests

Every test type below follows the taxonomy `tests/AGENTS.md` defines; this
spec introduces no new top-level `tests/*.sh` gate (the closed-set rule
applies here exactly as everywhere else in the repository).

### Type 1 — eval cases (`tests/unit/nix/cases/`)

| Case | Asserts |
| --- | --- |
| `cutover-snapshot-retention-bounds.nix` | `d2b.site.cutoverSnapshotRetention` eval-rejects values outside `1..16`; defaults to `3` |
| `cutover-candidate-bundle-validation.nix` | A candidate `d2b.zones.<zone>.*` configuration with a dangling `*Ref` fails eval with a structured error naming the offending field |
| `zone-reset-scope-target-parsing.nix` | `--scope`/`--target` combinations for `zone`/`provider`/`guest` parse to the correct `ResourceRef` type; a `Zone/<name>` target under `--scope provider` is rejected |

### Type 2 — unit tests (`packages/<crate>/src/**`)

| Test | Asserts |
| --- | --- |
| `checkpoint_id` digest determinism | Same sorted inventory produces the same `checkpoint_id`; any single differing inventory element changes it |
| Journal resume logic | Given a journal with phases 0-4 `succeeded` and phase 5 `started` (no `succeeded`/`failed`), a resumed `apply` re-executes phase 5 from its own beginning, not phase 0 |
| Disposition framework invariant | Every disposition table entry compiles to exactly one of `Adopt`/`Preserve`/`Destroy`; a path absent from every table defaults to `Preserve` |
| TPM/durable-Volume Destroy exclusion | Property test: no code path can assign `Destroy` to a path tagged `kind: durable`-equivalent or TPM-marker-bearing in the inventory |

### Type 3 — integration tests (`packages/<crate>/tests/*.rs`)

| Test | Asserts |
| --- | --- |
| `cutover_preflight_refuses_dirty_flake_check` | `preflight` refuses with `cutover-precondition-failed` when the legacy configuration does not currently evaluate |
| `cutover_apply_requires_exact_consent_phrase` | `apply` refuses with `cutover-consent-required` for a missing, mismatched, or stale-checkpoint consent string |
| `cutover_drain_refuses_on_live_process` | Drain refuses with `cutover-precondition-failed` naming the exact stuck Guest/process rather than force-killing it |
| `cutover_rollback_window_closes_after_phase_5` | `rollback` succeeds for a checkpoint at phase `<=4` and refuses with `cutover-rollback-window-closed` at phase `>=5` |
| `host_reset_scope_isolation` | A `Provider reset` does not mutate the Zone resource, other Providers, or unrelated Guests; a `Guest reset` does not mutate the Zone, Providers, or other Guests |

### Type 4 — contract tests (`packages/d2b-contract-tests/tests/*.rs`)

| Test | Asserts |
| --- | --- |
| `cutover_snapshot_schema_matches_doc` | The rendered `snapshot.json`/`journal.jsonl` shapes match this spec's documented fields exactly (drift gate) |
| `cutover_plan_json_schema_v1` | `plan`'s JSON envelope matches the documented schema, frozen at version 1 |

### Type 5 — policy lints (`packages/d2b-contract-tests/tests/policy_*.rs`)

| Test | Asserts |
| --- | --- |
| `policy_no_destroy_without_gate` | Every `Destroy`-classified matrix row has a corresponding entry in [Old artifact/unit/schema removal gates](#old-artifactunitschema-removal-gates) |
| `policy_legacy_cli_verbs_absent_after_gate` | Compilation fails if any retired `cmd_vm_*`/`cmd_realm_*` dispatch entry is reintroduced after its Phase 10 gate is recorded clear |
| `policy_no_manual_chmod_chown_setfacl_text` | No cutover CLI human-output string contains `chmod`/`chown`/`setfacl` remediation text |

### Type 6 — flake checks (`tests/unit/smoke/`)

| Check | Asserts |
| --- | --- |
| `smoke-eval-cutover-candidate-bundle` | The candidate v3 Zone bundle for the repository's own example configurations (`examples/minimal`, `examples/multi-env`) evaluates and builds under the new `d2b.zones.*` namespace |

### Type 9 — container (`tests/integration/containers/`, `make test-integration`)

| Test | Asserts |
| --- | --- |
| `cutover-rehearsal-container.sh` | A rootless-podman fixture proves the Adopt migration `EphemeralProcess` pattern (prepare/stage/commit/rollback) against a synthetic swtpm-directory/store-view fixture without requiring a real Guest boot |

### Type 10 — VM/host-KVM (`tests/host-integration/*.nix`, `make test-host-integration`)

| Test | Asserts |
| --- | --- |
| `cutover-full-rehearsal.nix` | A real NixOS VM with one TPM-enabled Guest, one store-view Guest, and framework SSH keys: full Preflight→Verify cutover rehearsal, `verify` digest checks pass, old units retired only after their Phase 10 gates clear |
| `cutover-crash-resume.nix` | Kills the `apply` process mid-Phase-4 (mid-Adopt) and confirms a re-run resumes idempotently with no data loss and no duplicate Volume creation |
| `zone-provider-guest-reset-isolation.nix` | Full/Provider/Guest reset scope isolation proven against a live multi-Guest, multi-Provider Zone |
| `tpm-adopt-retirement.nix` | Phase 10 TPM gate: adopted TPM Volume survives a full Guest restart cycle before its source directory may be Destroyed |

### Type 11 — live-host (`tests/integration/live/`, `D2B_LIVE=1`, manual, never CI)

| Test | Asserts |
| --- | --- |
| `live/cutover-real-host.sh` | Full cutover against a real deployed pre-ADR-0046 host (never CI); requires explicit operator sign-off and a pre-existing full disk/VM backup outside this spec's own snapshot mechanism |
| `live/cutover-real-host-cloud-guest.sh` (manual cloud) | Cutover of a host with a live `runtime-azure-container-apps`/`runtime-azure-virtual-machine` Guest against real Azure resources; proves the gateway-custody boundary holds with a real relay-backed realm |

### Type 12 — hardware (`tests/host-integration/hardware/`, manual, real devices)

| Test | Asserts |
| --- | --- |
| `hardware/cutover-real-tpm.sh` | Cutover against a host with a real hardware-backed TPM passthrough Guest (not swtpm), proving the TPM Adopt path handles both the swtpm and hardware-TPM device-tpm Provider variants identically from the cutover's perspective |
| `hardware/cutover-real-usbip-security-key.sh` | Cutover against a host with a live USBIP/security-key attachment, proving Phase 8 device attachment reconciliation does not require detaching a physically-present device |

## Implementation work items

### ADR046-reset-001 — Inventory and snapshot engine

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reset-001` |
| Dependency/owner | W0 shared contract root; storage/broker integrator |
| Current source | `packages/d2bd/src/storage_lifecycle.rs` (`run_startup_contract_check`, bundle-versioned contract validation pattern); `packages/d2bd/src/ownership_preflight.rs` (`EntrySpec`, legacy-recovery-artifact optionality); `packages/d2b/src/lib.rs` `build_storage_migration_plan`/`storage_migration_checkpoint_id` |
| Reuse source | None from main; this is a v3-only cross-cutting concern with no main-branch equivalent |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-cutover/src/{inventory,snapshot,checkpoint}.rs` |
| Detailed design | Implement the seven closed inventories of [Authoritative inventories](#authoritative-inventories); the `checkpoint_id` digest algorithm of [Preflight and immutable snapshot](#preflight-and-immutable-snapshot); the atomic snapshot-write sequence (temp file, fsync, rename, parent fsync, post-rename immutability) |
| Integration | `d2b host cutover preflight`/`plan` CLI commands consume this crate exclusively; no other crate re-implements inventory walking |
| Data migration | New; no prior inventory/snapshot format exists |
| Validation | `checkpoint_id` determinism property test; snapshot atomic-write crash-injection test; `cutover_preflight_refuses_dirty_flake_check` |
| Removal proof | Not applicable (net-new capability) |

### ADR046-reset-002 — Config/artifact/schema validation

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reset-002` |
| Dependency/owner | ADR046-reset-001; `d2b-resource-store-redb` owner; `d2b-provider` catalog owner |
| Current source | `nixos-modules/bundle-artifacts.nix`, `nixos-modules/assertions.nix` (existing eval-time validation precedent); `ADR-046-nix-configuration` "Bundle and generation emission" |
| Reuse source | None from main |
| Reuse action | adapt |
| Destination | `packages/d2b-cutover/src/{bundle_validate,trust_preflight}.rs` |
| Detailed design | Independent legacy-flake-check gate; candidate v3 bundle schema/cross-ref/determinism validation per [Config/artifact/schema validation](#configartifactschema-validation); Provider trust preflight per `ADR-046-provider-model-and-packaging` "Trust" |
| Integration | Invoked by `preflight` before the snapshot is written; failures block `plan` from being offered |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `cutover-candidate-bundle-validation.nix`; trust-preflight rejection tests for each of digest/publisher/signature/deny/provenance/conformance failure modes |
| Removal proof | Not applicable |

### ADR046-reset-003 — Consent, drain, and disposition executor

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reset-003` |
| Dependency/owner | ADR046-reset-001, ADR046-reset-002; Process/Guest lifecycle owner |
| Current source | `packages/d2b/src/lib.rs` `require_explicit_mutation_flag`, `cmd_host_destroy` (dry-run/apply precondition pattern); [ADR 0040](../adr/0040-graceful-vm-shutdown.md) graceful shutdown path |
| Reuse source | None from main |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-cutover/src/{consent,drain,disposition}.rs` |
| Detailed design | Exact-consent-phrase gate bound to `checkpoint_id`; Phase 3 drain algorithm (§ [Old daemon/unit/process drain](#old-daemonunitprocess-drain)); the [Disposition framework](#disposition-framework)'s Adopt/Preserve/Destroy executor, delegating every Adopt to ADR046-reset-004 |
| Integration | `d2b host cutover apply` orchestrates drain then disposition execution then hands off to Phase 5 (ADR046-reset-005) |
| Data migration | Destructive; this is where Phase 3/4 boundary-of-no-return-approach begins (rollback still open through end of Phase 4) |
| Validation | `cutover_apply_requires_exact_consent_phrase`; `cutover_drain_refuses_on_live_process` |
| Removal proof | Not applicable |

### ADR046-reset-004 — Adopt migration EphemeralProcess integration

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reset-004` |
| Dependency/owner | ADR046-reset-003; `ADR-046-provider-state` owner; `device-tpm`/`volume-local` Provider owners |
| Current source | `packages/d2b-priv-broker/src/ops/swtpm_dir.rs` (marker fail-closed pattern); `packages/d2b-host/src/hardlink_farm.rs` (same-filesystem rename pattern) |
| Reuse source | None from main |
| Reuse action | adapt |
| Destination | `packages/d2b-cutover/src/adopt.rs`, thin wrapper invoking `ADR-046-provider-state`'s migration `EphemeralProcess` prepare/stage/commit/rollback machinery with cutover-specific source paths |
| Detailed design | One Adopt invocation per matrix row tagged `Adopt` in the [migration/disposition matrix](#migrationdisposition-matrix); marker re-validation before every step; new-marker-before-old-removal ordering; idempotent re-run safety per [Crash/power-loss/retry/idempotency journals](#crashpower-lossretryidempotency-journals) |
| Integration | Called by ADR046-reset-003's disposition executor for every Adopt row; writes to the state Volumes ADR046-device-tpm-004/ADR046-vl-004/ADR046-vl-006 define |
| Data migration | This work item *is* the data migration mechanism for TPM/store-view/disk-image/unsafe-local-scope bytes |
| Validation | Crash-injection at every step boundary (Type 10 `cutover-crash-resume.nix`); TPM/durable-Volume Destroy-exclusion property test |
| Removal proof | Not applicable (the mechanism is retained permanently for later Full/Provider/Guest reset relocation use, not retired after first use) |

### ADR046-reset-005 — Resource-store bootstrap and Provider install sequencer

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reset-005` |
| Dependency/owner | ADR046-reset-004; `d2b-resource-store-redb` owner (`ADR046-store-003`); core-controller owner (`ADR046-core-001`) |
| Current source | None (bootstrap sequencing over Zone runtime startup, which is itself ADR-only) |
| Reuse source | None from main |
| Reuse action | new |
| Destination | `packages/d2b-cutover/src/{store_bootstrap,provider_sequence}.rs` |
| Detailed design | Phase 5 store creation per [Resource-store initialization](#resource-store-initialization); Phase 6 topological Provider install per [Provider install/topological start](#provider-installtopological-start), including the fixed staged default order and cycle-rejection check |
| Integration | Invoked immediately after ADR046-reset-003/004 complete; hands off to Phase 7 (ADR046-reset-006) |
| Data migration | Destructive v3 bootstrap; no v2 resource import (per `ADR046-store-003`, `ADR046-object-001`) |
| Validation | Provider install topological-order determinism test; cycle-rejection test; store-identity mismatch fail-closed test |
| Removal proof | Not applicable |

### ADR046-reset-006 — Zone/ZoneLink/Guest activation and gateway custody boundary

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reset-006` |
| Dependency/owner | ADR046-reset-005; `ADR-046-zone-routing` owner; `ADR-046-resources-zone-control` owner |
| Current source | ADR 0032 gateway guest custody evidence (`d2b-realm-router/src/service_v2.rs` `CredentialCustody`); `ADR-046-zone-routing` §3 evidence |
| Reuse source | None from main |
| Reuse action | adapt |
| Destination | `packages/d2b-cutover/src/{zonelink_cutover,guest_activation}.rs` |
| Detailed design | Phase 7 ZoneLink translation from `EntrypointMode::GatewayBacked`/`HostResident` per [Zone/ZoneLink cutover](#zonezonelink-cutover); Phase 8 Network→Volume→Device→Guest ordering per [Guest/runtime/network/store view activation](#guestruntimenetworkstore-view-activation); enforcement that the parent inventory never enumerates gateway-guest-internal credential/audit state |
| Integration | Consumes Providers installed by ADR046-reset-005; hands off to ADR046-reset-007 (verification) |
| Data migration | None (ZoneLink resources are ordinary Nix-authored configuration, not migrated credential bytes) |
| Validation | Gateway-custody-boundary test asserting the parent inventory never contains a gateway-guest-internal path; ZoneLink `Degraded/waiting-on-remote` non-blocking test |
| Removal proof | Not applicable |

### ADR046-reset-007 — Verification, doctor, and degraded-ledger integration

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reset-007` |
| Dependency/owner | ADR046-reset-006; telemetry-audit-and-support owner (`d2b-audit`) |
| Current source | ADR 0034 degraded-state ledger taxonomy and repair-never-trusts-ledger-paths invariant |
| Reuse source | None from main |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-cutover/src/{verify,doctor,degraded}.rs` |
| Detailed design | The ten `verify` checks in [Post-cutover verification](#post-cutover-verification); the `cutover-quarantined` degraded class and `doctor` reporting in [Failure/quarantine/manual recovery](#failurequarantinemanual-recovery); audit chain closure/genesis-record cross-check (check 9) |
| Integration | `d2b host cutover verify`/`doctor` CLI commands; consumed by the Phase 10 finalize gate table |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | Injected-digest-mismatch test for TPM/durable-Volume verify checks; audit-genesis-cross-check test; `cutover-full-rehearsal.nix` |
| Removal proof | Not applicable |

### ADR046-reset-008 — Old artifact/unit/schema removal gate engine (Phase 10)

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reset-008` |
| Dependency/owner | ADR046-reset-007; owner of each retiring artifact's ADR 0046 successor work item |
| Current source | `ADR-046-cli-and-operations` "Removal notes" (live-successor-before-deletion criterion) |
| Reuse source | None from main |
| Reuse action | new |
| Destination | `packages/d2b-cutover/src/finalize.rs` |
| Detailed design | Per-candidate independent gate evaluation exactly as tabled in [Old artifact/unit/schema removal gates](#old-artifactunitschema-removal-gates); separate consent phrase from `apply`; never partial-destroys a candidate |
| Integration | `d2b host cutover finalize` CLI command; reads gate status from ADR046-reset-007's verify results plus each named policy-lint/integration test's pass/fail recorded in CI |
| Data migration | This work item is where every previously-Preserved legacy artifact is finally Destroyed, one gate at a time |
| Validation | `policy_no_destroy_without_gate`; `policy_legacy_cli_verbs_absent_after_gate`; `tpm-adopt-retirement.nix` |
| Removal proof | Each candidate's own row in [Old artifact/unit/schema removal gates](#old-artifactunitschema-removal-gates) states its exact removal proof |

### ADR046-reset-009 — Rollback, journal resume, and incident hold

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reset-009` |
| Dependency/owner | ADR046-reset-003 through ADR046-reset-006; `ADR-046-provider-state` incident-hold owner |
| Current source | ADR 0034 "dry-run and preflight output print the checkpoint id and exact rollback command before any apply step begins" |
| Reuse source | None from main |
| Reuse action | adapt |
| Destination | `packages/d2b-cutover/src/{journal,rollback,hold}.rs` |
| Detailed design | Append-only journal per [Crash/power-loss/retry/idempotency journals](#crashpower-lossretryidempotency-journals); [Rollback boundary](#rollback-boundary) enforcement (`cutover-rollback-window-closed` past phase 4); cutover-wide incident hold per [Incident hold (cutover-wide)](#incident-hold-cutover-wide) |
| Integration | `d2b host cutover rollback`/`hold` CLI commands; consulted by ADR046-reset-003's disposition executor and ADR046-reset-008's finalize gate before every mutating step |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `cutover_rollback_window_closes_after_phase_5`; incident-hold-blocks-destructive-step test |
| Removal proof | Not applicable |

### ADR046-reset-010 — Full/Provider/Guest reset CLI and scope isolation

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reset-010` |
| Dependency/owner | ADR046-reset-005 through ADR046-reset-007; `ADR-046-resources-zone-control` owner (§2.6/§9.4) |
| Current source | `ADR-046-resources-zone-control` §2.6 (`core.zone-drain` finalizer algorithm), §9.4 (out-of-band destructive reset, uid=0 authentication) |
| Reuse source | None from main |
| Reuse action | adapt |
| Destination | `packages/d2b-cutover/src/reset_scope.rs`; `d2b host reset` CLI command |
| Detailed design | The three reset scopes and their comparison table in [Full Zone reset vs Provider reset vs Guest reset](#full-zone-reset-vs-provider-reset-vs-guest-reset); durable-Volume preserve-by-default with explicit `--destroy-durable-volumes`/`--destroy-volumes` opt-in; OS-level authentication for the zone scope only |
| Integration | Standalone from the cutover Phases 0-10 above; usable at any later time as a recovery/maintenance lever once a Zone exists |
| Data migration | None (this is a post-cutover recovery operation, not part of the cutover data migration itself) |
| Validation | `host_reset_scope_isolation`; `zone-provider-guest-reset-isolation.nix`; durable-Volume-preserved-by-default property test for both Provider and Guest scopes |
| Removal proof | Not applicable |

### ADR046-reset-011 — Live-host and hardware validation

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reset-011` |
| Dependency/owner | ADR046-reset-001 through ADR046-reset-010, fully landed |
| Current source | `tests/integration/live/` conventions; `tests/host-integration/hardware/` conventions |
| Reuse source | None from main |
| Reuse action | new |
| Destination | `tests/integration/live/cutover-real-host.sh`, `tests/integration/live/cutover-real-host-cloud-guest.sh`, `tests/host-integration/hardware/cutover-real-tpm.sh`, `tests/host-integration/hardware/cutover-real-usbip-security-key.sh` |
| Detailed design | Manual, `D2B_LIVE=1`/hardware-gated validation scripts described in [Tests](#tests) Type 11/12 rows; never run in CI; require operator sign-off and an independent out-of-band backup before execution |
| Integration | Run manually by an operator against a real host/device before the reset-and-cutover implementation is declared production-ready |
| Data migration | None (validation only) |
| Validation | Manual pass/fail sign-off recorded per the project's existing live-host/hardware validation conventions |
| Removal proof | Not applicable |
