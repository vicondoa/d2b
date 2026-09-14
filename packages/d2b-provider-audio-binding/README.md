# `d2b-provider-audio-binding`

This is the crate root for the `audio.d2bus.org.AudioBinding` ResourceType, one of the six
resource types of the interaction family. It owns the binding's driver, its spec decoder, its child-intent port, and the driver declaration the v3 resource plane registers the type by; the driver verbs come from the interaction family's shared engine in `d2b-provider-wayland-policy`.

## Provider identity

| Field | Value |
| --- | --- |
| Provider identity | `audio-binding` (resource-type owner) |
| ResourceType | `audio.d2bus.org.AudioBinding` |
| Package | `packages/d2b-provider-audio-binding/` |
| Driver declaration | `audio_binding_descriptor` -> `DriverDescriptor` |
| Registration | `packages/d2b-provider-audio-binding/tests/registration.rs` |

## Config schema

The audio Provider's typed binding spec (`providerRef`, service reference, guest target, zone, channels, extension). Validate re-inserts the universal `providerRef` and decodes the spec typed.

## Exported resource types

`AudioBinding` is not exportable (`exportable: false`).

## Controllers / services / workers / binaries

One driver, `AudioBindingDriver`, built by `AudioBindingFactory`. The binding owns the audio worker Process children and their Endpoint children the Provider's controller declares; the daemon serves them behind `AudioBindingChildSource` and this crate materializes them into manager child rows.

## Placement and dependencies

The crate depends on the resource contracts, the provider wire contracts, the runtime contracts, the declaration vocabulary, the audio Provider's spec vocabulary, and the family engine crate.

## RBAC requirements

Reads follow the zone RBAC rows for the type; the driver holds no host audio handle and applies no grant.

## Security posture

The binding row selects `Provider/audio-pipewire` structurally, and the guest target it attaches to is read from the row, never inferred. Refusals are closed codes.

## State and telemetry

The audio lease and microphone arbitration live with the Provider's controller; the driver keeps an in-memory status projection only.

## Build and test

```bash
cargo test -p d2b-provider-audio-binding
bazel test //packages/d2b-provider-audio-binding:all-tests
```

`tests/registration.rs` proves the declaration, the service/target reads, and that the Provider's own child intents materialize into four manager child rows.
