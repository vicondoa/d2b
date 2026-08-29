# d2b strategy

## Product purpose

d2b is an opinionated NixOS framework for a single-user Wayland desktop that
needs separate trust boundaries without separate computers. It makes isolated
Linux microVM workloads feel like one desktop while keeping the host as the
trusted place for operator identity and credentials.

The active d2b product and control-plane model is Zone and Zone-owned
resources. A Zone is the isolation, policy, routing, resource, state, and
audit boundary; every resource belongs to one Zone, and each Zone has an
isolated runtime for its store, controllers, and Provider sessions. Generic
environment names remain useful descriptions of workload context, but they
are not a competing product hierarchy.

## Target user and outcome

The target user runs work, personal, agent, development, or risky browsing
workloads on one trusted NixOS host. The desired outcome is reasonable,
declarative isolation with one operator surface: each Zone owns the resources
that give its workloads identity, network policy, files, devices, and risk
profile while graphical applications remain integrated with the host desktop.

## Ownership and architecture

d2b owns its execution substrate end to end. The daemon-only control plane is
`d2bd` plus `d2b-broker`: the daemon supervises isolated per-Zone runtimes and
their resource lifecycle, while the privileged broker performs audited host
mutations. The framework keeps runner processes behind typed broker
operations and does not expose a second per-workload control plane.

## Isolation and security posture

The host is trusted; workloads are not. Per-Zone networks, closure-only
per-Guest `/nix/store` views, mediated devices, dedicated sidecar identities,
brokered host mutation, and Guest boundaries reduce cross-workload exposure.
Network resources retain their gateway fields. The intended gateway-backed
isolation model will use Zone-owned Guest and ZoneLink resources; once that
path is implemented, its gateway credentials, remote registries, and gateway
audit stay inside the relevant Guest rather than moving into the host control
plane. The framework does not make an insecure host safe or provide
multi-tenant isolation.

## Declarative contract

NixOS configuration is the source of truth for Zones and their owned
resources, including workloads, policies, providers, and optional components.
Generated manifests and private bundle artifacts give the daemon and broker
versioned, typed inputs. Changes to an active contract move its schema,
emitter, documentation, and supporting evidence together.

## Current direction

Current v3 work is the Zone-only control plane: Zone-owned resource identity,
isolated per-Zone runtime state, and mediated desktop features behind the local
Rust CLI and daemon fast path. The framework continues to favor explicit
boundaries, auditable ownership, restart-safe lifecycle, and
compositor-agnostic presentation metadata over broad orchestration or
host-side convenience services.

See [`README.md`](./README.md), the
[`design overview`](./docs/explanation/design.md), and accepted decisions
[ADR 0015](./docs/adr/0015-daemon-only-clean-break.md),
[ADR 0018](./docs/adr/0018-microvm-nix-removal.md), [ADR
0021](./docs/adr/0021-broker-user-namespace-for-virtiofsd.md), and [ADR
0034](./docs/adr/0034-storage-lifecycle-restart-and-synchronization.md),
[ADR 0043](./docs/adr/0043-realm-native-control-plane.md) for the historical
realm-native and gateway-isolation context, and [ADR
0046](./docs/adr/0046-d2b-3-provider-control-plane.md) for the current
Zone-native product and architecture context.
