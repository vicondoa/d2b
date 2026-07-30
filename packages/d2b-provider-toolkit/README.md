# `d2b-provider-toolkit`

The Provider authoring toolkit: a Provider-neutral common library that declares
no Provider identity of its own.

Because it is a common library rather than a Provider, the per-Provider
sections below are answered from that position rather than left as
placeholders.

## Provider identity

None, deliberately and permanently. A common library cannot register a second
Provider identity or become a hidden multi-Provider composition binary.

## Config schema

None. It offers the redaction, audit, dispatch, conformance, and fake-port
helpers a Provider crate would otherwise re-derive, and reads no configuration
of its own.

## Exported resource types

None. The conformance kit checks a Provider's declared `ResourceApiBinding`
against the installed ResourceType contracts; it owns no ResourceType.

## Controllers / services / workers / binaries

None. The crate is a library with no binary target.

## Placement and dependencies

Linked into a Provider agent binary. Its only dependency is the shared v3
contract catalog: it imports no daemon, broker, Zone-store, Nix-emitter, or
Provider implementation internals.

## RBAC requirements

None. A bootstrap identity names who the agent is so it can label an audit
event and refuse a Zone it was not placed in; it authorizes no call, route, or
effect. Authorization stays with ComponentSession admission and the Zone RBAC
binding.

## Security posture

No privileged mutation, no broker, D-Bus, or systemd socket, no host path
resolution, no process spawn, and no direct-effect escape. The `fakes` module
is hermetic by construction: its supervisor records a launch intent and never
spawns, its effect port records an intent and mutates nothing, and its bus
resolves one declared dependency alias without ever handing back its binding
table.

## State and telemetry

No persistent state. The audit ring is bounded and in-memory, and it counts
drops rather than growing. No caller-supplied value, alias binding, resource
name, artifact identifier, or digest reaches a `Debug`, `Display`, audit
record, or metric label.

## Build and test

```bash
cd packages && cargo test -p d2b-provider-toolkit
cd packages && cargo clippy -p d2b-provider-toolkit --all-targets
```

Heavier container, Host, Guest, and cross-process fixtures live in
`integration/`; everything under `src/` and `tests/` is fast, in-process, and
parallel-safe.
