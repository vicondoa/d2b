# d2b strategy

## Product purpose

d2b is an opinionated NixOS framework for a single-user Wayland desktop that
needs separate trust boundaries without separate computers. It makes isolated
Linux microVM workloads feel like one desktop while keeping the host as the
trusted place for operator identity and credentials.

## Target user and outcome

The target user runs work, personal, agent, development, or risky browsing
workloads on one trusted NixOS host. The desired outcome is reasonable,
declarative isolation with one operator surface: each workload gets its own
identity, network policy, files, devices, and risk profile while graphical
applications remain integrated with the host desktop.

## Ownership and architecture

d2b owns its microVM substrate end to end. The daemon-only control plane is
`d2bd` plus `d2b-broker`: the daemon supervises lifecycle DAGs and the
privileged broker performs audited host mutations. The framework declares no
per-VM systemd templates and keeps runner processes behind typed broker
operations.

## Isolation and security posture

The host is trusted; workloads are not. Per-environment networks, closure-only
per-VM `/nix/store` views, mediated devices, dedicated sidecar identities,
brokered host mutation, and guest boundaries reduce cross-workload exposure.
The framework does not make an insecure host safe, does not provide
multi-tenant isolation, and does not move realm credentials into the host
control plane.

## Declarative contract

NixOS configuration is the source of truth for environments, realms,
workloads, policies, and optional components. Generated manifests and private
bundle artifacts give the daemon and broker versioned, typed inputs. Changes to
that contract move the schema, emitter, documentation, and compatibility
evidence together.

## Current direction

Current v3 work keeps the local Rust CLI and daemon fast path while extending
realm-native workload identity and mediated desktop features. The framework
continues to favor explicit boundaries, auditable ownership, restart-safe
lifecycle, and compositor-agnostic presentation metadata over broad
orchestration or host-side convenience services.

See [`README.md`](./README.md), the
[`design overview`](./docs/explanation/design.md), and accepted decisions
[ADR 0015](./docs/adr/0015-daemon-only-clean-break.md),
[ADR 0018](./docs/adr/0018-microvm-nix-removal.md), [ADR
0021](./docs/adr/0021-broker-user-namespace-for-virtiofsd.md), and [ADR
0034](./docs/adr/0034-storage-lifecycle-restart-and-synchronization.md), and
[ADR 0043](./docs/adr/0043-realm-native-control-plane.md) for the committed
product and architecture context.
