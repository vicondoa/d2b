# `d2b-provider-guest`

The Guest resource family: the driver for the `Guest` resource type over the
four runtime Providers that realize it, the driver declaration the resource
plane registers the type by, and the Guest-side target-control service the
guest daemon serves.

## Resource family

One factory serves every runtime Provider row of the family:

| Provider | Kind | Children |
| --- | --- | --- |
| `runtime-cloud-hypervisor` | `CloudHypervisor` | committed by the provider controller session |
| `runtime-qemu-media` | `QemuMedia` | the runtime Volume and the VMM Process |
| `runtime-azure-container-apps` | `AzureContainerApps` | the sandbox-agent control Endpoint |
| `runtime-azure-virtual-machine` | `AzureVirtualMachine` | none |

A stored `Guest` spec selects its Provider row by `spec.providerRef`; that
selection picks the kind, and the kind fixes the effect keying, the child set,
the resync cadence, and the teardown order the driver preserves. The Provider,
controller, and child-Provider references are the realizer crates' own
constants, so the table cannot drift from the Providers it names.

## Declarations

`guest_descriptor(args)` builds the type's descriptor. `Guest` is
`BUILTIN | STARTUP` and is not exportable, so the plane refuses to open
without a guest driver. The declaration carries the type's verbs, execution
domains (`host`, `guest`), reads, and the children every runtime Provider
creates on the driver's behalf - all controller-owned: the Cloud Hypervisor
controller session commits its fixed child roles through the plane's child
bridge, and the qemu-media and azure-container-apps controllers own the
runtime Volume, the VMM Process, and the sandbox-agent Endpoint.

## Effect port

The driver reaches every host effect through `GuestDriverEffects`: one typed
`reconcile` call and one typed `finalize` call per kind. The production
implementation - the Cloud Hypervisor controller session and the preserved
framework state machines - lives in the daemon behind this port. Manager-
routed child reads cross the same boundary through `GuestChildSurface`, so the
driver keeps the only mutable context and the effects never touch the store.

## Target control

`target_service` is the Guest half of the U13 target-control protocol: one
ttrpc method on the same authenticated ComponentSession that carries the
Guest-local Resource API, the live-session generation fence, and the
target-local effect map. `target_control` is the host half: a
generation-bound channel over a session port the daemon implements
(`GuestTargetSession`), so the family owns the channel's policy and the daemon
owns the carrier.

## Placement and dependencies

`Guest` rows carry the canonical `spec.executionRef`, which names the `Host`
carrying the guest or the containing `Guest` of a nested provider scope. The
crate depends on the resource contracts, the runtime's driver, decoder, and
target-control contracts, the declaration vocabulary, and the four realizer
crates whose identities and child Providers it declares. It imports no daemon
runtime, broker, or store internals.

## Security posture

The driver never invents a child identity, a provider reference, or a session
generation: children are declared, Provider rows are read through the manager,
and every effect call binds the controller generation. A spec that names a
Provider outside the family is a terminal refusal, and a target-control frame
that names another zone, another generation, or a type with no registered
target-local effect is refused before any state exists.

## State and telemetry

Guest status is the row's own: the driver publishes the closed phase plus the
Provider's layered `status.resource` projection and persists nothing of its
own (R11). Failures travel as registered failure kinds on the structured
failure surface. The crate emits no telemetry of its own.

## Build and test

```bash
cargo test -p d2b-provider-guest
bazel test //packages/d2b-provider-guest:all-tests
```

The driver's behavior tests run over a scripted effect port;
`tests/registration.rs` proves the declaration registers the type with its
decoder and factory, that a second registration is refused, that the declared
mask cannot arrive after the plane opens, and that the declared children are
the ones the family's Providers create. The Guest-side target-control tests
run from the daemon crate (`packages/d2bd/tests/guest_target_service.rs`),
because their scene is the daemon's authenticated session runtime.
