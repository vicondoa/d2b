# `d2b-provider-audio-service`

This is the crate root for the `audio.d2bus.org.AudioService` ResourceType, one of the six
resource types of the interaction family. It owns the service's driver, its spec decoder, and the driver declaration the v3 resource plane registers the type by; the driver verbs come from the interaction family's shared engine in `d2b-provider-wayland-policy`.

## Provider identity

| Field | Value |
| --- | --- |
| Provider identity | `audio-service` (resource-type owner) |
| ResourceType | `audio.d2bus.org.AudioService` |
| Package | `packages/d2b-provider-audio-service/` |
| Driver declaration | `audio_service_descriptor` -> `DriverDescriptor` |
| Registration | `packages/d2b-provider-audio-service/tests/registration.rs` |

## Config schema

The audio Provider's typed service spec (`providerRef`, service role, implementation endpoint references, operations, grants, extension). Validate re-inserts the universal `providerRef` the envelope split removed and decodes the spec typed.

## Exported resource types

`AudioService` is not exportable (`exportable: false`).

## Controllers / services / workers / binaries

One driver, `AudioServiceDriver`, built by `AudioServiceFactory`. The service realizes no child resources; the audio Provider's controller registry stays in the daemon behind the family effect port.

## Placement and dependencies

The crate depends on the resource contracts, the runtime contracts, the declaration vocabulary, the audio Provider's spec vocabulary, and the family engine crate. It imports no daemon internals.

## RBAC requirements

Reads follow the zone RBAC rows for the type; the driver performs no privileged operation and applies no host audio grant itself.

## Security posture

The service row selects `Provider/audio-pipewire` structurally: a row naming another Provider is refused before any effect. Refusals are closed codes and the driver reports no grant value.

## State and telemetry

The driver publishes an in-memory status projection only; host audio state stays with the Provider's controller and the daemon's mediator.

## Build and test

```bash
cargo test -p d2b-provider-audio-service
bazel test //packages/d2b-provider-audio-service:all-tests
```

`tests/registration.rs` proves the declaration, the required Provider selector, and that the typed row decodes while a foreign row is refused.
