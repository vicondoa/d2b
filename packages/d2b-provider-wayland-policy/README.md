# `d2b-provider-wayland-policy`

This is the crate root for the `display-wayland.d2bus.org.WaylandPolicy` ResourceType, one of the six
resource types of the interaction family. It is also the interaction family's shared engine: `interaction` carries the reconcile, recover, finalize, and delete verbs, the spec-envelope decode, the manager-child plumbing, and the effect port every interaction type drives. It owns the policy type's driver, its spec decoder, and the driver declaration the v3 resource plane registers the type by.

## Provider identity

| Field | Value |
| --- | --- |
| Provider identity | `wayland-policy` (resource-type owner) |
| ResourceType | `display-wayland.d2bus.org.WaylandPolicy` |
| Package | `packages/d2b-provider-wayland-policy/` |
| Driver declaration | `wayland_policy_descriptor` -> `DriverDescriptor` |
| Registration | `packages/d2b-provider-wayland-policy/tests/registration.rs` |

## Config schema

Nothing outside the envelope: the policy document is the whole spec, so validate only requires a JSON object.

## Exported resource types

`WaylandPolicy` is not exportable (`exportable: false`): the type names a Zone's display policy, which is never an export subject.

## Controllers / services / workers / binaries

One driver, `WaylandPolicyDriver`, built by `WaylandPolicyFactory`. The type realizes no child resources; process and endpoint work belongs to `WaylandSession` rows.

## Placement and dependencies

The crate depends on the resource contracts, the runtime's driver and decoder contracts, the declaration vocabulary, the display Provider's spec vocabulary, and the core-controller child vocabulary the shared engine materializes. It imports no daemon, broker, or store internals.

## RBAC requirements

Reads follow the zone RBAC rows for the type; the driver performs no privileged operation of its own and opens no session.

## Security posture

The policy envelope carries no secret and the driver records no value it read: refusals are closed codes (`interaction-spec-invalid`, `interaction-child-mutation`, `interaction-unavailable`, `interaction-delete-pending`).

## State and telemetry

The driver publishes an in-memory status projection only; nothing is persisted and the crate emits no telemetry of its own.

## Build and test

```bash
cargo test -p d2b-provider-wayland-policy
bazel test //packages/d2b-provider-wayland-policy:all-tests
```

`tests/engine.rs` drives every shared verb over scripted effects and a recording manager (child ensure before the effect, one watch per target, endpoint-first retirement, drain-before-Provider teardown, the closed validate surface); `tests/registration.rs` proves the declaration the plane registers.
