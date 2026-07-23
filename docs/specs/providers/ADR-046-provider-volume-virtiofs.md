# ADR 0046 Provider dossier: volume-virtiofs

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-volume-virtiofs` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-volume-virtiofs` crate, volume-virtiofs controller, virtiofsd worker, attachment lifecycle |
| Depends on | `ADR-046-resources-volume`, `ADR-046-provider-model-and-packaging`, `ADR-046-components-processes-and-sandbox`, `ADR-046-provider-state`, `ADR-046-componentsession-and-bus`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-nix-configuration`, `ADR-046-telemetry-audit-and-support` |
| Supersedes | `nixos-modules/processes-json.nix` virtiofsdRunner block; `nixos-modules/minijail-profiles.nix` virtiofsdProfiles; `packages/d2b-host/src/virtiofsd_argv.rs`; `ProcessRole::Virtiofsd` dag nodes in `packages/d2bd/src/supervisor/dag.rs` |
| ADR 0021 | Accepted invariant; fully governs virtiofsd sandbox; no exception or partial closure permitted |

---

## 1. Purpose

This dossier exhaustively specifies `Provider/volume-virtiofs` — the d2b v3 controller
that reconciles virtiofs attachments declared in Volume resources and owns every virtiofsd
worker Process. It is the authoritative reference for:

- the crate/package/provider identity and required crate layout;
- the controller component descriptor and watch plan;
- the owned virtiofsd worker Process template;
- the ADR 0021 broker-pre-established user-namespace invariant, enforced in full;
- zero host capability classes and `startRoot: false`;
- the `--sandbox=chroot` / `--inode-file-handles=never` / `--readonly` argv contract;
- single-writer and shared-write enforcement;
- per-attachment status, guest-mount readiness, export-socket privacy;
- the store-view farm attachment;
- two-phase attachment creation and two-phase attachment deletion;
- Nix authoring, canonical ResourceSpec JSON, eval/build validation, and cleanup;
- d2b-bus access and RBAC;
- broker operations;
- status/errors/audit/telemetry/performance budgets;
- exact implementation work items, test file layout, and removal proofs.

`Provider/volume-virtiofs` implements Volume attachment lifecycle only. It does not implement
layout provisioning, ACL reconciliation, or store management. Those belong to
`Provider/volume-local` (see `ADR-046-resources-volume`).

---

## 2. Crate and package identity

| Field | Value |
| --- | --- |
| Crate path | `packages/d2b-provider-volume-virtiofs/` |
| Crate name | `d2b-provider-volume-virtiofs` |
| Provider resource name | `Provider/volume-virtiofs` |
| `artifactId` key | `volume-virtiofs-provider` |
| Package type | `provider` |
| ResourceTypes implemented | `Volume` (attachment lifecycle and status; no layout) |
| Attachment transports owned | `virtiofs` |
| Dependencies | `d2b-contracts` (v3 Volume/Process types), `d2b-provider-toolkit` (ResourceClient, reconciler, fake seams), `d2b-session`, `d2b-bus`, `d2b-audit`, `d2b-telemetry` |
| Prohibited imports | `d2bd`, `d2b-priv-broker` internals, `d2b-provider-volume-local`, any other Provider's implementation |

### Required crate layout

```text
packages/d2b-provider-volume-virtiofs/
  src/
    main.rs / lib.rs          controller binary entry point
    controller.rs             volume-virtiofs-controller reconcile loop
    attachment.rs             attachment lifecycle state machine
    virtiofsd_argv.rs         argv generation (reuse from d2b-host/src/virtiofsd_argv.rs)
    socket_path.rs            private per-attachment socket path derivation
    readiness.rs              export socket and guest-mount readiness probes
    user_ns.rs                ADR 0021 user-namespace spec generation
    metrics.rs                bounded telemetry labels
    audit.rs                  volume-virtiofs audit record types
    error.rs                  typed error catalog
    tests/                    (colocated unit tests — allowed by workspace policy)
  tests/
    argv_golden.rs            migrated and extended virtiofsd_argv unit tests (≥14 tests)
    attachment_lifecycle.rs   create / ready / delete lifecycle
    adr021_invariant.rs       ADR 0021 rejection tests
    single_writer.rs          single-writer enforcement
    shared_write.rs           shared-write capability gate
    readonly_flag.rs          --readonly per access mode
    multi_attachment.rs       multi-attachment process isolation
    socket_path_privacy.rs    socket path never-in-status invariant
    schema_conformance.rs     ResourceType/controller/fault/redaction conformance
    fake_port.rs              fake-core/bus/supervisor seam tests
  integration/
    README.md                 integration fixture index and run instructions
    virtiofsd_launch/         virtiofsd process launch fixture
    guest_mount_readiness/    guest-control health probe fixture
    finalizer_drain/          finalizer drain under Guest restart
    store_view_readonly/      ro-store attachment with shared-dir=store-view/live
  README.md                   Provider identity, config schema, ResourceTypes, controllers/
                               workers, placement, deps/RBAC, ADR 0021 summary, socket
                               privacy, security invariants, state/telemetry, build/test/
                               integration commands, standalone-repo consumption
```

Workspace policy rejects a Provider crate missing any of `src/`, `tests/`, `integration/`,
or `README.md`. This is enforced by the workspace crate-layout gate
(`packages/xtask/src/workspace_policy.rs`).

---

## 3. Provider resource spec

### 3.1 Canonical Provider ResourceSpec

```yaml
apiVersion: resources.d2b.io/v3
type: Provider
metadata:
  name: volume-virtiofs
  zone: dev
  uid: <store-generated>
  generation: 1
  revision: <opaque>
  ownerRef: null
  finalizers: []
  deletionRequestedAt: null
  createdAt: 2026-07-22T00:00:00Z
  updatedAt: 2026-07-22T00:00:00Z
spec:
  artifactId: volume-virtiofs-provider
  config: {}           # no root config; all settings live in Volume attachment spec
  resourceTypes:
    - Volume
  controllerComponents:
    - id: volume-virtiofs-controller
      type: controller
      resourceTypes: [Volume]
      supportedAttachmentTransports: [virtiofs]
      domain: system
      cardinality: one-per-zone
      processTemplate: volume-virtiofs-controller
  workerTemplates:
    - id: virtiofsd-worker
      binary: virtiofsd
      domain: system
      sandbox:
        userNamespace:
          singleEntry: true
          principalSource: per-attachment-vfd-user
        capabilityClasses: []
        startRoot: false
        seccompClass: w1-virtiofsd
status:
  observedGeneration: 1
  phase: Ready
  conditions: []
  lastReconciledAt: 2026-07-22T00:00:01Z
```

No root config is validated (empty `config: {}`). Every per-attachment option is declared
inside the Volume spec's `attachments[*].settings` object and validated against the Provider's
signed `attachment.schema.json` at Nix eval time.

### 3.2 Nix artifact catalog entry

```nix
d2b.artifacts."volume-virtiofs-provider" = {
  package = pkgs.d2b-provider-volume-virtiofs;
  type    = "provider";
};
```

The store path is private catalog implementation data. It never appears in any ResourceSpec,
status field, or audit record.

### 3.3 Nix Provider installation

```nix
d2b.zones."dev".resources."volume-virtiofs" = {
  type = "Provider";
  spec = {
    artifactId = "volume-virtiofs-provider";
    config = {};
  };
};
```

`spec.artifactId` must exist in `d2b.artifacts` with `type = "provider"`. A missing or
wrong-type entry aborts the Nix build with a structured error naming the Provider and the
missing catalog ID.

---

## 4. Volume attachment reconciliation

### 4.1 Attachment spec fields (virtiofs)

An attachment in a Volume spec selects `transport: virtiofs` and a named View:

```yaml
attachments:
  - executionRef: Guest/work-vm
    transport: virtiofs
    view: controller
    access: read-write          # read-only | read-write | shared-write
    mountPath: /state
    settings:
      posixAcl: false
      xattr: false
      cache: auto               # auto | always | never
      inodeFileHandles: never   # never | prefer | mandatory
      threadPoolSize: null      # null → resolved from target Guest vcpu count
      socketGroup: null         # null → broker-default (runner gid)
```

| Field | Type | Required | Default | Constraints |
| --- | --- | --- | --- | --- |
| `executionRef` | ResourceRef | Yes | — | Resolves to a `Guest/<name>` in the same Zone; same Zone required |
| `transport` | enum | Yes | — | `virtiofs` for this Provider |
| `view` | ViewName | Yes | — | Must exist in the Volume's `views` map at Volume creation time |
| `access` | enum | No | `read-only` | `read-only`, `read-write`, or `shared-write`; must be compatible with the named View's declared rights |
| `mountPath` | absolute path | Yes | — | Guest-side VirtIO-FS mount path; must not overlap with other mounts on the same Guest |
| `settings.posixAcl` | bool | No | `false` | Passes `--posix-acl` to virtiofsd; omitted for store-view shares |
| `settings.xattr` | bool | No | `false` | Passes `--xattr` to virtiofsd |
| `settings.cache` | enum | No | `auto` | `auto` \| `always` \| `never`; maps to `--cache=<mode>` |
| `settings.inodeFileHandles` | enum | No | `never` | `never` \| `prefer` \| `mandatory`; `never` is the default and the only value tested for current shares |
| `settings.threadPoolSize` | int or null | No | `null` | `null` resolves to the target Guest's declared vcpu count at reconciliation time; range 1–256 |
| `settings.socketGroup` | int or null | No | `null` | `null` uses the broker-default gid (volume-virtiofs-worker principal gid); explicit value must be an authorized group ID |

### 4.2 Single-writer constraint

At most one attachment per Volume may carry `access: read-write` at any moment. The controller
enforces this before creating or accepting any update that would introduce a second concurrent
`read-write` attachment.

- On attempt to add a second `read-write` attachment while one is active: the controller
  writes a status condition `SingleWriterViolation` and returns `ResourceConflict` to the
  caller. The Volume phase transitions to `Degraded`.
- The constraint applies across all attachment types for the same Volume, not per-Guest.
- `access: shared-write` is a distinct mode. It requires the Provider descriptor to declare
  `supportsSharedWrite: true` in its capabilities. Write-ordering semantics for
  `shared-write` are the caller's responsibility. `Provider/volume-virtiofs` does not
  declare `supportsSharedWrite: true` in v3.0; attempting `shared-write` in v3.0 returns a
  `CapabilityUnsupported` error.

### 4.3 Two-phase attachment creation

```text
Phase 1 — virtiofsd Process bootstrap
  Volume spec mutation arrives (new or changed attachment entry)
  → core delivers owned-resource-changed hint to volume-virtiofs controller
  → controller reads current Volume spec / attachment list
  → controller computes or updates the virtiofsd Process resource for the attachment
  → controller emits ResourceMutationBatch: Create (or UpdateSpec) for Process/vol-<vol>-virtiofsd-<guest>
  → system-minijail Process controller receives spec-generation-changed hint
  → system-minijail issues LaunchTicket to ProviderSupervisor
  → ProviderSupervisor calls broker SpawnRunner (virtiofsd-worker template)
  → broker pre-establishes user namespace (ADR 0021); execve virtiofsd
  → virtiofsd binds export socket at the private path
  → virtiofsd Process status → phase: Ready

Phase 2 — guest mount confirmation
  volume-virtiofs controller polls export socket existence (unix-socket-exists readiness kind)
  → socket present → attachment exportReady: true
  → Guest Provider mounts virtiofs at mountPath in the guest
  → guest-control health probe returns MountReady
  → controller writes attachment status guestMountReady: true
  → Volume AttachmentsReady condition → "True"
  → Volume phase → Ready
```

Neither phase blocks the other attachment reconciliations running for the same Volume.
Each attachment's virtiofsd Process lifecycle is independent.

### 4.4 Two-phase attachment deletion

```text
Phase 1 — virtiofsd Process teardown
  Attachment removed from Volume spec (or Volume deletion requested)
  → owned-resource-changed hint to volume-virtiofs controller
  → controller emits ResourceMutationBatch: Delete for the virtiofsd Process
  → system-minijail sends SIGTERM to virtiofsd; waits for process exit via pidfd
  → on process exit: store emits one Deleted revision event; row and index removed atomically
  → export socket file is removed (socket was created by virtiofsd; it exits cleanly)
  → controller sets attachment state → detaching

Phase 2 — guest mount absent confirmation
  volume-virtiofs controller queries guest-control health: is mountPath still mounted?
  → guest-control probe returns MountAbsent
  → controller clears volume-virtiofs/attachments finalizer for this entry
  → all attachment entries cleared → finalizer volume-virtiofs/attachments removed
  → Volume deletion proceeds to volume-local finalizer
```

The controller does not forcibly unmount a guest filesystem. If the Guest is unreachable
(e.g., shut down), the health probe returns a timeout and the controller transitions the
attachment to `Unknown`. The attachment finalizer remains held — `Degraded/Unknown` — until
the Guest runner becomes reachable again and the probe confirms `MountAbsent`, or until a
full Zone reset. If Guest runner absence can be positively proved (the process that owns the
Guest mount namespace is confirmed dead via pidfd and the mount namespace is therefore gone),
the controller clears the finalizer with that proof recorded in the audit record. There is no
time-based force-clear; ambiguity keeps the finalizer.

---

## 5. Owned virtiofsd Process template

### 5.1 Process resource shape

Each virtiofs attachment owns exactly one virtiofsd Process resource. The resource is named
`vol-<volume-name>-virtiofsd-<guest-name>` and carries `ownerRef: Volume/<volume-name>`.

```yaml
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: vol-work-state-virtiofsd-work-vm
  zone: dev
  uid: <store-generated>
  generation: 1
  ownerRef: Volume/work-state
  finalizers: [system-minijail/process]
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  processClass: worker
  template: virtiofsd-worker
  sandbox:
    userNamespace:
      hostUid: <resolved stable UID of User/vol-work-state-vfd>
      hostGid: <resolved stable GID of User/vol-work-state-vfd>
    namespaceClasses: [user]       # user namespace only; no mount/pid/net classes
    capabilityClasses: []          # zero host capability classes; full caps inside NS only
    seccompClass: w1-virtiofsd
    startRoot: false               # broker does NOT start virtiofsd as root
    noNewPrivileges: true          # PR_SET_NO_NEW_PRIVS before exec (required when startRoot=false)
    readOnlyRoot: true             # rootfs mounted read-only inside the user namespace
  readiness:
    initialDelay: "0s"
    timeout: "30s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined        # ProviderSupervisor checks unix-socket-exists; path in LaunchTicket only
  budget:
    cpu:
      request: "50m"
      limit: "500m"
    memory:
      request: "32Mi"
      limit: "128Mi"
    pids:
      limit: 64
    fds:
      limit: 256
  mounts: []
status: {}
```

Private implementation data that lives exclusively in the LaunchTicket and broker state,
never in the Process spec, status, or any public surface:
- the export socket path (derived in `socket_path.rs`; passed opaquely in the LaunchTicket);
- the cgroup subtree placement (assigned by the ProviderSupervisor from the executionRef and
  Provider component placement template);
- raw host paths for the Volume View FD (received as an `OwnedFd` via `ProvideFdToWorker`;
  never a path string in spec or status);
- the `readOnlyPaths`/`writablePaths` mount policy (compiled by system-minijail from the
  `namespaceClasses` and sandbox plan; not inline spec fields).

### 5.2 Zero host capability classes (ADR 0021)

The `sandbox.capabilityClasses` field is always `[]`. virtiofsd obtains a full capability set
**inside the user namespace**, where those capabilities are scoped to the namespace and
confer no authority on the host.

Any change to this Process template that introduces a non-empty `sandbox.capabilityClasses`
list or sets `sandbox.startRoot: true` violates ADR 0021 and must be rejected by:

1. the workspace policy gate (`tests/unit/nix/cases/broker-caps.nix`);
2. the `minijail-validator-virtiofsd` gate (`tests/tools/gen-migration-ledger.sh`);
3. the hermetic Rust test `adr021_invariant.rs::virtiofsd_capability_classes_must_be_empty`.

`--sandbox=namespace` is never emitted. The only accepted sandbox mode is `--sandbox=chroot`.

### 5.3 Broker-pre-established user namespace (ADR 0021)

Before virtiofsd's first instruction, the broker performs:

```text
broker:
  sync_pipe = pipe2(O_CLOEXEC)
  outcome = clone3({
      flags: CLONE_NEWUSER | CLONE_PIDFD,
      #       ^CLONE_NEWNS is intentionally absent; created lazily after sync
  })
  if outcome.is_child:
    close(sync_pipe.write_fd)           # prevent self-deadlock if broker dies
    read(sync_pipe.read_fd, 1 byte)     # blocks until parent writes uid_map
    prctl(PR_SET_NO_NEW_PRIVS, 1)
    # CLONE_NEWNS is not in clone3 flags; mount NS created here if required:
    # (not required for virtiofsd — no mount NS isolation needed)
    setgid(0)                           # in-NS GID 0 → host_gid_for_zero
    setuid(0)                           # in-NS UID 0 → host_uid_for_zero
    # setgroups() SKIPPED — parent wrote setgroups=deny
    # supplementary groups MUST be empty (preflight enforces)
    capset(full_caps_inside_ns)
    execve(virtiofsd_binary, argv, env)
  else:  # parent
    write("/proc/<child_pid>/uid_map",   "0 <host_uid_for_zero> 1\n")
    write("/proc/<child_pid>/setgroups", "deny")
    write("/proc/<child_pid>/gid_map",   "0 <host_gid_for_zero> 1\n")
    close(sync_pipe.read_fd)
    write(sync_pipe.write_fd, 1 byte)   # unblock child
    return pidfd
```

Parent write ordering is strict: `uid_map` → `setgroups=deny` → `gid_map`. This matches
`man 7 user_namespaces`: writing `gid_map` requires either `CAP_SETGID` in the parent or
`setgroups=deny` first. volume-virtiofs uses `setgroups=deny` defensively.

`CLONE_NEWNS` is intentionally absent from the `clone3` flags. Mount namespace, when needed,
is created via `unshare(CLONE_NEWNS)` inside the child **after** the sync-pipe read unblocks,
i.e. after in-NS root is acquired. Doing `CLONE_NEWNS` before sync would fail because the
child has not yet acquired in-NS root via uid_map. virtiofsd does not require a mount
namespace for its `--sandbox=chroot` operation; `--sandbox=chroot` uses `pivot_root(2)` with
`CAP_SYS_ADMIN` inside the user NS.

The mapping is single-entry: in-NS UID/GID 0 → the stable UID/GID of `User/vol-<vol>-vfd`.
Only that single mapping is written. All other host UIDs are unmapped (overflow `65534`).

This is acceptable for all virtiofs shares:
- `/nix/store` is content-addressed and world-readable; guest does not need root-UID
  ownership semantics.
- Per-VM state Volumes are owned by the runner principal; that maps to in-NS 0 correctly.
- Read-only shares (`access: read-only`) carry `--readonly` and guests observe correct
  mode bits without needing true UID fidelity beyond the runner principal.

If a future share requires UID-preserving semantics for arbitrary host UIDs (e.g., a
`/home/<user>` mount), a multi-entry mapping is necessary. That is out of v3.0 scope and
requires a new ADR section and a separate work item.

### 5.4 Dedicated per-Volume principal

Each Volume that has at least one virtiofs attachment receives a dedicated system User
resource `User/vol-<volume-name>-vfd`. The volume-virtiofs controller creates this User
resource when the first virtiofs attachment is added and it does not already exist. The
User resource is owned by the Volume (`ownerRef: Volume/<name>`).

The User resource provides:
- a stable UID and GID allocated by the system-core User controller;
- the `hostUidForZero` / `hostGidForZero` values for the user-namespace mapping;
- the `--socket-group` gid for the export socket (or the resolved `socketGroup` if
  explicitly configured in attachment settings).

The gctl share for guest-control (`d2b-gctl`) uses a separate narrower principal
`User/vol-<vol>-gctlvfd` per ADR 0021 §"d2b-gctl guest-control token share". The
volume-virtiofs controller selects the principal by share type.

---

## 6. virtiofsd argv contract

### 6.1 Canonical argv shape

```text
virtiofsd
  --socket-path=<private-derived-path>
  --socket-group=<resolved-gid>
  --shared-dir=<volume-view-root-fd-path>
  --thread-pool-size=<N>
  --sandbox=chroot
  --inode-file-handles=never
  --cache=<mode>
  [--posix-acl]           # present only if attachment settings.posixAcl == true
  [--xattr]               # present only if attachment settings.xattr == true
  [--readonly]            # present only if access: read-only
```

No `--sandbox=namespace` is ever emitted.
No `--inode-file-handles=always` or `--inode-file-handles=prefer` is emitted in v3.0.
No free-form `extraArgs` pass-through is accepted; root config is empty.

### 6.2 `--socket-path` — private derived path

The export socket path is a **private implementation detail of volume-virtiofs**. It is:

- derived deterministically as:
  ```text
  <zone-runtime-dir>/vms/<guest-name>/vol-<sha256_trunc8(zone+volume+guest)>.vfd.sock
  ```
  where `sha256_trunc8` is the first 8 hex characters of the SHA-256 of the
  concatenated canonical form `<zone-name>\x00<volume-name>\x00<guest-name>`.
- no longer than 108 bytes (kernel `sun_path` limit);
- under `/run/d2b/vms/<guest-name>/` (the Zone/Guest runtime directory; path row
  `path:vm-run:<vm>` in the current storage.json migration table);
- never written to Volume spec, Volume status, process spec, process status, audit records,
  CLI output, telemetry labels, or log messages;
- unlinked when the virtiofsd process exits (by virtiofsd itself on clean exit; by the
  controller's socket cleanup step on unclean exit).

The ProviderSupervisor resolves the socket path from the LaunchTicket at launch time only.
It does not pass the path to the caller after launch; it signals readiness to the controller
via the `unix-socket-exists` readiness predicate only (not by returning the path).

### 6.3 `--shared-dir` — volume root FD path

The controller does not pass a raw host filesystem path as `--shared-dir`. It receives an
`OwnedFd` from the broker that opens the volume root directory. The argv generator
uses `/proc/self/fd/<N>` as the `--shared-dir` value so that virtiofsd inherits the open
FD; the path is never a literal filesystem path visible in any public surface.

For the store-view Volume, `--shared-dir` resolves to the `live/` subdirectory of the
hardlink farm (`store-view/live`) via the `ro-store` View. virtiofsd is **never** pointed
at `/nix/store` directly. `share.source == "/nix/store"` is the Nix eval-time sentinel that
triggers store-view substitution in the resource compiler; the running virtiofsd process sees
only `store-view/live`.

### 6.4 `--thread-pool-size`

`settings.threadPoolSize == null` (the default) causes the controller to read the target
Guest's declared `spec.vcpus` at reconciliation time and use that value. If the Guest spec
has not been reconciled yet, the controller requeues the Volume reconciliation with a short
exponential backoff and does not launch virtiofsd until the Guest vcpu count is known.

### 6.5 `--readonly`

`--readonly` is emitted when:
- `access: read-only` is declared on the attachment; OR
- the named View's `rights` do not include `write`.

It is NOT emitted for `access: read-write` or `access: shared-write` attachments.

### 6.6 Baseline source and migration

The current baseline is `packages/d2b-host/src/virtiofsd_argv.rs`:
- `VirtiofsdArgvInput` (11 fields): socket_path, socket_group, shared_dir,
  thread_pool_size, sandbox, inode_file_handles, cache, posix_acl, xattr,
  readonly, extra_args.
- `generate_virtiofsd_argv(input: &VirtiofsdArgvInput) -> Vec<String>` (14 unit tests;
  pinned golden `argv.txt` lines 166–184).

The 14 existing unit tests migrate verbatim to
`packages/d2b-provider-volume-virtiofs/tests/argv_golden.rs`. The `extra_args` field is
removed in v3 (Provider root config is empty; no free-form arg injection). A new test
`no_extra_args_ever_emitted` is added.

---

## 7. Per-attachment status

Volume status (`status.attachmentStatuses`) carries one `AttachmentStatus` per declared
attachment:

```yaml
attachmentStatuses:
  - executionRef: Guest/work-vm
    transport: virtiofs
    virtiofsdProcessRef: Process/vol-work-state-virtiofsd-work-vm
    exportReady: true
    guestMountReady: true
    state: attached         # attached | attaching | detaching | failed | unknown
    phase: Ready
    conditions:
      - type: ExportReady
        status: "True"
        reason: socket-exists
        observedGeneration: 1
      - type: GuestMountReady
        status: "True"
        reason: health-probe-ok
        observedGeneration: 1
    lastCheckedAt: 2026-07-22T00:00:01Z
```

| State | Meaning |
| --- | --- |
| `attaching` | virtiofsd Process not yet Ready; export socket absent |
| `attached` | virtiofsd Ready; export socket present; guest mount confirmed |
| `detaching` | Delete requested; virtiofsd Process terminating or guest unmounting |
| `failed` | virtiofsd Process failed; export socket never appeared before timeout |
| `unknown` | Guest unreachable; health probe timed out |

The virtiofsd export socket path is **never** a field in `AttachmentStatus`. Its presence
or absence drives `exportReady` as a boolean. Callers observe readiness state, not the path.

### 7.1 Guest mount readiness

Guest mount readiness is observed via the guest-control health protocol
(`ADR-046-resources-host-guest-process-user` §"Guest control health"). The controller sends
a `VirtioFsMountReady?` probe to the guest-control vsock endpoint of the target Guest. The
guest-side health handler responds with `MountReady` when the kernel has completed the
VirtIO-FS mount at `mountPath`, or `MountAbsent` when it has not.

If the Guest is off or the vsock endpoint is unreachable, the probe returns a timeout error.
The controller sets `guestMountReady: false`, `state: unknown`, and schedules a retry.

`guestMountReady: true` requires:
1. `exportReady: true` (socket is present);
2. `virtiofsdProcessRef` Process phase is `Ready`;
3. the guest-control health probe returned `MountReady`.

---

## 8. Store-view farm attachment

The per-Guest closure-only Nix store hardlink farm is served via virtiofs. The store-view
Volume is declared with:

- `Provider/volume-local`, `kind: durable`, `source.settings.kind: local-path`;
- `source.settings.hostPath` = `<storeStateDir>/<guest>/store-view` (injected by the
  resource compiler; never in spec/status/audit);
- `views.ro-store = { path: "live", rights: ["read", "traverse"] }`;
- one attachment with `transport: virtiofs`, `view: ro-store`, `access: read-only`,
  `mountPath: /nix/.ro-store`, `settings.posixAcl: false`, `settings.xattr: false`.

volume-virtiofs is responsible for the virtiofsd Process that serves `store-view/live`.
It is not responsible for populating the hardlink farm (that is volume-local's store-view
mode).

Key invariants enforced by the controller:

1. **`--shared-dir` = `store-view/live`**: virtiofsd is always pointed at the `live/`
   subdirectory, never at `/nix/store`. The `ro-store` View has `path: "live"` and the
   controller resolves the FD to that subtree.
2. **`--readonly` always emitted**: the `ro-store` View rights are `[read, traverse]` (no
   `write`); `access: read-only`; `--readonly` is unconditional for this attachment.
3. **`--posix-acl` and `--xattr` omitted**: `/nix/store` paths have no POSIX ACLs and d2b
   hardlink farms are d2b-managed; these flags are not needed.
4. **Marker file prerequisite**: virtiofsd is not started until
   `store-view/live/.d2b-marker-<guest>` exists (zero-length file, `d2bd:users 0444`). This
   is the readiness marker that confirms the hardlink farm has been populated. The controller
   checks for the marker via a bounded blocking adapter (e.g., `tokio::task::spawn_blocking`
   wrapping an `fstatat(2)` relative to the Volume root `OwnedFd`, or an async-safe
   fd-relative equivalent); no blocking syscall is issued on the async executor thread
   directly. If the marker is absent, the controller requeues with exponential backoff.

---

## 9. d2b-bus routing and RBAC

### 9.1 Bus route

volume-virtiofs controller processes connect to the Zone resource API over:

```text
volume-virtiofs-controller (Process/vol-*-virtiofsd-*)
  → d2b-bus (local enrolled KK session)
  → ComponentSession (Noise_KK, authenticated as Provider/volume-virtiofs controller subject)
  → Zone d2b.resource.v3 service
  → redb coordinator
```

The controller never receives a direct redb handle, store path, or ambient socket.
It uses only the ResourceClient from `d2b-provider-toolkit` over the bus-provided route.

### 9.2 Broker operations

| Op | Trigger | Fields logged (audit) |
| --- | --- | --- |
| `SpawnRunner` (virtiofsd-worker template) | virtiofsd Process LaunchTicket | Volume UID digest, attachment executionRef digest, worker template name |
| `VirtiofsdLaunch` | post-spawn readiness phase | Volume UID, attachment executionRef digest, result class |
| `ProvideFdToWorker` | volume-local shares Volume root FD | Volume UID, view name, worker Process UID digest |
| `TerminateProcess` | attachment deletion | Process UID digest, reason class |

The raw socket path, host path, and principal UID are never logged in any broker audit field.

### 9.3 RBAC

The controller is authorized by its enrolled KK identity as `Provider/volume-virtiofs`
controller. Required Role rules:

```yaml
# volume-virtiofs controller
rules:
  - resourceTypes: [Volume]
    verbs: [get, list, watch, update-status, update-finalizers]
    zones: [<zone>]
  - resourceTypes: [Process]
    verbs: [create, get, list, watch, update-spec, delete]
    zones: [<zone>]
    ownerConstraint: owned-by-volume-virtiofs-controller
  - resourceTypes: [User]
    verbs: [create, get, list, watch]
    zones: [<zone>]
    namePattern: vol-*-vfd
    ownerConstraint: owned-by-volume-virtiofs-controller
```

The controller may not write Volume spec. Status and finalizers for Volume resources use
a separate status-subresource authorization. The `volume-virtiofs/spawn-virtiofsd`
permission claim gates broker `SpawnRunner` for virtiofsd; this claim is granted only to
`Provider/volume-virtiofs` controller processes.

The `hostPath` field in `Volume.spec.source.settings` is read by volume-local only, not by
volume-virtiofs. volume-virtiofs receives only the validated `OwnedFd` for the View root
via the `ProvideFdToWorker` broker op; it never reads `source.settings` directly.

---

## 10. Controller component descriptor

```yaml
id: volume-virtiofs-controller
type: controller
providerId: volume-virtiofs
resourceTypes:
  - type: Volume
    verbs: [update-status, update-finalizers, watch]
  - type: Process
    verbs: [create, update-spec, delete, watch]
  - type: User
    verbs: [create, watch]
watchSelectors:
  - resourceType: Volume
    filter: attachments[*].transport == "virtiofs"
  - resourceType: Process
    filter: ownerRef starts-with "Volume/"
    ownerType: Volume
  - resourceType: User
    filter: name starts-with "vol-" and name ends-with "-vfd"
ownerChildTriggers:
  - trigger: owned-resource-changed
    ownerType: Volume
    childTypes: [Process, User]
dependencySelectors:
  - resourceType: Guest
    purpose: vcpu-count-resolution
  - resourceType: User
    purpose: vfd-principal-uid-resolution
reconcileConcurrency: 16          # 16 parallel attachment reconciliations
maxPendingResources: 1024
observeIntervalSeconds: 0         # event-driven only; no periodic re-scan for virtiofsd
finalizers:
  - volume-virtiofs/attachments
serviceFingerprint: <sha256 of attachment.schema.json>
```

---

## 11. Error catalog

| Error code | Meaning | Retryable |
| --- | --- | --- |
| `virtiofsd-launch-failed` | broker SpawnRunner returned non-zero or clone3 failed | yes, with backoff |
| `user-ns-sync-timeout` | child blocked on sync pipe; parent uid_map write timeout | yes, once |
| `export-socket-timeout` | socket did not appear within `readiness.timeout` | yes, with backoff |
| `guest-mount-probe-timeout` | guest-control health probe timed out | yes; volume phase → Unknown |
| `single-writer-violation` | second read-write attachment attempted while one is active | no |
| `shared-write-unsupported` | shared-write requested but Provider does not declare supportsSharedWrite | no |
| `view-not-found` | attachment references a View that does not exist in Volume spec | no |
| `execution-ref-not-found` | attachment executionRef does not resolve to a Guest in this Zone | no; fails closed |
| `vcpu-count-unavailable` | Guest spec not yet reconciled; threadPoolSize cannot be resolved | yes; requeue |
| `vfd-user-creation-failed` | User resource for vfd principal could not be created | yes, with backoff |
| `store-view-marker-absent` | `live/.d2b-marker-<guest>` absent; farm not yet populated | yes; requeue |
| `process-adoption-ambiguous` | virtiofsd process identity ambiguous on controller restart | no; quarantine |
| `socket-cleanup-failed` | stale socket unlink failed; previous virtiofsd may still be running | yes, once |
| `adr021-violation-detected` | `capabilityClasses` non-empty or `startRoot: true` detected at preflight | no; halt |

All error messages are bounded at 512 bytes, UTF-8/control-character validated, and contain
no host paths, socket paths, guest paths, process data, terminal bytes, raw errno details,
or credential material.

---

## 12. Status conditions

| Condition type | Normal value | Abnormal state |
| --- | --- | --- |
| `ProcessReady` | `"True"` / reason `process-running` | `"False"` when virtiofsd not yet started or has crashed |
| `ExportReady` | `"True"` / reason `socket-exists` | `"False"` if socket absent or cleanup in progress |
| `GuestMountReady` | `"True"` / reason `health-probe-ok` | `"False"` / `Unknown` on probe timeout or Guest off |
| `SingleWriterViolation` | absent | `"True"` if second read-write attachment attempted |
| `FinalizersBlocked` | `"False"` (not blocked) | `"True"` while virtiofsd Process deletion is pending |
| `AttachmentsReady` | `"True"` | `"False"` or `Unknown` while any attachment is not fully ready |

---

## 13. Audit records

All volume-virtiofs audit records use the Zone-local audit stream
(`d2b-audit` over the private local Unix datagram socket).

### 13.1 Attachment create

```json
{
  "subject_digest": "sha256:<hex>",
  "zone": "dev",
  "verb": "create-virtiofsd-process",
  "resourceRef": "Volume/work-state",
  "attachmentExecutionRefDigest": "sha256:<hex-of-Guest/work-vm>",
  "workerTemplate": "virtiofsd-worker",
  "accessMode": "read-write",
  "view": "controller",
  "correlationId": "<opaque>",
  "outcome": "process-created"
}
```

### 13.2 Attachment delete

```json
{
  "subject_digest": "sha256:<hex>",
  "zone": "dev",
  "verb": "delete-virtiofsd-process",
  "resourceRef": "Volume/work-state",
  "virtiofsdProcessRefDigest": "sha256:<hex>",
  "reason": "attachment-removed",
  "correlationId": "<opaque>",
  "outcome": "process-deletion-requested"
}
```

### 13.3 Attachment finalizer hold policy

If the Guest runner process absence can be positively proved (the runner process that owns the
Guest mount namespace is confirmed dead via pidfd, making the mount namespace observably gone),
the controller clears the attachment finalizer with that proof recorded in the audit record
(verb: `finalizer-cleared-with-proof`).

If the Guest is unreachable or the absence is ambiguous, the attachment remains in
`Degraded/Unknown` phase with the finalizer held. There is no time-based force-clear; the
finalizer is held until either the proof arrives or a full Zone reset is performed by the
operator. The audit record in the ambiguous case carries `outcome: finalizer-held` and
`reason: guest-unreachable-ambiguous`.

Excluded from all audit records: socket paths, host paths, raw PIDs, PID FDs, cgroup paths,
mount paths inside guests, virtiofsd binary path, process argv, environment variables,
guest credential material, and layout entry content.

---

## 14. Telemetry

### 14.1 Lightweight bounded emitter

volume-virtiofs uses the Zone-local lightweight bounded emitter (`tracing` +
bounded in-process ring → private Unix datagram socket). It does not import
`opentelemetry_sdk` or `opentelemetry-otlp`. Emitted frames are consumed by
`Provider/observability-otel` if installed.

### 14.2 Metric labels

All metric labels are from the closed set below. No free-form values,
no host paths, no socket paths, no guest names beyond a stable opaque digest.

| Label | Values |
| --- | --- |
| `provider` | `volume-virtiofs` (literal constant) |
| `operation` | `attach` \| `detach` \| `spawn-virtiofsd` \| `readiness-probe` \| `finalizer-drain` |
| `outcome` | `success` \| `error` \| `timeout` \| `conflict` \| `unknown` |
| `access_mode` | `read-only` \| `read-write` \| `shared-write` |
| `error_class` | stable error code from §11 Error catalog |

The `zone` and `execution` resource attributes are set at the OTEL resource level (from the
Process resource context) and are not repeated as metric labels.

VM name / Guest name is never a metric label. It may appear in OTEL trace context resource
attributes only, re-stamped at ingress boundary.

### 14.3 Key metrics

| Metric | Type | Description |
| --- | --- | --- |
| `d2b_volume_virtiofs_attachments_total` | Counter | Total attachment create attempts, labeled by outcome |
| `d2b_volume_virtiofs_detachments_total` | Counter | Total attachment delete attempts, labeled by outcome |
| `d2b_volume_virtiofs_ready_attachments` | Gauge | Current count of attachments with both exportReady and guestMountReady true |
| `d2b_volume_virtiofs_export_ready_seconds` | Histogram | Time from virtiofsd spawn to export socket appearing |
| `d2b_volume_virtiofs_mount_ready_seconds` | Histogram | Time from export socket ready to guest mount confirmed |
| `d2b_volume_virtiofs_process_restarts_total` | Counter | virtiofsd Process restart events, labeled by error_class |
| `d2b_volume_virtiofs_finalizer_drain_seconds` | Histogram | Time from deletion request to finalizer cleared |

### 14.4 Performance budgets

| Gate | Requirement |
| --- | --- |
| Export socket appears after virtiofsd spawn | p95 ≤ 500 ms for a warmed NixOS host |
| Guest mount confirmed (probe round trip) | p95 ≤ 2 s for a running Guest |
| Attachment status written after virtiofsd Ready | p95 ≤ 5 ms (matches core commit-to-handler budget) |
| Controller reconcile loop iteration (one attachment) | p95 ≤ 10 ms excluding spawn and probe I/O |

---

## 15. Nix configuration

### 15.1 Artifact catalog entry

```nix
d2b.artifacts."volume-virtiofs-provider" = {
  package = pkgs.d2b-provider-volume-virtiofs;
  type    = "provider";
};
```

### 15.2 Provider resource

```nix
d2b.zones."dev".resources."volume-virtiofs" = {
  type = "Provider";
  spec = {
    artifactId = "volume-virtiofs-provider";
    config = {};
  };
};
```

### 15.3 Volume with virtiofs attachment (minimal state Volume)

```nix
d2b.zones."dev".resources."work-state" = {
  type = "Volume";
  spec = {
    providerRef = "Provider/volume-local";
    source = {
      executionRef = "Host/host-system";
      settings.kind = "local-path";
    };
    kind = "state";
    layout = [
      {
        path = "";
        type = "directory";
        ownerRef = "User/d2b-work-vm-runner";
        groupRef = "User/d2b-work-vm-runner";
        mode = "0700";
        sensitivity = "private";
        createPolicy = "create-if-never-provisioned";
        repairPolicy = "fail-closed";
        cleanupPolicy = "never";
      }
    ];
    views.controller = {
      path = "";
      rights = [ "read" "write" "create" "delete" "traverse" ];
    };
    attachments = [
      {
        executionRef = "Guest/work-vm";
        transport = "virtiofs";
        view = "controller";
        access = "read-write";
        mountPath = "/state";
      }
    ];
  };
};
```

### 15.4 Canonical rendered ResourceSpec JSON (attachment defaults materialized)

```json
{
  "apiVersion": "resources.d2b.io/v3",
  "type": "Volume",
  "metadata": {
    "name": "work-state",
    "zone": "dev",
    "ownerRef": null,
    "finalizers": []
  },
  "spec": {
    "providerRef": "Provider/volume-local",
    "source": {
      "executionRef": "Host/host-system",
      "settings": { "kind": "local-path" }
    },
    "kind": "state",
    "layout": [
      {
        "path": "",
        "type": "directory",
        "ownerRef": "User/d2b-work-vm-runner",
        "groupRef": "User/d2b-work-vm-runner",
        "mode": "0700",
        "noFollow": true,
        "recursive": false,
        "sensitivity": "private",
        "createPolicy": "create-if-never-provisioned",
        "repairPolicy": "fail-closed",
        "cleanupPolicy": "never",
        "adoptionPolicy": "adopt-with-live-owner-proof",
        "restartPolicy": "preserve-across-controller-restart",
        "leaseClass": "none",
        "foreignChildPolicy": "preserve",
        "accessAcl": [],
        "defaultAcl": [],
        "invariants": ["no-symlink"]
      }
    ],
    "views": {
      "controller": {
        "path": "",
        "rights": ["create", "delete", "read", "traverse", "write"]
      }
    },
    "attachments": [
      {
        "executionRef": "Guest/work-vm",
        "transport": "virtiofs",
        "view": "controller",
        "access": "read-write",
        "mountPath": "/state",
        "settings": {
          "cache": "auto",
          "inodeFileHandles": "never",
          "posixAcl": false,
          "socketGroup": null,
          "threadPoolSize": null,
          "xattr": false
        }
      }
    ],
    "quota": null
  }
}
```

Rights are sorted lexicographically. All keys are sorted. Defaults are always materialized.
`hostPath` is injected by the resource compiler after Nix eval and is never in this JSON.

### 15.5 Eval/build validation

The following validations are fatal at Nix eval time for virtiofs attachments:

1. `transport = "virtiofs"` requires `Provider/volume-virtiofs` to be installed in the
   same Zone. Missing Provider aborts with a structured error naming the Volume and the
   missing Provider.
2. `view` must exist in the Volume's `views` map. Unknown view name aborts.
3. `access` must be compatible with the named View's declared `rights`. `read-write` on a
   View with only `[read, traverse]` aborts.
4. `shared-write` aborts unconditionally in v3.0 (Provider does not declare
   `supportsSharedWrite: true`).
5. `settings` is validated against the Provider's signed `attachment.schema.json` from the
   private artifact catalog entry. Unknown fields abort; out-of-range values abort.
6. `executionRef` must resolve to a `Guest/<name>` resource in the same Zone.
7. At most one `read-write` attachment per Volume at eval time. The Nix resource compiler
   rejects two simultaneous `read-write` entries at build time.
8. Credential refs: no secret values may appear in attachment settings.

### 15.6 Attachment schema JSON (volume-virtiofs signed schema)

The signed `attachment.schema.json` is part of the Provider package. Nix reads it from the
private artifact catalog entry for `volume-virtiofs-provider`. Its canonical form:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "VirtiofsAttachmentSettings",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "posixAcl":           { "type": "boolean", "default": false },
    "xattr":              { "type": "boolean", "default": false },
    "cache":              { "type": "string", "enum": ["auto", "always", "never"], "default": "auto" },
    "inodeFileHandles":   { "type": "string", "enum": ["never", "prefer", "mandatory"], "default": "never" },
    "threadPoolSize":     { "type": ["integer", "null"], "minimum": 1, "maximum": 256, "default": null },
    "socketGroup":        { "type": ["integer", "null"], "default": null }
  }
}
```

### 15.7 Store-view Volume (resource compiler output, generated per Guest)

```nix
# Auto-generated by the Nix resource compiler for each Guest with a VM runtime Provider.
# Operators do not write this resource directly.
d2b.zones."dev".resources."store-view-work-vm" = {
  type = "Volume";
  spec = {
    providerRef = "Provider/volume-local";
    source = {
      executionRef = "Host/host-system";
      settings.kind = "local-path";
      # hostPath = "<storeStateDir>/work-vm/store-view" — injected by compiler
    };
    kind = "durable";
    layout = [
      { path = "";              type = "directory"; ownerRef = "User/d2bd"; groupRef = "User/users"; mode = "0755"; }
      { path = "live";          type = "directory"; invariants = ["no-symlink" "broker-opaque-id-only"]; cleanupPolicy = "cutover-only"; repairPolicy = "exact-owner"; ownerRef = "User/d2bd"; groupRef = "User/users"; mode = "0755"; }
      { path = "live/.d2b-marker-work-vm"; type = "file"; ownerRef = "User/d2bd"; groupRef = "User/users"; mode = "0444"; invariants = ["no-symlink" "same-filesystem" "hardlink-farm-no-recursion" "broker-opaque-id-only"]; repairPolicy = "exact-owner"; }
      { path = "meta";          type = "directory"; invariants = ["no-symlink" "same-filesystem" "hardlink-farm-no-recursion" "broker-opaque-id-only"]; repairPolicy = "exact-owner"; ownerRef = "User/d2bd"; groupRef = "User/users"; mode = "0755"; }
      { path = "meta/generations"; type = "directory"; invariants = ["no-symlink" "same-filesystem" "hardlink-farm-no-recursion" "broker-opaque-id-only"]; cleanupPolicy = "cutover-only"; repairPolicy = "exact-owner"; ownerRef = "User/d2bd"; groupRef = "User/users"; mode = "0755"; }
      { path = "meta/current";  type = "symlink"; target = "generations/0"; noFollow = false; invariants = ["broker-opaque-id-only"]; cleanupPolicy = "cutover-only"; repairPolicy = "exact-owner"; ownerRef = "User/d2bd"; groupRef = "User/users"; mode = "0777"; }
      { path = "state";         type = "directory"; invariants = ["no-symlink" "broker-opaque-id-only"]; repairPolicy = "exact-owner"; ownerRef = "User/d2bd"; groupRef = "User/users"; mode = "0700"; }
      { path = "gcroots";       type = "directory"; invariants = ["no-symlink" "same-filesystem" "hardlink-farm-no-recursion" "broker-opaque-id-only"]; cleanupPolicy = "cutover-only"; repairPolicy = "exact-owner"; ownerRef = "User/d2bd"; groupRef = "User/users"; mode = "0755"; }
      { path = "sync.lock";     type = "file"; ownerRef = "User/d2bd"; groupRef = "User/users"; mode = "0640"; leaseClass = "none"; invariants = ["no-symlink" "broker-opaque-id-only"]; restartPolicy = "preserve-across-controller-restart"; }
    ];
    views = {
      ro-store = { path = "live"; rights = [ "read" "traverse" ]; };
      meta     = { path = "meta"; rights = [ "read" "traverse" ]; };
    };
    attachments = [
      {
        executionRef = "Guest/work-vm";
        transport    = "virtiofs";
        view         = "ro-store";
        access       = "read-only";
        mountPath    = "/nix/.ro-store";
        settings = { posixAcl = false; xattr = false; cache = "auto"; inodeFileHandles = "never"; };
      }
    ];
  };
};
```

---

## 16. Cleanup contract

### 16.1 Volume deletion

When a Volume with virtiofs attachments is deleted:

1. volume-virtiofs controller observes `deletionRequestedAt` set on the Volume.
2. Controller emits Delete for each owned virtiofsd Process resource.
3. system-minijail sends SIGTERM to each virtiofsd process; waits via pidfd.
4. On process exit, controller sets attachment `state: detaching`, `exportReady: false`.
5. Controller queries guest-control health probe; waits for `MountAbsent`.
6. When all attachments confirm `MountAbsent`, controller clears
   `volume-virtiofs/attachments` finalizer.
7. volume-local finalizer proceeds (child finalizers first).
8. After all finalizers are cleared, volume-local emits a Deleted revision event for the
   Volume; the row and indexes are removed atomically from the store.

Controller-created User resources (`User/vol-<vol>-vfd`) are deleted after the last
virtiofsd Process resource referencing them is deleted, using the normal owner-child
finalizer cascade (ownerRef: Volume, not Process).

### 16.2 Attachment removal (Volume not deleted)

When a specific attachment entry is removed from the Volume spec while the Volume itself
remains:

1. volume-virtiofs detects attachment list change via `spec-generation-changed` hint.
2. Controller deletes only the virtiofsd Process owned by that attachment.
3. After Process deletion and guest mount absent confirmation, controller writes updated
   attachment status (entry removed from `attachmentStatuses`).
4. Controller does not touch attachments for other Guests on the same Volume.

### 16.3 Configuration-removed condition

When a Volume is removed from the Nix configuration generation:

```yaml
status:
  phase: Degraded
  conditions:
    - type: ConfigurationRemoved
      status: "True"
      reason: absent-from-configuration
    - type: FinalizersBlocked
      status: "True"
      reason: finalizers-draining
  attachmentStatuses:
    - executionRef: Guest/work-vm
      state: detaching
      exportReady: false
      guestMountReady: false
```

### 16.4 Prior-generation retention

The Zone retains the last `priorGenerationCount` generations (default 3, range 1–16).
A Volume that has been deleted but whose generation is within the retention window may be
reactivated via `ActivateGeneration`. Reactivation cancels in-flight Delete for the Volume
and its owned Processes; the controller reconciles from the retained spec.

---

## 17. Current-code fit

| Item | Evidence class | Treatment |
| --- | --- | --- |
| `packages/d2b-host/src/virtiofsd_argv.rs`: `VirtiofsdArgvInput`, `generate_virtiofsd_argv`, 14 unit tests, golden `argv.txt` | `implemented-and-reachable` | Extract to `d2b-provider-volume-virtiofs/src/virtiofsd_argv.rs`; migrate 14 tests to `tests/argv_golden.rs`; remove `extra_args` field |
| `nixos-modules/minijail-profiles.nix`: `virtiofsdProfiles`; principals `d2b-<vm>-runner`, `d2b-<vm>-gctlfs`; ADR 0021 user-NS exception | `generated-or-eval-contract` | Becomes `virtiofsd-worker` Process sandbox spec; ADR 0021 invariants fully preserved; principals → typed `User/<name>` ResourceRefs; no numeric form |
| `nixos-modules/processes-json.nix`: `virtiofsdRunner` shape; `roStoreSharedDir` redirect sentinel `share.source == "/nix/store"` → `store-view/live` | `generated-or-eval-contract` | Replaced by volume-virtiofs controller-owned Process resource; `store-view/live` redirect preserved in resource compiler |
| `packages/d2b-core/src/processes.rs`: `ProcessRole::Virtiofsd`, `VmProcessDag` virtiofsd entry | `generated-or-eval-contract` | Replaced by Process resource template `virtiofsd-worker` owned by volume-virtiofs |
| `packages/d2b-priv-broker/src/ops/spawn_runner.rs`: `SpawnRunnerPlanInput`, `RunnerIsolationSpec`, `adr_carve_out` virtiofsd path | `implemented-and-reachable` | `SpawnRunnerPlanInput` → v3 `LaunchTicket` with typed sandbox spec; `adr_carve_out` field removed; ADR 0021 is no longer a carve-out but the normal path |
| `packages/d2b-priv-broker/src/sys.rs`: `clone3_spawn_runner` user-NS pre-establishment | `implemented-and-reachable` | Extract and adapt to `d2b-provider-volume-virtiofs/src/user_ns.rs`; exact sequence preserved per ADR 0021 implementation contract |
| `packages/d2bd/src/supervisor/dag.rs`: `ProcessRole::Virtiofsd` dag node supervised as entry under `WorkloadId`-keyed `VmProcessDag` | `implemented-and-reachable` | Replaced by Process controller lifecycle in v3; dag node retired after controller parity |
| `packages/d2b-contract-tests/tests/storage_sync_contracts.rs`: virtiofsd argv shape gate | `implemented-and-reachable` | Adapted to Process sandbox spec gate in `d2b-provider-volume-virtiofs/tests/schema_conformance.rs` |
| `tests/tools/gen-migration-ledger.sh` → `virtiofsd-argv-shape` gate | `implemented-and-reachable` | Adapted to validate Process template argv golden vector |
| `tests/tools/gen-migration-ledger.sh` → `minijail-validator-virtiofsd` gate | `implemented-and-reachable` | Adapted to enforce Process sandbox spec ADR 0021 invariants |
| `tests/unit/nix/cases/broker-caps.nix` | `implemented-and-reachable` | Adapted to v3 Process template capability policy gate |
| `packages/d2b-host/src/virtiofsd_argv.rs` (baseline): socket path format `/run/d2b/vms/<vm>/<vm>-virtiofs-<tag>.sock` | `implemented-and-reachable` | Replaced by private hash-derived path in `socket_path.rs`; exact current format is not preserved (v3 path is stable but different); new path is equally private |
| ADR 0021 (`docs/adr/0021-broker-user-namespace-for-virtiofsd.md`) | `implemented-and-reachable` | Full invariant; not a carve-out; see §5.3 |

**Main reuse**: `packages/d2b-session/` and `packages/d2b-session-unix/` from main commit
`a1cc0b2d` are the selected ComponentSession sources per `ADR-046-componentsession-and-bus`.
volume-virtiofs uses the toolkit ResourceClient, which wraps ComponentSession and d2b-bus;
it does not import session implementation internals directly.

---

## 18. Implementation work items

### ADR046-vvfs-001 — crate bootstrap and argv extraction

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-volume-001 (Volume contract types must exist); W1; volume-virtiofs Provider owner |
| Current source | `packages/d2b-host/src/virtiofsd_argv.rs` (VirtiofsdArgvInput, generate_virtiofsd_argv, 14 unit tests, golden argv.txt); `packages/d2b-host/src/lib.rs` (module declaration) |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-provider-volume-virtiofs/src/virtiofsd_argv.rs`; `packages/d2b-provider-volume-virtiofs/tests/argv_golden.rs` |
| Detailed design | Create crate skeleton with mandatory `src/`, `tests/`, `integration/`, `README.md`. Extract `VirtiofsdArgvInput` and `generate_virtiofsd_argv` with these changes: (1) replace `extra_args: Vec<String>` field with nothing (removed; Provider root config is empty); (2) replace `socket_path: String` with `socket_path: SocketPath` newtype backed by `socket_path.rs`; (3) add `shared_dir_fd: i32` replacing `shared_dir: String` (FD-based to avoid path leaks); (4) replace `socket_group: Option<u32>` with `socket_group: Option<Gid>` newtype. Implement `socket_path.rs`: private path derivation using SHA-256 of canonical `<zone>\x00<volume>\x00<guest>`, truncated to 8 hex chars, formatted as `<zone-runtime-dir>/vms/<guest>/vol-<hash>.vfd.sock`. Assert path length ≤ 108 bytes in tests. |
| Integration | volume-virtiofs controller `attachment.rs` calls virtiofsd_argv.rs at spawn time; LaunchTicket carries the resolved socket path as an opaque FD ref |
| Data migration | v3.0 reset; socket paths change format (private hash-derived vs current `<vm>-virtiofs-<tag>.sock`) |
| Validation | `tests/argv_golden.rs`: 14 migrated tests + `no_extra_args_ever_emitted`, `socket_path_is_not_in_args`, `shared_dir_is_fd_path`, `path_length_within_sunpath_limit`; `tests/socket_path_privacy.rs`: `socket_path_not_in_process_status`, `socket_path_not_in_volume_status`, `socket_path_not_in_audit_record`; `tests/schema_conformance.rs`: `process_spec_readiness_class_is_provider_defined`, `process_spec_readiness_has_no_kind_or_period_fields`, `process_spec_budget_cpu_request_limit_nested`, `process_spec_budget_memory_request_limit_nested`, `process_spec_budget_pids_limit_present`, `process_spec_budget_fds_limit_present`, `process_spec_sandbox_no_new_privileges_true`, `process_spec_sandbox_read_only_root_true` |
| Removal proof | `packages/d2b-host/src/virtiofsd_argv.rs` removed only after volume-virtiofs parity is confirmed by argv-shape gate adaption |

### ADR046-vvfs-002 — ADR 0021 user-namespace pre-establishment

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-vvfs-001; ADR046-volume-001; W1; broker/spawn owner |
| Current source | `packages/d2b-priv-broker/src/sys.rs` (`clone3_spawn_runner`, user-NS pre-establishment block); `packages/d2b-priv-broker/src/ops/spawn_runner.rs` (`SpawnRunnerPlanInput.user_namespace`, `RunnerIsolationSpec.user_namespace`); ADR 0021 implementation contract |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-provider-volume-virtiofs/src/user_ns.rs`; `packages/d2b-provider-volume-virtiofs/tests/adr021_invariant.rs` |
| Detailed design | Extract the user-NS pre-establishment spec generation: `UserNsSpec { host_uid_for_zero: u32, host_gid_for_zero: u32 }` built from the resolved `User/vol-<vol>-vfd` stable UID/GID. The v3 LaunchTicket carries this spec; the broker `clone3_spawn_runner` path is unchanged. Add an adr021 conformance check in the virtiofsd-worker Process template descriptor: assert `capabilityClasses: []`, `startRoot: false`, `noNewPrivileges: true`, `readOnlyRoot: true`, `sandbox.userNamespace.singleEntry: true`. Any template mutation that introduces host capability classes, sets `startRoot: true`, disables `noNewPrivileges`, or disables `readOnlyRoot` must be rejected by the conformance check before the LaunchTicket is issued. |
| Integration | volume-virtiofs controller builds `UserNsSpec` from resolved User UID/GID and populates the Process spec sandbox before emitting the Process resource; ProviderSupervisor passes spec to broker spawn path |
| Data migration | v3.0 reset; current `adr_carve_out` field in `SpawnRunnerPlanInput` removed; ADR 0021 path is now the default, not a carve-out |
| Validation | `tests/adr021_invariant.rs`: `virtiofsd_capability_classes_must_be_empty`, `virtiofsd_start_root_must_be_false`, `sandbox_namespace_never_emitted`, `user_ns_single_entry_single_uid_mapping`, `uid_map_write_ordering_uid_setgroups_gid`, `child_setuid_in_ns_not_host_uid`, `clone_newns_not_in_clone3_flags`, `child_exits_user_ns_sync_on_pipe_eof`; tests adapted from current `packages/d2b-priv-broker/src/ops/spawn_runner.rs` tests (`user_namespace_round_trips_*`, `user_namespace_with_zero_uid_*`, `user_namespace_true_requires_spec`, `user_namespace_spec_requires_namespace_flag`) |
| Removal proof | `adr_carve_out` field and virtiofsd-specific branch in current `SpawnRunnerPlanInput` removed only after v3 LaunchTicket covers all virtiofsd spawn cases |

### ADR046-vvfs-003 — attachment lifecycle controller

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-vvfs-001, ADR046-vvfs-002; ADR046-volume-001; W2; volume-virtiofs controller owner |
| Current source | `packages/d2bd/src/supervisor/dag.rs` (ProcessRole::Virtiofsd dag node: current spawn/adopt/stop loop under WorkloadId-keyed VmProcessDag); `nixos-modules/processes-json.nix` (virtiofsdRunner block; attachment-to-Process mapping) |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-provider-volume-virtiofs/src/controller.rs`; `packages/d2b-provider-volume-virtiofs/src/attachment.rs` |
| Detailed design | Implement volume-virtiofs-controller reconcile loop using toolkit ResourceClient. Watch selector: Volume resources with at least one `transport: virtiofs` attachment, owned Process resources, owned User resources. On `spec-generation-changed` for a Volume: (1) compute desired virtiofsd Process set from attachments list; (2) diff against current Process resources; (3) emit Create for new, UpdateSpec for changed, Delete for removed. On `owned-resource-changed` for a Process: update `attachmentStatuses` for the affected attachment. Single-writer enforcement: before emitting Process Create, check current attachment status for any active `read-write` entry; reject with `ResourceConflict` if found. Per-attachment state machine: `Attaching → Attached | Failed`, `Attached → Detaching → Detached`. |
| Integration | volume-virtiofs controller registered under `Host/host-system` as a Process; receives owned-resource-changed trigger from Volume; emits Process resources consumed by system-minijail |
| Data migration | Current `ProcessRole::Virtiofsd` dag nodes keyed by `WorkloadId` replaced by Process resources; current dag node supervision loop in `d2bd/src/supervisor/dag.rs` retired |
| Validation | `tests/attachment_lifecycle.rs`: `attachment_create_spawns_virtiofsd_process`, `attachment_ready_when_export_socket_present`, `attachment_delete_terminates_virtiofsd`, `attachment_delete_waits_for_guest_mount_absent`, `attachment_delete_with_guest_unreachable_holds_finalizer_degraded`; `tests/single_writer.rs`: `second_read_write_rejected`, `read_only_plus_read_write_allowed`, `read_write_delete_then_new_read_write_allowed`; `tests/multi_attachment.rs`: `two_guests_get_separate_processes`, `process_failure_does_not_affect_sibling_attachment` |
| Removal proof | `ProcessRole::Virtiofsd` branch in `d2bd/src/supervisor/dag.rs` removed only after v3 controller passes all lifecycle tests and VmProcessDag parity gate passes |

### ADR046-vvfs-004 — readiness and guest-mount probe

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-vvfs-003; guest-control integration owner; W2 |
| Current source | `packages/d2bd/src/vm_readiness.rs` (`ReadinessKind::UnixSocketExists`); `packages/d2b-core/src/processes.rs` (readiness definitions); guest-control vsock health protocol |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-provider-volume-virtiofs/src/readiness.rs`; `packages/d2b-provider-volume-virtiofs/integration/guest_mount_readiness/` |
| Detailed design | `unix-socket-exists` readiness: check file existence at the private socket path via a bounded blocking adapter (e.g., `tokio::task::spawn_blocking` wrapping `fstatat(2)` relative to the zone runtime `OwnedFd`, or an async-safe fd-relative equivalent); no blocking syscall on the async executor thread. Probe period 1 s; timeout 30 s. On socket present → set `exportReady: true`. Guest-mount readiness: send `VirtioFsMountReady?` probe to guest-control health endpoint over vsock. Response `MountReady` sets `guestMountReady: true`. Response `MountAbsent` or timeout sets `guestMountReady: false`. The vsock health probe is async-native (non-blocking). If Guest is down, set `state: unknown`. All readiness probes (unix-socket-exists, guest-mount health) use bounded blocking adapters or async-safe fd-relative equivalents; no blocking I/O on the reconcile executor thread. |
| Integration | `readiness.rs` called from `controller.rs` reconcile loop; uses toolkit health probe client |
| Data migration | Current `UnixSocketExists` readiness kind adapted to FD-based path resolution |
| Validation | `tests/attachment_lifecycle.rs` (extended); `integration/guest_mount_readiness/`: virtiofsd launches, socket appears, guest-control probe returns MountReady, guestMountReady flips to true; probe returns MountAbsent on umount |
| Removal proof | Current `UnixSocketExists` readiness path in `d2bd` retired after volume-virtiofs readiness covers all cases |

### ADR046-vvfs-005 — store-view attachment and marker prerequisite

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-vvfs-003, ADR046-vvfs-004; ADR046-volume-002 (store-view Volume); W3 |
| Current source | `packages/d2b-host/src/hardlink_farm.rs` (`live_dir()`, marker `live/.d2b-marker-<vm>`, zero-length); `nixos-modules/processes-json.nix` (`roStoreSharedDir` sentinel `share.source == "/nix/store"` → `store-view/live`); `nixos-modules/store.nix` (per-VM hardlink farm) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-volume-virtiofs/src/controller.rs` (pre-launch prerequisite check); `packages/d2b-provider-volume-virtiofs/integration/store_view_readonly/` |
| Detailed design | Before issuing LaunchTicket for a store-view virtiofsd Process, check that `live/.d2b-marker-<guest>` exists (zero-length, correct mode) via a bounded blocking adapter (e.g., `tokio::task::spawn_blocking` wrapping `fstatat(2)` relative to the Volume root `OwnedFd`, or an async-safe fd-relative equivalent); no blocking syscall on the async executor thread directly. If absent, requeue with exponential backoff. This prevents virtiofsd from starting before the hardlink farm is populated, which would serve an empty or partial store to the guest initrd. Assert `--shared-dir` resolves to `store-view/live` (the `ro-store` View path), never to `/nix/store`. Validate in `integration/store_view_readonly/` that virtiofsd serves only paths under `store-view/live`. |
| Integration | Pre-launch check in controller.rs; store-view Volume attachment recognized by `view == "ro-store"` and `access == "read-only"` |
| Data migration | Current `roStoreSharedDir` redirect in `processes-json.nix` replaced by `ro-store` View definition in the store-view Volume resource |
| Validation | `integration/store_view_readonly/`: mount from guest reads closure paths; no host-store path escapes; `tests/argv_golden.rs`: `store_view_shared_dir_is_live_not_nix_store`; `tests/attachment_lifecycle.rs`: `store_view_launch_waits_for_marker` |
| Removal proof | `nixos-modules/processes-json.nix` `virtiofsdRunner` block and `roStoreSharedDir` sentinel removed only after store-view virtiofsd Process resources pass parity gate |

### ADR046-vvfs-006 — Nix resource compiler integration and cleanup

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-vvfs-001, ADR046-volume-004; Nix integrator; W3 |
| Current source | `nixos-modules/processes-json.nix` (virtiofsdRunner block); `nixos-modules/minijail-profiles.nix` (virtiofsdProfiles); `nixos-modules/options-vms.nix` (`d2b.vms.<vm>.shares.*`) |
| Reuse action | adapt |
| Destination | `nixos-modules/resources-volume.nix` (store-view and user Volume attachment emission); `nixos-modules/options-volumes.nix` (optional user-facing volume/attachment options) |
| Detailed design | Extend the Nix resource compiler to: (1) auto-emit a store-view Volume (with `ro-store` and `meta` Views, virtiofs ro-store attachment) per Guest that has a VM runtime Provider; (2) emit virtiofs attachment entries for explicitly configured user Volumes; (3) emit `User/vol-<vol>-vfd` resources for each Volume with virtiofs attachments; (4) emit `Provider/volume-virtiofs` as a Provider resource when any virtiofs attachment is configured. All 15 Nix eval validation steps from `ADR-046-resources-volume` §15.5 apply. |
| Integration | `nixos-modules/default.nix` wires resources-volume.nix; nix-unit tests verify canonical output |
| Data migration | `d2b.vms.<vm>.shares` virtiofs entries → Volume attachments; `d2b.vms.<vm>` store-view auto-emission replaces `nixos-modules/store.nix` virtiofsd portion |
| Validation | nix-unit: `store_view_volume_auto_emitted_per_guest`, `volume_virtiofs_attachment_canonical_json`, `virtiofs_provider_emitted_when_attachment_configured`, `vfd_user_emitted_per_volume`, `second_read_write_attachment_rejected_at_eval`, `transport_virtiofs_requires_provider_installed`; drift-check gate for `nixos-modules/processes-json.nix` virtiofsdRunner removal |
| Removal proof | `nixos-modules/processes-json.nix` virtiofsdRunner block, `nixos-modules/minijail-profiles.nix` virtiofsdProfiles removed only after Nix resource compiler produces parity output and all nix-unit cases pass |

---

## 19. Integration test layout

The crate must contain the four mandatory top-level entries required by the workspace policy
gate: `src/`, `tests/`, `integration/`, and `README.md`. The `integration/` directory must
contain at least the four fixture subdirectories listed below, each with a `README.md` and at
least one Rust or shell test driver; empty directories fail the gate.

The `integration/README.md` at `packages/d2b-provider-volume-virtiofs/integration/README.md`
must document each fixture's purpose, run instructions, and prerequisites. No nested
`src/tests/integration/README.md` is required or created.

The four required fixture subdirectories and their coverage obligations:

| Subdirectory | Coverage |
| --- | --- |
| `virtiofsd_launch/` | Spawns a real virtiofsd process (from `pkgs/virtiofsd`) against a local tmpfs Volume. Asserts: process starts; export socket appears within 5 s; process exits cleanly on SIGTERM. Requirements: virtiofsd binary in PATH; `/dev/fuse` accessible. |
| `guest_mount_readiness/` | Uses a container/Host fixture with a running guest-control stub. Asserts: guest-control probe returns `MountReady` after virtiofsd starts; probe returns `MountAbsent` after socket removed. Requirements: podman; network access disabled. |
| `finalizer_drain/` | Simulates Guest restart during Volume deletion. Asserts: volume-virtiofs finalizer is not cleared while Guest is unreachable and no pidfd proof is available; finalizer is cleared after Guest comes back and confirms `MountAbsent`; finalizer is cleared immediately when pidfd proof of mount-namespace death is present. Requirements: podman; guest-control stub container. |
| `store_view_readonly/` | Mounts a real store-view Volume (tmpfs-backed for CI) via virtiofsd. Asserts: `--shared-dir` resolves to `live/` not `/nix/store`; marker prerequisite gates launch; read-only flag set; no host-store paths accessible. Requirements: virtiofsd binary in PATH; `/dev/fuse` accessible; fake hardlink-farm marker fixture. |

---

## 20. Removal proofs

| Current artifact | Removed after | Successor |
| --- | --- | --- |
| `packages/d2b-host/src/virtiofsd_argv.rs` | ADR046-vvfs-001 parity confirmed; argv-shape gate adapted | `packages/d2b-provider-volume-virtiofs/src/virtiofsd_argv.rs` |
| `nixos-modules/minijail-profiles.nix` virtiofsdProfiles block | ADR046-vvfs-006; Process template sandbox spec passes broker-caps gate | `packages/d2b-provider-volume-virtiofs/src/` Process template descriptor |
| `nixos-modules/processes-json.nix` virtiofsdRunner block and `roStoreSharedDir` sentinel | ADR046-vvfs-005, ADR046-vvfs-006; VmProcessDag parity gate passes | volume-virtiofs controller-owned Process resources |
| `packages/d2bd/src/supervisor/dag.rs` `ProcessRole::Virtiofsd` branch | ADR046-vvfs-003; Process controller lifecycle covers all virtiofsd spawn/adopt/stop paths | volume-virtiofs attachment lifecycle controller |
| `packages/d2b-priv-broker/src/ops/spawn_runner.rs` `adr_carve_out` virtiofsd field | ADR046-vvfs-002; v3 LaunchTicket handles all virtiofsd spawn cases without carve-out | Process spec `sandbox.userNamespace` field |
| `packages/d2b-core/src/processes.rs` `ProcessRole::Virtiofsd` enum variant | All volume-virtiofs work items complete; no remaining consumer | Process resource template `virtiofsd-worker` |

No current path is removed until its resource/controller/Provider successor is integrated,
tested, and confirmed by parity gates. Removal is recorded in the CHANGELOG under the
relevant release section with `managedBy: configuration` confirmation.
