# `d2b-provider-process`

The Process resource family: the driver for the `Process` and
`EphemeralProcess` resource types, their declarations, the spec decoder and
driver factory the plane's registry serves, and the neutral launch and
supervision primitives every Process Provider exchanges.

## Resource family

`Process` is the durable row: a launch, a liveness observation, a restart
policy, and a term-then-kill delete. `EphemeralProcess` is the one-shot row:
one launch, a bounded runtime deadline, a terminal outcome, and a retention
Ttl. One factory serves both, and each type registers its own descriptor, so
the plane's registry keys them separately.

## Declarations

`process_family_descriptors(args)` builds the two descriptors. Both types are
`BUILTIN | STARTUP` and neither is exportable, so the plane refuses to open
without a process launcher. The descriptors carry no broker operations and no
child creations today.

## Effect port

The driver reaches every host effect through `ProcessDriverEffects`: launch,
adopt, probe, stop, finalize, and the typed Device-worker launch parameters.
The production implementation lives in the daemon behind this port; the family
crate holds no host state, no path, and no numeric identity.

## Placement and dependencies

The crate depends on the resource contracts, the runtime's driver and decoder
contracts, the declaration vocabulary, the process conformance surface, and
the two Process realizer crates whose identities a Process spec may select. It
imports no daemon, broker, or store internals.

## Security posture

Every launch goes through the signed provider-ticket path behind the port; the
driver assembles no ticket and no argument vector. Identity ambiguity is
quarantined, never signalled, and a refused one-shot launch is terminal. Errors
are closed codes; the details the driver reports carry field names and
comparisons, never the values it read.

## State and telemetry

The restart budget, the one-shot runtime clock, and the terminal outcome are
runtime memory only: nothing is persisted and no restart annotation is written
to the durable envelope. The crate emits no telemetry of its own.

## Build and test

```bash
cargo test -p d2b-provider-process
bazel test //packages/d2b-provider-process:all-tests
```

The driver's behavior tests run over a scripted effect port; the registry test
proves both member types register from the family's declarations.
