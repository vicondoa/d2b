# `d2b-provider-audio-pipewire`

This is the canonical implementation crate for `Provider/audio-pipewire`.
It keeps the existing audio policy as the migration source while exposing
typed owner/projection admission, bounded speaker mixing, exclusive
microphone arbitration, and host/guest readiness through an AudioMediator
effect port.

See [Create a Provider](../../docs/how-to/create-provider.md) and the
[audio-pipewire dossier](../../docs/specs/providers/ADR-046-provider-audio-pipewire.md)
for the implementation contract.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `audio-pipewire` |
| Provider reference | `Provider/audio-pipewire` |
| Package | `packages/d2b-provider-audio-pipewire/` |

## Config schema

Provider configuration contains only bounded implementation settings such as a
capture alias. PipeWire sockets, node IDs, process argv, and store paths are
never public resource fields.

## Exported resource types

The Provider implements the provider-neutral
`audio.d2bus.org.AudioService` and `audio.d2bus.org.AudioBinding` contracts.
Bindings reference same-Zone Services and Guest targets; projection Services
cannot open PipeWire locally.

## Controllers / services / workers / binaries

`AudioBindingController` uses the existing policy DTO and a typed
`AudioMediator`. Private component templates omit live Process argv and
socket arguments; the provider also owns the signed audio argv projection.
Process launch remains owned by the Process Providers.

## Placement and dependencies

The owner mediator is a same-UID user-session effect boundary. Guest
readiness and host PipeWire readiness are separate status observations.

## RBAC requirements

Audio effects are requested through the mediator/controller boundary; no
direct broker, pidfd, filesystem, or user mutation is performed by this crate.

## Security posture

Projection routes are refused if they would open a local PipeWire session.
Microphone release mutes before the next queued lease is granted.

## State and telemetry

The Provider owns no state Volume. Grants are durable in AudioBinding spec;
telemetry uses only closed role/channel/outcome labels and rejects paths or
Zone/resource identity labels.

## Build and test

```bash
bazel test //packages/d2b-provider-audio-pipewire:d2b_provider_audio_pipewire_doc_test
```

The current test targets are structural compile checks. Executable scenarios
belong to the owning implementation.
