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
health and inspect through that retained registry before serving requests.
Duplicate or missing bindings, descriptor/factory mismatches, stale
generations, scope or placement mismatches, unavailable intent mappings, and
non-dispatchable capabilities abort startup. The registry is not reconstructed
per call. An empty registry is valid only when the artifact explicitly contains
zero provider rows.

The generated artifact carries canonical realm, workload, and provider IDs plus
opaque references to existing bundle intents. It contains no argv, host path,
credential, or secret payload. `bundleVersion` 12 adds the artifact; its own
schema remains version `v2`. Provider activation accepts exactly bundle version
12 with bundle schema `v2`, a declared provider artifact path, a bundle hash,
and an artifact-hash entry for that path. Older bundle versions remain readable
by compatibility consumers but cannot activate this registry.

The live host registry currently registers only local runtime providers:

| Axis | Live implementations | Mapping and daemon authority |
| --- | --- | --- |
| Runtime | `cloud-hypervisor`, `qemu-media` | Explicit realm workloads map to matching VM-start and runner intent IDs. The daemon authenticates the mapping against the process DAG, observes its pidfd table, and calls the existing lifecycle start/stop authority through the provider adapter. |

Only explicit realm workload rows with a matching generated VM process DAG are
eligible. Realm and workload IDs must derive exactly from the DAG's
`workloadIdentity`; each VM and intent pair has one owner; and the runner must
carry explicit executable and argv data rather than the legacy synthesis
fallback. A first-class workload without existing VM-start and runner intents is
not emitted as live. Azure VM IDs and `RuntimeExecute` are rejected.

Start, stop, and restart requests for a mapped VM enter registry admission and
the retained `RuntimeProvider` instance. The daemon constructs a bounded,
workload-scoped operation context from the trusted mapping, and the concrete
adapter preserves the original lifecycle flags and caller role when invoking
the existing daemon authority. Restart is a provider stop followed by a
provider start, and host-shutdown stop uses the same route. VMs without an
explicit provider row retain the direct compatibility path.

Mutation deadlines cover the daemon's complete lifecycle budget rather than a
fixed provider timeout. Start includes the configured readiness wait and
rollback/snapshot margin; stop and restart include graceful shutdown,
SIGTERM/SIGKILL cleanup, and snapshot margin. These budgets are capped by the
provider contract maximum. Runtime adapters retain each admitted mutation in an
owned task keyed by its operation and idempotency identity. If the provider
waiter expires after dispatch, it reports an ambiguous result requiring
observation while the daemon task continues through normal cleanup. A retry of
that operation joins the retained task and cannot dispatch the mutation twice.
Read-only inspection does not need this retention and remains cancellable.

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
- PipeWire/vhost-user audio needs generated route and runner mappings; and
- local observability needs bounded query/export mappings to current daemon
  projections.

These axes are intentionally absent rather than backed by no-op success. The
provider registry currently mediates only mapped runtime lifecycle requests;
the other v1 behavior stays on its existing paths until each axis gains a
complete generated mapping and concrete adapter.

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
