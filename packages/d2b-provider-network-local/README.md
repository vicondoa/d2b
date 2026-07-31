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

The Provider implements `Network`. The crate supplies the asynchronous
controller state machine, typed config Volume, Guest, guest-agent and mDNS
child projections, deterministic IfName admission, complete bridge-port
readback, projection-scoped nftables policy, route readiness, and ordered IPv6
suppression.

## Controllers / services / workers / binaries

The `d2b-provider-network-local-ctrl` controller is Host-placed and uses the
library through injected resource and network effect ports. The net-VM
guest-agent and optional mDNS workers run inside the owned Guest. The controller
never invokes `nft`, netlink, or the privileged broker directly.

## Placement and dependencies

The controller is placed on the configured Host and the guest-agent is placed
inside the net VM. The crate depends only on provider-neutral contracts. It has
no dependency on the daemon, privileged broker, host implementation crate,
resource store, or another Provider implementation.

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
- The generic net-VM module force-neutralizes `10-eth-dhcp`, matches both NICs
  by MAC, suppresses IPv6, and installs an IPv6 drop-all table. It contains no
  per-Network DHCP, DNS, route, nftables, attachment, or mDNS desired data.
- The guest-agent receives network capabilities only in the Guest network
  namespace. The Host controller has no ambient network capabilities.

## State and telemetry

The types that can contain an interface name, address, rule, marker, or table
bytes implement redacted `Debug` and no value-bearing `Display`. Errors expose
only closed reason codes. Firewall status stores a projection-only digest, not
rules or device-owned marker churn. Metric keys and values come from closed
semantic sets and contain no Zone, Network, resource, VM, caller, address, path,
or interface identity. This layer owns no durable Provider state.

## Build and test

```bash
cd packages && cargo check -p d2b-provider-network-local
cd packages && cargo test -p d2b-provider-network-local
cd packages && cargo test -p d2b-provider-network-local --test '*'
```

Run the declared provider-system scenarios only through the repository's
`make test-integration` and `make test-host-integration` entrypoints. For future
standalone-repository packaging, preserve the dependency direction, four-path
crate layout, and same contract and controller-toolkit revisions as core.
