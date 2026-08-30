# Critical subsystems

Read this page before changing a load-bearing boundary. The repository root
[`AGENTS.md`](../../AGENTS.md) is the binding index; current code and owner
tests are authoritative.

## Zone networking and firewall

**Where:** `packages/d2b-provider-network-local/nix/net.nix`,
`packages/d2b-provider-network-local/src/`, and broker Network operations.

Network Providers own Zone topology, gateway policy, interface names, and
firewall neutralization. Preserve the `10-eth-dhcp` `lib.mkForce` neutralizer,
fail closed on foreign ownership markers, and keep east-west access explicit.
Validate Nix behavior with
`tests/unit/nix/cases/net-vm-network.nix`.

## Per-Guest store views

**Where:** `packages/d2b-provider-volume-local/nix/`, the storage bundle
contract, and broker `StoreSync` operations.

Each Guest sees only its declared closure through a broker-owned store view.
The host's complete `/nix/store` is never shared as a Guest filesystem.
Restart adoption, anchored paths, OFD locks, explicit fd transfer, and the
single repair owner rule are load-bearing. No broad cleanup, chmod, chown,
or runtime-directory sweep is permitted.

## TPM and mediated devices

**Where:** device Providers under `packages/d2b-provider-device-*` and their
broker operations.

TPM state is persistent and identity-bound. A missing or replaced previously
provisioned state directory fails closed rather than silently creating a new
identity. USBIP, GPU, video, audio, and security-key effects use typed
Provider contracts, exact Zone/Guest identity, bounded device allowlists, and
broker-spawned runners. No caller-supplied device path or per-Guest systemd
unit may be introduced.

## ComponentSession and Zone bus

**Where:** `packages/d2b-session/`, `packages/d2b-session-unix/`,
`packages/d2b-bus/`, and `packages/d2b-resource-api/`.

Authenticated evidence is consumed by one session owner. Zone equality,
Resource UID, revision, Provider generation, capability, and cursor are
checked before every authority mint. Capability types remain sealed and
non-clonable; no public subject-registration or raw-claim constructor may
appear. The default bus is deny-all until authenticated Zone runtime
composition installs the registrar-bound capability.

## Resource mutation and controller effects

**Where:** `packages/d2b-resource-store/`,
`packages/d2b-resource-store-redb/`, `packages/d2b-resource-api/`, and
`packages/d2b-core-controller/`.

The store accepts mutations only through the concrete sealed capability and
the matching committed revision proof. Controllers create desired Resources
and observe status; effect owners perform effects only after durable commit.
Single-flight reconciliation, deterministic owner propagation, stale-proof
rejection, and restart-safe idempotency must remain intact.

## Guest lifecycle

**Where:** `packages/d2b-core-controller/`, `packages/d2bd/`,
`packages/d2bd-runtime/`, and the Cloud Hypervisor Provider.

The Guest controller derives deterministic direct children from the Guest
Resource and private Provider contract. It does not spawn, mount, provision,
bind, or call the broker. Direct children carry exact ownerRefs and are fenced
by Guest UID, child UID, generation, and revision. Readiness is status-first;
session loss is degraded and reconnect-safe; deletion drains descendants in
reverse order and clears the Guest finalizer last.

## Daemon and broker control plane

**Where:** `packages/d2bd/`, `packages/d2bd-runtime/`,
`packages/d2b-broker/`, and `packages/d2b-contracts/`.

Only `d2bd.service`, `d2b-broker.socket`, and `d2b-broker.service` are
framework-declared root units. The broker owns delegated cgroup mutation,
runner launch, pidfd handoff, host-device access, and child reaping. d2bd
adopts only matching immutable identity and quarantines PID/start-time drift.
Raw PIDs, host paths, credentials, argv, and private broker handles never
cross the public API.

## Provider assignment and scoped routing

**Where:** `packages/d2b-contracts-provider/`,
`packages/d2b-core-controller/`, and Zone bus/session tests.

Provider manifests carry a closed placement contract and required effect
classes. Assignments bind Resource UID/revision, Provider and controller
generations, target, session generation, and assignment epoch. Queries add the
assignment-owned filter; mutations reject forged or stale evidence. There is
no fallback target or caller-selected Provider identity.

## Unsafe-local and shell Providers

**Where:** `packages/d2b-unsafe-local-helper/`,
`packages/d2b-provider-shell-terminal/`, and
`packages/d2b-contracts-control/src/unsafe_local_wire.rs`.

Unsafe-local is an explicit, default-denied Host execution posture. It runs
only as the authenticated requester UID and never becomes a Guest or a
cross-identity launcher. Shell sessions use bounded names, exact requester
identity, a validated terminal fd, and bounded output cursors. No configured
argv, environment, host path, root service, or broad same-UID cleanup may be
exposed.

## Generated contracts

**Where:** `docs/reference/schemas/`, `docs/reference/cli-output/`,
`nixos-modules/generated/`, Provider manifests, and `packages/xtask/`.

Generate artifacts from owner-local Rust/Nix sources:

```bash
bazel run //packages/xtask:xtask -- gen-schemas
bazel run //packages/xtask:xtask -- gen-cli-schemas
bazel run //packages/xtask:xtask -- gen-zone-schemas
bazel run //packages/xtask:xtask -- gen-resource-schemas
```

Update schemas, emitters, manifests, signatures, fixtures, policy closures,
prose, and changelog together. Do not add a second inventory or drift gate.

## Required evidence

Use the smallest owner-local test first, then the applicable public aliases:

```bash
make test-nix-unit
make test-fixture-contracts
make test-rust-supply-chain
make test-policy
make test-drift
make test-flake
make test-unit
make check
```

Host and VM acceptance are separate higher tiers owned by U20. U20's scope is
the `/etc/nixos` switch, d2b startup, and Cloud Hypervisor Guest boot; an
advisory skip is not evidence for those checks. U20 must also run both
`make test-host-integration` and `make test-integration`, which may be
scheduled alongside real-host testing. U19 only converges their declarations
and current inputs. The host lane injects the Bazel-built d2b binary bundle
through `D2B_HOST_TOOL_BUNDLE`; it does not rebuild d2b binaries through Nix.
