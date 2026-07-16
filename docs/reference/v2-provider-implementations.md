# d2b 2.0 provider implementation crates

First-party provider implementations use the base-first
`d2b-provider-<type>-<implementation>` naming convention. Each crate below
owns one authority boundary and consumes the canonical `d2b-provider` traits
and `d2b-contracts` provider DTOs.

The workspace boundaries do not imply production availability. A provider is
available only after its implementation and conformance coverage are complete
and the production registry explicitly registers its live capabilities.

| Crate | Ownership and scope |
| --- | --- |
| `d2b-provider-audio-pipewire-vhost-user` | PipeWire routing and vhost-user-sound lifecycle, status, mute, volume, and policy. |
| `d2b-provider-credential-entra` | Entra credential acquisition inside the owning workload or provider agent; only opaque leases may cross the boundary. |
| `d2b-provider-credential-managed-identity` | Managed identity acquisition inside its exact cloud consumer boundary; credential material remains local. |
| `d2b-provider-credential-secret-service` | Per-user Secret Service interaction through the user daemon and opaque lease issuance. |
| `d2b-provider-device-host-mediated` | Host-mediated TPM, USBIP, CTAPHID/UHID security key, GPU, video, and mediated-device capabilities. |
| `d2b-provider-display-wayland` | Wayland, Waypipe/cross-domain, proxy authorization, readiness, and lifecycle. |
| `d2b-provider-infrastructure-azure-vm` | Scaffold-only Azure VM create, power, adopt, bootstrap, inspect, and delete authority. It does not own workload deployment or execution. |
| `d2b-provider-network-local-realm` | Realm-scoped bridge, TAP, net VM, NAT, DHCP, nftables, netlink, external attachment, and isolation behavior. |
| `d2b-provider-observability-local` | Bounded local metrics, tracing, audit export, status, and projections without audit or repair-state ownership. |
| `d2b-provider-runtime-azure-container-apps` | Azure Container Apps workload lifecycle and inspection. |
| `d2b-provider-runtime-azure-vm` | Scaffold-only workload deployment, execution, local start/stop, and inspection over an opaque infrastructure handle. It does not own VM resources. |
| `d2b-provider-runtime-local` | Local Cloud Hypervisor, qemu-media, and systemd-user workload lifecycle, graceful stop, and adoption. |
| `d2b-provider-storage-local` | Local state, disk image, Nix store-view, closure synchronization, media, and persistent-state operations. |
| `d2b-provider-substrate-host` | NixOS and Linux host checks, plans, and authorized substrate apply. |
| `d2b-provider-transport-azure-relay` | Azure Relay connection behavior; transport authentication never grants d2b authorization. |
| `d2b-provider-transport-local` | Unix stream, Unix seqpacket, native vsock, and Cloud Hypervisor vsock transports. |
| `d2b-azure-vm-fake-sdk` | In-process fake Azure VM SDK boundary shared by infrastructure/runtime conformance tests; it has no live Azure or network client. |

The Azure VM provider reservations remain unavailable to production
registries. A later accepted decision is required before either crate may make
live SDK calls or advertise Azure VM support.

## Production composition

`d2bd` owns one canonical provider-registry value for its full process
lifetime. After production services and `ServerState` exist, startup loads the
integrity-pinned private `provider-registry-v2.json` artifact, validates the
complete transaction, constructs instances with each implementation crate's
public factory, and initializes a one-shot registry cell. It then calls provider
health and the axis-specific read-only inspection or status operation through
that retained registry before serving requests.
Duplicate or missing bindings, descriptor/factory mismatches, stale
generations, scope or placement mismatches, unavailable intent mappings, and
non-dispatchable capabilities abort startup. The registry is not reconstructed
per call. An empty registry is valid only when the artifact explicitly contains
zero provider rows.

The generated artifact carries canonical realm and provider IDs in descriptor
placement, a target workload ID in each local-runtime binding, and opaque
references to existing bundle intents. Descriptor placement is the sole realm
authority: the binding deliberately cannot encode a contradictory realm.
Keeping `workloadId` is necessary because a realm-scoped trusted in-process
descriptor does not identify its target workload. The artifact contains no
argv, host path, credential, or secret payload. `bundleVersion` 12 adds the
artifact; its own schema remains version `v2`. Provider activation accepts
exactly bundle version 12 with bundle schema `v2`, a declared provider artifact
path, a bundle hash, and an artifact-hash entry for that path. Older bundle
versions remain readable by compatibility consumers but cannot activate this
registry.

The host `d2bd.service` restart trigger includes both the realised bundle and
the realised provider-registry artifact. A changed generation therefore
restarts the daemon before it can retain stale composition. This remains a
continuation event: `KillMode=process` preserves broker-spawned runners, and
the notify-ready replacement completes adoption before serving requests.

The live host registry currently registers local runtime and local
observability providers:

| Axis | Live implementations | Mapping and daemon authority |
| --- | --- | --- |
| Runtime | `cloud-hypervisor`, `qemu-media` | Explicit realm workloads map to matching VM-start and runner intent IDs. The daemon authenticates the mapping against the process DAG, observes its pidfd table, and calls the existing lifecycle start/stop authority through the provider adapter. |
| Observability | `local` | Each enabled host-local root realm receives a closed binding containing only query/export limits. The daemon projects bounded aggregate metrics and audit-sink health into closed records. The provider-owned sink streams exact JSON Lines or OTLP `ExportMetricsServiceRequest` protobuf within the configured record, byte, and time-window limits, then atomically persists a private `0600` artifact under daemon-owned state keyed only by the opaque operation ID. Internal inspection resolves that ID without exposing a host path through provider DTOs or diagnostics. |

Only explicit realm workload rows with a matching generated VM process DAG are
eligible. Realm and workload IDs must derive exactly from the DAG's
`workloadIdentity`; each VM and intent pair has one owner; and the runner must
carry explicit executable and argv data rather than the legacy synthesis
fallback. A first-class workload without existing VM-start and runner intents is
not emitted as live. Azure VM IDs and `RuntimeExecute` are rejected.

Unavailable Azure VM scaffold crates remain outside every shipped production
graph. The workspace policy derives roots from Cargo binary-target metadata,
adds the gateway library boundary, and reconciles them with the exact Rust
package outputs declared by the flake. A pinned output-to-package map makes a
new or renamed flake package fail policy until its dependency root is reviewed.

Start, stop, and restart requests for a mapped VM enter registry admission and
the retained `RuntimeProvider` instance. The daemon constructs a bounded,
workload-scoped operation context from the trusted mapping, and the concrete
adapter preserves the original lifecycle flags and caller role when invoking
the existing daemon authority. Restart is a provider stop followed by a
provider start, and host-shutdown stop uses the same route. VMs without an
explicit provider row retain the direct compatibility path.

Mutation deadlines cover the daemon's complete lifecycle budget rather than a
fixed provider timeout. Start sums each sequential DAG node's spawn and
readiness or API-readiness budget, qemu-media boot readiness, every possible
rollback TERM/KILL and cgroup-kill/emptiness wait, and snapshot margin. Stop
sums the configured graceful timer plus its bounded request and trailing poll
overhead, every declared or currently tracked role's TERM/KILL waits, broker
cgroup-kill request and post-kill emptiness waits, and snapshot margin. A
strict start with configured USBIP claims adds the 15-second reconciliation
window once. Stop adds each trusted configured claim's bounded guest detach,
firewall withdrawal, host unbind, and proxy reconciliation phases. Each stop
phase reserves 15 seconds, for 60 seconds per claim. USB-free VMs add no USBIP
cost, and a no-wait start does not charge its detached background
reconciliation to the mutation. Restart is the sum of stop and start. All sums
and claim multiplications are checked. Startup rejects a mapped runtime whose
full restart budget exceeds the provider contract maximum; it never truncates
a required cleanup budget.

Runtime adapters retain each admitted mutation in an owned task keyed by its
operation and idempotency identity. The task also retains exclusive per-VM
lifecycle authority until cleanup and snapshot work ends. If the provider
waiter expires after dispatch, it reports an ambiguous result requiring
observation while the daemon task continues through normal cleanup. A retry of
that operation joins the retained task and cannot dispatch the mutation twice.
Fresh operation identities and other mutations for that VM wait for the
retained lifecycle authority; unrelated VMs remain independent. Read-only
inspection does not need this retention and remains cancellable.

Per-VM lifecycle admission runs on the daemon's blocking adapter rather than
the provider executor. The wait observes both the provider cancellation token
and its effective deadline. If the provider drops an admission future, the
abandoned waiter cannot later acquire authority or dispatch work after the
active mutation completes.

Mapped lifecycle polls the retained synchronous broker, cgroup, and readiness
implementation through a dedicated Tokio blocking adapter. The provider's
current-thread bridge remains free to drive deadlines, cancellation
observation, and unrelated timers while cleanup continues under the owned
lifecycle task and per-VM permit.

The local observability binding carries no realm, workload, provider, label, or
cardinality payload. Descriptor placement supplies realm authority, and the
configuration schema fixes explicit maxima for record count, bytes, and time
window. Queries use bounded opaque cursors. The metrics projection deliberately
drops VM and source labels; audit projection exposes only closed sink-health
states. Export writes only those bounded projections to the provider-owned sink
and has no audit repair or unbounded-read authority. Startup probes
`ObservabilityStatus` through the retained registry.

Other first-party host implementation crates remain dependencies so their exact
factory contracts are checked and available for the eventual composition
cutover, but they are not registered by the production artifact yet:

- local transport needs generated endpoint bindings tied to the daemon's
  authenticated socket/vsock authority;
- host substrate needs opaque host-plan and host-apply intent mappings;
- Wayland display needs generated session and proxy intent mappings;
- local realm networking needs realm-scoped bridge, TAP, and network-operation
  mappings;
- local storage needs generated storage and synchronization row IDs;
- host-mediated devices need typed device and broker-operation mappings;
- PipeWire/vhost-user audio needs generated route and runner mappings.

These axes are intentionally absent rather than backed by no-op success. The
provider registry currently mediates mapped runtime lifecycle requests and
bounded local observability reads/exports; the other v1 behavior stays on its
existing paths until each axis gains a complete generated mapping and concrete
adapter.

## Process placement

Credential and cloud transport authority stays out of the host registry:

- `systemd-user`, Azure Container Apps, and Azure Relay are constructed only by
  the generic agent composer with ports supplied by the co-located agent.
- Entra and managed-identity credential providers are composed only in their
  exact provider-agent process. Credential bytes never enter `d2bd`.
- Secret Service composition belongs to `d2b-userd`; the host daemon has no
  Secret Service provider dependency.
- Both Azure VM implementation IDs are rejected before any factory or fake SDK
  construction. Neither Azure VM crate is in a production dependency graph.
- `RuntimeExecute` is not dispatchable and cannot be advertised by a live
  registry entry.
