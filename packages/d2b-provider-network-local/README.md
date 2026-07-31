# `d2b-provider-network-local`

Host-network policy and observation primitives for `Provider/network-local`.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `network-local` |
| Publisher | first-party, `vicondoa/d2b` |
| Version | tracks the workspace version of this crate |
| Trust attestation | exact package digest from the offline artifact catalog |
| Conformance attestation | hermetic tests under `src/` and `tests/` |

## Config schema

The Provider config contains `controllerExecutionRef`, a same-Zone `Host`
reference selecting controller placement. Network policy is authored on the
`Network` ResourceType, not in Provider config. There is no raw broker option,
interface-name option, ownership-marker option, or alternate host mutation
command.

## Exported resource types

The Provider implements `Network`. This crate currently supplies its reusable
host-fabric policy layer: deterministic IfName admission, complete bridge-port
defaults and readback, per-Network nftables projections, route and net-VM
readiness checks, and ordered IPv6 suppression.

## Controllers / services / workers / binaries

The eventual `d2b-provider-network-local-ctrl` controller is Host-placed and
uses these primitives through an injected network effect port. No controller
binary, service, or worker is introduced by this library slice. In particular,
this crate never invokes `nft`, netlink, or the privileged broker directly.

## Placement and dependencies

The controller is placed on the configured Host. This crate depends only on the
provider-neutral contracts. It has no dependency on
the daemon, privileged broker, host implementation crate, resource store, bus,
or another Provider implementation.

## RBAC requirements

The controller identity requires Network status/finalizer authority and the
bounded child-resource permissions declared by the Provider role. These pure
helpers grant no authority. Host mutation remains in the core effect adapter
and the broker authorization matrix.

## Security posture

- Interface names use the canonical redacted contract implementation. Collision
  admission runs across the complete Host domain before a link effect.
- Workload bridge ports default to isolated and readback checks every flag.
- Firewall apply and remove are projection-scoped. There is no whole-table
  replacement API. Sibling Network, device-owned, and foreign bytes are
  preserved, while a foreign marker in the expected Network slot fails closed.
- Network projections reject USBIP and TCP/3240 rules; those remain owned by the
  device Provider.
- Cross-Zone bridge-mode physical-NIC multiplexing uses the canonical
  Host-global admission check and is rejected before any host effect.
- IPv6 suppression runs before a new bridge is brought up and is re-applied on
  reconciliation as defense in depth.

## State and telemetry

The types that can contain an interface name, address, rule, marker, or table
bytes implement redacted `Debug` and no value-bearing `Display`. Errors expose
only closed reason codes. Firewall status stores a projection-only digest, not
rules or device-owned marker churn. This layer owns no durable Provider state.

## Build and test

```bash
cd packages && cargo check -p d2b-provider-network-local
cd packages && cargo test -p d2b-provider-network-local
```

The declared container scenario under `integration/` becomes executable when
the core adapter and the closed broker operations have production handlers.
For future standalone-repository packaging, preserve the dependency direction,
the four-path crate layout, and the same contract-crate revision as core.
