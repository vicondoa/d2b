# Implementation Plan: Complete the ADR-046 Provider Control Plane (d2b 3.0)

**Branch**: `001-adr046-d2b3-completion` | **Date**: 2026-07-29 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/001-adr046-d2b3-completion/spec.md`

## Summary

Turn the ADR-046 foundation delivered in waves W0 and W1 - currently 52k lines of
deliberately test-only, production-unwired code that no shipped binary depends on - into a
live Zone-scoped resource control plane, replace the pre-ADR-046 control plane through a
one-time destructive cutover, and release the result as d2b 3.0.

The architecture is already defined by the accepted ADR-046 member specifications. This plan
organizes implementation around the live Zone resource plane, the one-time destructive cutover,
provider/runtime integration, focused evidence, and the d2b 3.0 release. It does not re-derive
architecture or create a second source of truth.

**Current installed-host bootstrap state: BLOCKED.** Committed protocol 4 has no host-generation
handoff operation, while the source generation broker service executes its own broker package.
The target closure therefore cannot be a supervised compatibility actor before profile publication.
The implementation must add the already-defined handoff contract without a new unit, runtime
override, child process, mutating entrypoint, or daemon recovery owner. Numeric protocol 4
without the negotiated fingerprint refuses.

## Technical Context

**Language/Version**: Rust from `packages/rust-toolchain.toml` (currently 1.97.0, with
`rustfmt` and `clippy`); Nix for the NixOS module surface. The pin, not this plan, is
authoritative for the compiler version used by enforcing gates.

**Primary Dependencies**: redb `=4.1.0` is a direct dependency of the production
`packages/d2b-resource-store-redb` backend. The disposable
`proofs/redb-resource-store-spike` workspace separately retains its provisional `=4.1.0`
pin and remains quarantined under D128; that quarantine does not apply to the production
crate. Other primary dependencies are ttrpc/protobuf for the resource service, Noise
handshakes for ComponentSession, and Cloud Hypervisor and crosvm as runtime backends. No
new toolchain, linter, formatter, or nixpkgs overlay is introduced.

**Storage**: One embedded redb database per Zone, opened by owned fd, with full crash-safe
durability - one fsync per write transaction, no reduced-durability mode. Write queue 256,
group-commit batch 16, read pool 4, concurrent reads 16, read lifetime 250 ms.

**Testing**: Existing closed layer set - nix-unit eval cases, Rust unit and binary integration
tests, rendered-artifact contract tests, policy lints, and flake checks at Layer 1; podman
containers and `runNixOSTest` at Layer 2; hardware, live-host, and cloud tiers manual. No new
top-level shell gate. Every heavy lane runs through the two-slot semaphore via its public
`make` target.

**Target Platform**: `x86_64-linux` NixOS host with KVM, single trusted user. Graphics paths
are x86_64-only by existing platform gate.

**Project Type**: NixOS module framework plus a multi-crate Rust control plane (58 workspace
members at committed HEAD c758a377703c523edd88a987e48a6f30034e1912, plus two deliberately
excluded standalone workspaces)

**Performance Goals**: Empty-store readiness <=500 ms; p95 local Get and bounded List <=2 ms;
p95 crash-safe single-resource mutation <=10 ms; p95 durable commit to controller handler
start <=5 ms; p95 ready Process commit to launch-attempt start <=20 ms

SC-002 performance and evidence semantics are owned by the feature specification and its
focused implementation evidence. This plan does not copy the clock, receipt, digest,
census, publication, or recovery protocol.

**Constraints**: Whole-process RSS <=24,576 KiB with **no baseline subtraction** - historical
production fixtures passed at their recorded tips. The former final-F measurement
plan is historical and is not rerun; aggregate idle
RSS <=64 MiB; per-component budgets 22 MiB for `Provider/system-core` and 12 MiB for
`Provider/system-minijail`; per-Provider-crate hermetic suite aggregate process-CPU p95 <=3 s

**Scale/Scope**: 27 Provider crates; 19 standard ResourceTypes; hard fixtures at 10,000
resources and 100 concurrent watches

## Constitution Check

*Constitution review: verify the following boundaries while implementing.*

| Principle | Assessment | Status |
| --- | --- | --- |
| **I. Daemon-Only Control Plane** | ADR-046 adds per-Zone runtimes as **parent-spawned processes**, not PID1 units, and DELETEs the three per-realm units. Unit count does not grow; the `systemctl list-units` exit criterion is unchanged. Restart remains a continuation event via FR-003. | PASS (see research R5) |
| **II. Broker-Mediated Audited Privilege** | FR-012 keeps every privileged host mutation on the audited broker path; D077 forbids any Provider process importing the broker, enforced by a policy lint. FR-070 adds a daemon-owned resource-mutation audit drainer, not a new service, and requires audit durability before success. `SO_PEERCRED` plus group membership at the public socket stays the sole initial local lifecycle authz surface and is never treated as a Resource API subject. Host-generation continuation consumes a sealed durable capability minted only after that classification; daemon identity, broker-socket credentials, and euid 0 never independently authorize. | PASS |
| **III. Reasonable Isolation Over Convenience** | FR-009 default-denies cross-Zone reference; FR-014 fails closed on missing identity state rather than reinitializing; FR-066 requires authoritative registrar-derived subjects; FR-069 forbids partial publication; FR-071 isolates a failed Zone without making it ready. virtiofsd zero-capability and per-VM store-farm invariants are untouched. | PASS |
| **IV. Contract-Driven Compatibility** | 3.0 is a deliberate major-version clean break with v3 schemas, versioned artifacts, and fail-closed drift gates (FR-031). The Zone desired-state schema is unchanged. | PASS |
| **V. Test-Layer Discipline** | FR-032 pins coverage to the lowest hermetic layer and forbids a new top-level shell gate; FR-029 routes every heavy lane through the single semaphore; FR-033 retires superseded suites. | PASS |
| **VII. Traceable, Marker-Free Shipped Artifacts** | SC-018 requires release notes to omit internal implementation bookkeeping; FR-019 lands docs with their behavior. | PASS |

**Gate result**: The implementation plan is ready for focused component validation. Broad
container, host, live, and hardware lanes are conditional on the changed surface.

**Execution model**: Work may proceed in parallel where files are disjoint, with each change
validated in its owning scope and integrated through the normal pull-request protections.
Heavy validation uses the public semaphore-protected targets.

### C1 correction and version impact

The accepted provider-system-core member specification currently uses
`system_core_host` and `system_core_user` both for internal telemetry labels and for the
serialized `Zone.status.handlers` names, while the committed
`packages/d2b-contracts/src/v3/zone.rs` closed enum uses kebab-case wire serialization.
Authoritative member specifications resolve that defect in favor of the committed serialization rule: the only serialized Zone
handler names are `system-core-host` and `system-core-user`. The underscore spellings remain
internal closed telemetry-label values only and MUST NOT appear in serialized
`Zone.status.handlers[]`. The authoritative prospective contract adds `ZoneHandlerName::SystemCoreHost` and
`ZoneHandlerName::SystemCoreUser`; readiness consumes exactly one status record for each.
`ProviderLifecycle` remains a separate aggregate enum value and cannot satisfy either record.

The owning technical specifications define the `Version` metadata and correct
their handler-name language in the same commit as the Rust enum, unit/serialization and
closed-list tests, the existing lowest-layer contract/policy guard, and
`docs/reference/resource-plane-runtime.md`. No `apiVersion`, JSON `schemaVersion`,
`manifestVersion`, or `bundleVersion` bump is made because no desired-state field or
ResourceType schema changes. The generated
`docs/reference/schemas/v3/core.d2bus.org_Zone.schema.json` remains byte-identical after its
existing generator and drift gate run. Emitter and consumer ownership follows the
committed source and owning contracts.

## Project Structure

### Documentation (this feature)

```text
specs/001-adr046-d2b3-completion/
├── plan.md              # This file
├── research.md          # Phase 0 output - resolves R1-R7, records RK-1..RK-6
├── spec-coverage.md     # Phase 1 output - requirement coverage and focused evidence
├── data-model.md        # Phase 1 output - Zone/Resource model and the 19 ResourceTypes
├── quickstart.md        # Phase 1 output - operator validation runbook
├── deferred-findings.md # Historical bounded-deferral compatibility record
├── friction-log.md      # Historical implementation friction and technical findings
├── contracts/           # Phase 1 output - the contract surfaces this program must deliver
│   ├── README.md
│   ├── resource-api.md
│   ├── operator-cli.md
│   ├── nix-configuration.md
│   ├── generated-artifacts.md
│   └── companion-contracts.md
|-- checklists/
|   |-- requirements.md  # Historical specification-quality checklist
|   \-- coverage.md      # Historical coverage notes; not an implementation gate
\-- tasks.md             # Implementation task list
```

## Specification coverage and the no-detail-loss rule

The ADR-046 set is the design. This plan organizes implementation without restating it or
creating a second source of truth.

**Feature artifacts are authoritative for requirements.** `tasks.md` maps implementation items
 to their owning specification, destination, and focused validation. It does not replace the
normative member specifications, and no auxiliary planning artifact controls release
eligibility.

**Why this plan does not inline the spec text.** Copying design and validation prose into
planning artifacts would create a second source of truth. Instead, `spec-coverage.md` records
the member specifications, implementation ownership, cross-cutting obligations, hard numeric
targets, and deletion obligation, and closes with a detail-preservation checklist.

Every implementation item in `tasks.md` must map to a requirement in the feature artifacts.

### Source Code (repository root)

The program writes into the existing tree. Paths below are the real destinations named by the
task list and owning contracts, grouped by implementation sequence.

```text
packages/
├── d2b-contracts/src/v3/          # resource-composition stage adds host, guest,
│                                  #   execution_policy, process, volume, user, network,
│                                  #   device, credential, zone_routing, zone_session
├── d2b-resource-store/            # engine-neutral contract (foundation, present)
├── d2b-resource-store-redb/       # runtime stage adds actor, transaction, revision_log, backup,
│                                  #   migration - the corrected production engine
├── d2b-resource-api/              # runtime stage adds watch.rs; registration path spans
│                                  #   the resource-composition and runtime stages
├── d2b-controller-toolkit/        # foundation present; runtime stage adds the real-backend
│                                  #   reaction benchmark
├── d2b-core-controller/           # resource-composition stage adds zone_links.rs, configuration.rs
├── d2b-session/  d2b-session-unix/ d2b-bus/
│                                  # foundation present; resource-composition stage adds bus
│                                  #   session/, transport/, zone_route.rs, relay.rs
├── d2b-zone-routing/              # resource-composition stage - engine, resolver, service,
│                                  #   vectors, benches
├── d2b-resource-client/           # resource-composition stage
├── d2b-provider/  d2b-provider-toolkit/
│                                  # adapts in place; provider-packaging stage owns the contract
├── d2b-process/  d2b-provider-supervisor/
│                                  # runtime stage (not resource composition; see drift note below)
├── d2b-provider-system-{core,systemd,minijail}/
├── d2b-provider-volume-{local,virtiofs}/
├── d2b-provider-network-local/  d2b-provider-credential-*/  d2b-provider-device-*/
│                                  # schema halves span the resource-composition and runtime
│                                  #   stages; implementations follow their component dependencies
├── d2b-telemetry/  d2b-audit/     # runtime stage
├── d2b/                           # operator CLI
└── xtask/                         # schema and Nix-option generators

nixos-modules/
├── options-zones.nix              # present; resource-composition stage restructures it
│                                  #   as the generated base
├── generated/                     # NEW - resource-types.nix, options-zones-<Type>.nix
├── zone-resources-json.nix        # resource-composition stage
├── resources-*.nix                # per-ResourceType emitters across implementation stages
├── assertions.nix                 # resource-composition stage adds Zone assertions (single writer)
└── bundle-artifacts.nix           # resource-composition stage adds the per-Zone row

docs/
├── reference/schemas/v3/          # NEW - per-ResourceType JSON schemas
├── reference/                     # per-behavior docs land with the behavior (FR-019)
└── specs/ADR-046-*                # normative member specifications and architectural rationale

proofs/redb-resource-store-spike/  # disposable; hosts the RSS correction prototype (RK-1)
tests/                             # extends existing closed layer set; no new top-level gate
```

**Structure Decision**: No new top-level structure is introduced. The program extends the
existing `packages/` workspace, `nixos-modules/`, `docs/reference/`, and `tests/` trees at the
exact destinations identified by implementation items. New crates follow the established
`d2b-provider-<base>-<implementation>` layout.

### Implementation sequencing

| Increment | Scope | Parallelism | Focus |
| --- | --- | --- | --- |
| Foundations | Resource model, routing, provider packaging, controllers, and sandbox | File-disjoint groups where possible | Preserve ownership boundaries and focused contract evidence |
| Runtime | Production store/watch plane and Provider families | Provider-family parallelism | Complete authenticated runtime reachability and host continuity where applicable |
| Cutover | Cutover, security, feasibility, and compatibility | Independent component scopes | Complete removal proofs and release compatibility |
| Release | Friction closure and publication preparation | Conditional on remaining findings | Resolve user-visible release and documentation friction |

Implementation increments describe dependency order for the task list. Each change is
validated against its owning requirements and integrated through normal repository protections.


### Production data flow and ownership

1. The broker resolves the opaque Zone store id and returns the owned database descriptor.
   The Zone runtime verifies immutable store and Zone identity, then reads mutable policy,
   active-configuration, and controller revisions from durable state. Reopen never supplies
   mutable revisions from constants.
2. `ZoneResourceRuntime` is the single lifecycle owner of the Zone policy. On initial install
   and every restart it owns one private, sealed, non-`Clone`, non-`Copy`, one-shot
   `PolicyBootstrapRead` minted only by one private issuer. It has no public constructor,
   conversion, `Default`, field, accessor, capability trait implementation, or reconstruction
   path. After immutable Zone/store identity is verified, that capability
   reads only the Zone's policy-input envelopes at the exact live durable nonzero revision.
   It carries no Resource API subject, has no general read or mutation method, and is consumed
   by the one installation attempt. `d2b-resource-api` compiles those envelopes into the
   first immutable `PolicySet` and installs it in `NativeAuthorizer`; redb never parses an
   RBAC DTO. Missing, stale, cross-Zone, or invalid input consumes the attempt and leaves the
   Zone unpublished and degraded. After installation, policy reads and revision advances
   use only the authenticated Resource API. A committed new revision is compiled completely
   before atomic replacement, and readiness advances only when the installed revision equals
   live durable metadata. This bootstrap-to-normal transition breaks the startup cycle
   without weakening authentication or D106.
3. The registrar consumes verified transport evidence into one ComponentSession, derives the
   authoritative subject from registrar-private state, and registers both ResourceService and
   the controller endpoint on the exact ZoneBus route. Unix admission obtains the peer process
   descriptor directly from the accepted socket with `SO_PEERPIDFD`; opening a pidfd later from
   the `SO_PEERCRED` numeric PID is forbidden because PID reuse can redirect that lookup. The
   kernel floor must provide `SO_PEERPIDFD`; unavailable support, a non-`CLOEXEC` descriptor,
   or any mismatch between `SO_PEERCRED` and the credential, process-generation, cgroup, or
   liveness evidence verified against that exact pidfd refuses admission. A daemon restart
   acquires a new peer pidfd from the newly accepted socket and never revives persisted
   numeric-PID evidence. The public daemon bridge may request registration but may not
   construct or pass a subject claim. `VerifiedUnixPeer` exposes no credentials or evidence
   accessor, `ZoneBootstrapIdentity` exposes no public issuer, constructor, verifier, clone, or
   identity accessor, and one registrar-private issuer consumes the complete pidfd evidence.
   The session adapter, descriptor, bus Unix transport, and session seam consume that same
   accepted-socket evidence and expose no caller-supplied verifier or credential constructor.
   Peer-pidfd acquisition uses T592's typed
   `OpenPeerPidfdFromAcceptedSocket` broker operation. The daemon transfers only the accepted
   Unix socket with `SCM_RIGHTS`; the request carries no raw descriptor number, credential
   tuple, or numeric PID, and the response returns only an `OwnedFd` pidfd with
   `FD_CLOEXEC` over `SCM_RIGHTS`. Both ancillary receive paths - broker receipt of the
   accepted socket and daemon receipt of the returned pidfd - call `recvmsg` with
   `MSG_CMSG_CLOEXEC`, reject `MSG_CTRUNC`, parse the complete control-message set, take
   ownership of every received fd immediately, require exactly one fd of the expected type,
   verify `FD_CLOEXEC`, and close every fd on count, type, index, decode, or later validation
   failure. An unexpected extra fd is never ignored. Descriptor-count and exec probes cover
   success, malformed payload, missing fd, extra fds, truncated control data, and errors
   after fd receipt. The sole raw `getsockopt(SO_PEERPIDFD)` call is consolidated
   in the already approved `packages/d2b-priv-broker/src/sys.rs` FFI quarantine. It uses a
   narrow item-level unsafe allowance and a `SAFETY:` justification on every unsafe block,
   passes and validates exact `optlen`, assumes ownership of every nonnegative returned fd
   before checking the syscall result or later invariants, and closes it on every short,
   oversized, malformed, syscall, missing-CLOEXEC, or later failure without assert, panic, or
   leak. The `nix` 0.31.3 `PeerPidfd` `MaybeUninit`/assert wrapper, a new repository-authored
   FFI crate, and any local `d2b-session-unix` syscall fallback are ineligible.
4. The admitted ResourceService watch opens through that registered route, replays from the
   durable checkpoint without a replay/live gap, and feeds the registered controller fan-in.
   Before any EffectPort call, the core controller records an outstanding effect in the
   existing per-Zone durable store through the engine-neutral store contract. The core
   controller alone interprets ledger bytes. The key binds Zone, controller generation,
   resource UID, committed revision, operation id, and effect ordinal. Restart adopts or
   idempotently replays pending entries before cleanup. Cleanup completion is a compare against
   the same UID and exact nonzero revision.
5. The Zone runtime owns the mutation-audit drainer. The same redb transaction that commits
   each privileged mutation also creates its immutable authoritative journal rows, one per
   mutation ordinal; export completion is separate mutable state and can never delete or
   rewrite an unexported row. Audit constructors accept typed fixed 32-byte,
   domain-separated digests for operation, correlation, subject, Zone, resource, replay
   binding, and any retained trace correlation; no constructor accepts their raw string
   equivalents. Raw values stay only in bounded private operation/replay state with redacted
   `Debug`, and raw trace context is excluded from authoritative rows and exports. The sole
   output exception is a direct Version 2 operator CLI/JSON status or recovery response,
   which may return bounded `zoneRef` and `operationId` values supplied or received by that
   operator as recovery coordinates. Those fields never become telemetry labels, span
   attributes, exported audit identities, or unrelated error context. The
   unprivileged Zone runtime owns the drain state machine, but every root-owned filesystem
   effect crosses one typed broker op carrying only fixed-digest records and bounded rotation
   policy. The root broker is the sole `SegmentWriter` owner and holds the root-owned segment
   directory fd; segment append, rotation, export, and prune use fd-relative
   `openat2`/`openat`/`unlinkat`, never joined paths. No service or unit is added. Export
   completion may advance only after the broker response proves the segment file and its
   directory have both been `fsync`ed and the opened segment inode has been revalidated. A
   normal successful mutation response is
   released only after the required append-only segment export and its completion state are
   durable. If export remains incomplete after commit, the API returns
   semantic `CommittedPendingAudit` through the layered `ResourceStatus` composite:
   `ResourceStatus.phase` is
   `ResourcePhase::Degraded`; `ResourceStatus.outcome.code` is
   `StatusCode("committed-pending-audit")` with retryable, safe remediation and no raw sink
   detail; `ResourceStatus.update.state` is `UpdateState::Blocked`; and
   `ResourceStatus.update.operation_id` is `Some(original_operation_id)`. Existing bounded,
   redacted condition, outcome, and update fields carry only safe same-ID retry/status
   instructions. The additive protobuf `PendingAuditStatus` field makes that composite
   representable on every mutation response, including `DeleteResponse`; it changes the
   ResourceService schema fingerprint but no Resource JSON `apiVersion` or `schemaVersion`.
   The result neither reports ordinary success nor implies rollback. The Zone is unpublished
   and degraded until export recovery. Same-ID observation or resumption first matches a
   persisted replay-binding digest over the registrar-derived subject, Zone, semantic request,
   target, verb, expected revision, and idempotency data. A mismatch is denied and audited.
   An exact retry returns the pending or one stored final result without reapplication; a
   different ID follows normal revision/conflict semantics. A typed `InspectOperation`
   ResourceService request/response carries this lookup through the store, generated protobuf
   and ttrpc bindings, method catalogue, authorization/router, daemon client, and CLI; no
   in-memory-only or CLI-local status path is eligible. Operation identity is exactly
   `(Zone, operation_id)`: inspection requires the Zone, the same opaque ID may be used
   concurrently in different Zones, and no host-global index exists. The 16 bytes use UUIDv7
   layout and lowercase 32-hex rendering. Checked issuance time plus the fixed 30-day
   operation recovery retention defines `expiresAt`; the per-Zone durable retention clock never moves
   backwards. Malformed, future, expired, overflowed, or clock-discontinuous IDs deny before
   observation or mutation, and pruning cannot make an old ID reusable. Restart deduplicates by fixed operation
   digest plus mutation ordinal and produces one logical exported record. The one
   configuration carrier is compiler-only `d2b.zones.<zone>.audit`, emitted as the
   required top-level `audit` object in that Zone's `resource-bundle.json`, outside every
   ResourceSpec and the controller-created empty `Zone.spec`. `audit.retentionDays`, default
   30 and range 1 through 3650, governs both exported segments and journal rows, but a journal
   row becomes prune-eligible only after durable export completion plus that retention
   interval. `audit.maxRecordsPerSegment`, default 65536 and range 1 through 1000000, and
   `audit.maxSegmentBytes`, default 67108864 and range 1048576 through 1073741824, bound
   rotation. This header change moves the only accepted resource-bundle pair from
   `schemaVersion: 3` / `bundleVersion: 1` to `schemaVersion: 4` /
   `bundleVersion: 2`; v4 `contentHash` covers canonical `{audit,resources}`, so an audit-only
   change cannot reuse a generation identity. Missing, old/mixed, malformed, misplaced, or
   unenforceable policy and any journal or segment prune failure produce typed degraded Zone
   health and block publication.
   Every sensitive audit or broker DTO and owner uses fixed redacted `Debug`, including
   `StoreSyncRequest`, `StoreSyncResponse`, the drain request, dispatcher errors,
   `SegmentWriter`, sink, exporter, root directory owner, and opaque storage handle owner.
   StoreSync wire fields, producers, consumers, schemas, and snapshots use only sealed typed
   digests or opaque handles. Present trace context becomes only its typed domain-separated
   digest, absence stays absent, and malformed input is denied before mutation; another
   digest class is never fabricated or relabelled as trace correlation.
   Metrics and OTEL resource attributes carry no raw or digested Zone, resource, operation,
   correlation, or trace identity. Logs and spans carry a typed digest only where correlation
   is required by the accepted contract. T182-T205 remain blocked until accepted
   the telemetry/audit contract and its associated product tasks are updated to remove
   raw identity fields and assign the redaction/cardinality matrix. The existing audit and
   telemetry owners perform the hashing; no secrets service or new runtime boundary is added.
6. One readiness projection is computed from store recovery, policy match, authenticated
   session/router admission, controller registration, watch admission, audit catch-up,
   mandatory controller health, and the `d2b-core-controller`-owned
   `Provider/system-core` registration. The minimum Provider handler set is the active,
   initialized, current `HostReconciler` and `UserReconciler`, observed through exactly one
   `Zone.status.handlers[]` record named `system-core-host` and exactly one named
   `system-core-user`. Each record carries `phase` and `lastReconciledAt`.
   `ProviderLifecycle` is a distinct aggregate handler name and cannot substitute for either
   record. Provider readiness is determined by the owning runtime contract, and no duplicate,
   missing, wrong-name, boolean, or detached-status substitute may satisfy this member. No
   component publishes itself. Startup and close collect per-Zone outcomes and visit every
   Zone; a missing or unhealthy system-core registration/handler degrades only that Zone and
   never aborts or silently drops later owners.
7. Installed-host migration begins with the target closure's
   `system.build.d2bHostGenerationDeploy` entrypoint and one durable, replayable handoff
   binding source/target system, broker, daemon, numeric protocol, negotiated operation
   catalogue fingerprint, bundle-pointer,
   complete bundle-set, stable-reference, and deployment-intent digests. The first 3/1-to-4/2
   migration obtains that entrypoint from an explicit target installable and never reads a
   target-generated stable file. Unprivileged resolution produces exactly one canonical Nix
   store output and caller-flake executable. Before authorization succeeds, the broker
   verifies that target object, creates a broker-managed GC root, and durably pins its
   canonical store identity, NAR hash, deployment-executable digest, and staged intent. It
   separately resolves only trusted installed-generation metadata and pins the canonical
   store identity, NAR hash, and executable digest of one broker-managed apply object. The
   caller-flake executable performs the unprivileged authorization only. Privileged apply
   invokes only the separately pinned installed apply object and performs no Nix eval, build,
   `nix run`, installable resolution, or symlink lookup; the broker reopens both pins and
   refuses any target/apply substitution, changed symlink target, digest mismatch, missing GC
   root, or cross-intent replay before mutation. On the accepted apply connection the broker
   obtains a connection-scoped peer pidfd directly from the accepted socket, binds the live
   peer's executable store/NAR/digest identity to the apply-object pin, and revalidates
   liveness, process start identity, and current executable identity immediately before each
   mutation. Peer exit, exec, PID reuse, mismatch, or ambiguous identity refuses. After the first durable mutation, apply-peer transition membership, ordered mutation edges,
   expected cases, meta-poisons, raw-identity canaries, and digest vectors are owned only by
   generated `VD2-SC002-REGISTRIES` and `VD2-SC002-TRACEABILITY` rows assigned to T589,
   T592, and T595. This plan copies no ids, counts, fixture contents, or transition matrix.
   The selected refusal still leaves that mutation and every successor unexecuted, preserves
   the durable prefix and audit, closes transient fds, and emits no raw peer identity.
   Missing, stale, runtime-derived, skipped, wrong-owner, non-ancestor, or failing generated
   coverage blocks evidence acceptance.
   Because `d2b host-generation apply-authorized-handoff` carries neither an intent selector nor an authority
   token, the source broker keeps exactly zero or one durable nonterminal handoff intent per
   source generation. Authorization and apply selection share the coordinator's exclusive
   lock. Authorization refuses while any authorized, claimed, mutating, recovery-pending, or
   transfer-pending intent exists. Apply selects only the sole `authorized-pending` intent
   and atomically claims it for the kernel-derived connection identity; zero, multiple, and
   already-claimed intents refuse without waiting or choosing by age. A pre-mutation
   disconnect can release the claim only after a durable zero-mutation proof. After mutation,
   only durable coordinator replay of the same intent can accept a replacement connection
   from the same pinned apply object after the old peer is proven dead. Terminal intents are
   never selected or replayed by a later command.
   T595's unprivileged selector-free
   `d2b host-generation inspect-authorized-handoff [--json]` exposes the exact
   `HostGenerationHandoffStatusV1` projection from `data-model.md`. Every
   `recovery-pending` row names whether the live apply peer, source broker, or target broker
   owns progress, one closed wait/restart-existing-broker action, and the exact allowed
   successors. `recovery-irreconcilable` is valid only when immutable pre-mutation/outcome
   audit proves one complete rollback; restarting the existing broker unit drives only that
   rollback to `rolled-back`. Root inspection, an intent selector, a path, a token, a daemon
   recovery owner, or a new unit is forbidden. Human/JSON status contains only bounded
   state/phase/owner/action/successor enums and no identity, generation, path, or apply-peer
   canary.
   The entrypoint may build and verify the target closure, durably stage immutable bytes, and
   submit one opaque intent; it cannot publish a profile, control a service, initiate
   rollback, or select a path, unit, generation, command, or argv.
   A protocol-5 installed broker accepts the typed request after transfer. Before transfer,
   only the externally installed source-generation compatibility peers may receive exactly
   one accepted public-socket evidence fd after their authenticated daemon/broker Hello has
   matched both numeric protocol 4 and Hello `operation_catalogue_sha256` equal to the exact
   `source-handoff-v1` operation-catalogue fingerprint. Bare committed protocol 4 omits the
   field or advertises a different catalogue and refuses; it
   cannot route authority to a target-closure mode. Before fd transfer, the accepted external floor must pass its generated
   `VD2-SC002-SOURCE-FLOOR`, `VD2-SC002-REGISTRIES`, and
   `VD2-SC002-TRACEABILITY` rows. Those rows solely own membership, receipts, signatures,
   fixtures, poison cases, and transitions; this plan copies none of them. A missing, stale,
   wrong-owner, non-ancestor, or failing row refuses before fd transfer or mutation. The installed source receiver derives
   authority only from that consumed fd and its broker-sealed staged-intent binding. It
   accepts no serialized uid, gid, role, provenance, root, daemon, or caller claim.
   Before either broker acts, the unprivileged operator must pass the existing public-socket
   `SO_PEERCRED` plus current `d2b`-group Admin classification. The broker consumes that
   one-shot evidence into one durably sealed, nonfabricable capability bound to the complete
   staged intent and pinned store object; process/socket credentials, daemon identity, Hello,
   target-closure provenance, and bootstrap euid 0 are integrity or eligibility inputs only.
   The installed source broker before transfer and target broker after transfer advance a
   phase only by consuming that capability or a
   broker-issued phase attenuation and emit immutable pre-mutation and outcome audit rows for
   every system-profile, service, bootstrap, publication, repair, and rollback phase.

   The existing `d2b-priv-broker.service`, reached through
   `d2b-priv-broker.socket`, is the sole executable lifecycle owner before and after transfer.
   Its existing `Restart=on-failure` path starts or restarts the externally installed source
   broker, whose ordinary `serve` startup reopens the durable coordinator; the deployment
   entrypoint and `d2bd` never supervise it,
   and no transient, template, path, timer, or additional service is created. The broker
   durably owns the coordinator before the first mutation, including the exact lifecycle
   owner and pinned bootstrap generation. The installed source broker retains that ownership until the
   authenticated target protocol-5 broker durably adopts it exactly once. Target broker
   activation and durable coordinator transfer precede target daemon activation. Killing the
   entrypoint or compatibility process at any pre-transfer boundary must cause the existing
   broker unit to reopen the same coordinator and resume or roll back idempotently. The target
   daemon starts, completes fresh exact-generation protocol-5 Hello while explicitly unready,
   then presents a phase attenuation in the authenticated opaque publication request. Only
   after the broker durably publishes and audits the matching d2b pointer and stable reference
   may daemon ingestion and readiness proceed. On failure the broker-owned coordinator, not
   the entrypoint, daemon, or a new supervisor, reopens the durable handoff. Before transfer,
   only the capability-authorized compatibility mode under the existing broker service may
   resume or roll back its phase. After
   transfer, the existing `d2b-priv-broker.service` reopens the coordinator after broker
   restart and completes or rolls back even when target daemon startup or reconciliation
   fails. The broker restores
   prior pointer/reference bytes or verified absence before performing typed stock rollback
   and source-service restoration. Runtime refusal carries only
   `rebuild-host-generation`. Documentation gives fail-closed parameterized
   authorization/apply pairs for first bootstrap, stable-reference use, and rollback; every
   preflight stops before public-socket authorization or `sudo`, every privileged apply names
   the already authorized exact store executable, and runtime output contains neither command
   nor reference. Runnable documentation redirects Nix evaluation and build stderr directly
   to `/dev/null`; it creates no diagnostic file and emits only the fixed stage-specific
   `fail` literals shown in `quickstart.md`. The production entrypoint may retain at most
   16,384 stderr bytes in memory, never on disk; overflow is a fail-closed stage failure, all
   raw bytes are dropped before return, and only the closed identifier-free typed
   failure/remediation is emitted. Canary
   tests require raw evaluator/builder stderr to be absent from human, JSON, wire, log, audit,
   metric, span, and `Debug` output.

The concrete failures this permits are a committed generation whose process dies after its
effect intent becomes durable but before the effect completes, and an audit segment export
that fails after the mutation and its immutable authoritative journal row commit together.
The durable ledger makes the first recoverable; the operation-bound pending-audit result makes
the second observable without lying about success or rollback. The restart crash-window
matrices catch a lost, duplicated, ambiguous, or stale effect/export record, while the
transactional journal prevents an unaudited committed privilege change. The aggregate
readiness projection prevents the recovered store from becoming success-shaped while policy,
route, watch, audit export, controller, or the exact system-core Provider ownership is absent.

Apply-peer, source-floor, and unit-fixture registries are not copied into this plan. Their
independently authored fixture sets and enforcing coverage contracts remain authoritative.
Missing, duplicate, stale, wrong-owner, non-ancestor, or non-enforcing coverage fails closed;
production discovery cannot define expected test membership.


### Recorded corrections and drift

The plan follows committed code where prose and implementation differ. The retained corrections
are implementation constraints:

- The `ZoneHandlerName` wire values are `system-core-host` and `system-core-user`; underscore
  spellings remain internal telemetry labels. Update the enum, serialization tests, API
  snapshots, policy guard, and reference status surface together. No desired-state schema or
  manifest version bump is needed.
- Network east-west access uses the double-opt-in expression
  `Network.spec.isolation.allowEastWest && d2b.site.allowUnsafeEastWest`, with both inputs
  defaulting to false. The four Network/Host combinations remain covered by focused tests.
- Host-generation handoff remains broker-only and fail-closed. Protocol 4 requires the exact
  negotiated handoff fingerprint; no target-only actor, new unit, caller-selected executable,
  or euid-0 shortcut substitutes for the handoff contract.
- Resource-store crates remain policy-neutral. The Resource API and Zone policy owner interpret
  policy DTOs; store validation remains limited to policy-neutral envelope and structural rules.
- The production audit writer remains in the session/runtime owner. Provider crates do not gain
  direct audit or telemetry dependencies merely to emit authoritative records.
- Removal proofs follow the path that actually performs a removal. Existing provider/runtime
  consumers determine whether a crate is retained or retired; destination names are resolved
  from committed code and the owning specification.
- The corrected RSS measurement and destination-drift amendments remain in their dedicated
  feature artifacts. This plan does not rewrite historical measurements or edit generated
  artifacts by hand.

### Program-local safety and delivery risks

The recovery-point requirement is an explicit safety control.

| Risk | Why Tracked | Guard and Rejected Alternative |
| --- | --- | --- |
| FR-043 (recovery-point evidence) must not allow a partial, old, wrong-host, or unverifiable point to become success-shaped. | The external operator-owned backup/snapshot and restore mechanism remains outside this feature; no host implementation is claimed. | T548 owns one hermetic validator used unchanged by T580, T555, and T556. It decodes timestamps through a bounded integer newtype, uses checked expiration arithmetic, requires `previewed <= captured <= verified <= attested <= verifier-now < expires`, independently varies every receipt field and binding including operator and restore-instruction digests, and fails on listing failure, empty discovery, ignored tests, or skip. T580 accepts only one external version 1 record for a verified full-host snapshot or backup covering boot/system state, the active generation, the preview inventory, and preserved identity state. It binds candidate/commit/tree, preview, daily-driver host, operator, and restore instructions; imports only its digest and opaque locator through the existing `EvidenceRecord`; and rejects negative, fractional, future, out-of-range, overflow, stale, expired, or mismatched values. Every cutover stage invokes the same validator; expiry requires fresh evidence and reconvergence before cutover can proceed. |
