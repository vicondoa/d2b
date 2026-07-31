# `d2b-provider-device-usbip`

This crate implements the USBIP Device Provider without importing the daemon,
broker, host effect implementation, or another Provider crate.

## Provider identity

The Provider identity is `Provider/device-usbip`. Its implementation artifact,
extension schemas, component templates, and trust evidence are supplied by the
signed Provider descriptor. The controller uses the provider-neutral USB
ResourceTypes and does not define a USBIP-qualified replacement type.

## Config schema

`Provider.spec.config` has one required field:

| Field | Type | Default | Validation |
| --- | --- | --- | --- |
| `controllerExecutionRef` | `ResourceRef` | none | A `Host/<name>` in the installing Zone |

The host and guest kernel module classes, executable artifacts, and standard
USBIP transport policy are signed Provider constants. Operators cannot supply
an executable path, raw device identifier, bus id, port, firewall body, or
authority key.

## Exported resource types

- `Device` owns physical USB inventory and presence in the owner Zone. It is
  not exportable.
- `usb.d2bus.org.UsbService` is the provider-neutral whole-device authority or
  imported projection. Only an authority Service is policy-gated exportable.
- `usb.d2bus.org.UsbBinding` is one Guest's desired attachment. It is not
  exportable.

Core-generated projections preserve `usb.d2bus.org.UsbService`, have no
`spec.provider`, and perform no local physical-device effect.

## Controllers / services / workers / binaries

The Service controller preserves the safety order of host authority, physical
claim, withhold, Network relay/firewall, bind, and readiness while calling only
semantic effect methods. Firewall acquisition and release use the shared closed
projection operation with `Apply` and `Remove`; release retains the firewall
token, status, and relay authority until Core confirms removal or validates
absence.

Long-lived workers are one shared Host backend, one multiplexed relay Endpoint
per Network, and one private proxy per attached Binding. Their signed templates
are `usbip-daemon`, `usbip-relay`, and `usbip-guest-proxy`. Module load, physical
claim, bind, unbind, and Guest attach or detach are one-shot semantic effects,
not additional Process resources.

## Placement and dependencies

The controller and shared backend run on the configured Host. The relay is
Host-placed but owned by one Core-derived authority per referenced Network. A
Binding proxy is placed in its exact Guest context.

The controller watches only Network identity, readiness, and generation. Core
privately resolves the Network attachment and exact per-device firewall intent.
The crate depends only on `d2b-contracts`; it has no broker or daemon dependency.

## RBAC requirements

The controller needs bounded read/watch access to its Device, USB Service,
Binding, Host, Network, Guest, Endpoint, export, and import dependencies. It may
write status and finalizers on its owned resources and create, update, or delete
only its owned Process and Endpoint children. It receives no direct broker
permission and no generic Network Endpoint resolution permission.

## Security posture

Every privileged mutation is performed by Core's injected effect adapter. The
Provider receives opaque resource identities and effect tokens only. It never
receives a device identifier, bus id, serial, path, host interface, address,
port, fd, ownership marker, firewall rule, caller identity, or authority digest.

Device and Network identities must belong to one Zone before a relay or
firewall effect. Core separately acquires the shared Host-global physical USB
backing authority before any physical effect. A misrouted attachment therefore
fails before exposing a security key to another Zone. Projection removal is
generation-fenced, ownership-scoped, idempotent after validated absence, and
foreign-marker fail-closed.

## State and telemetry

Bounded non-secret lifecycle state remains in Device, Service, and Binding
status plus the Core operation ledger. The Provider has no state Volume. USBIP
firewall drift and digest are strict Service Provider observations and never
mutate Network status.

Metrics use only the closed values `provider=device-usbip`,
`component=service-controller`, closed operation/outcome/error classes, and no
resource-derived label value. Errors and `Debug` output are identity-free. Core
owns post-effect audit records; raw device and firewall data never enter them.

## Build and test

```bash
cd packages
cargo check -p d2b-provider-device-usbip
cargo test -p d2b-provider-device-usbip
```

The `tests/` suite is hermetic. `integration/attach_detach_lifecycle.rs`
declares `host-integration` because a real module/backend/Guest lifecycle needs
a booted test host and must run only through `make test-host-integration`.

The crate boundary is suitable for a future standalone repository: preserve the
Provider identity, common USB contracts, injected semantic effect boundary,
and signed component descriptor while moving packaging and release metadata.
