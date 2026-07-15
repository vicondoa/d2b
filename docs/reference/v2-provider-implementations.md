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
