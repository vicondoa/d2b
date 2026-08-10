# `d2b-provider-device-tpm`

`Provider/device-tpm` manages one emulated TPM `Device` and its
state-preserving swtpm realization. The crate is intentionally independent of
the daemon, broker, and host implementation crates.

See [Create a Provider](../../docs/how-to/create-provider.md) for the
uniform crate layout, schema links, configuration, and test lanes.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `device-tpm` |
| Provider reference | `Provider/device-tpm` |
| ResourceType | `Device` |
| Package | `packages/d2b-provider-device-tpm/` |

The signed Provider descriptor owns the implementation identity and component
artifacts. This crate consumes only the neutral v3 contracts and opaque Core
effect interfaces.

## Config schema

The Device extension is
`device-tpm.d2bus.org/Device/spec`, version `1.0`. Its bounded setting is
`logLevel` (1-20, default 20). The pre-start flush is mandatory and has no
configuration toggle. Nix authors declare a `Device` under
`d2b.zones.<zone>.resources.<name>` with `providerRef =
"Provider/device-tpm"`, `deviceClass = "emulated"`, and exclusive
arbitration.

State directories, marker identities, executable paths, and Provider package
artifacts are not Device-spec fields. Core resolves those values from the
signed descriptor and private policy.

## Exported resource types

The Provider implements the standard `Device` ResourceType for one emulated
TPM. Its realization uses a persistent Device-owned TPM `Volume`, one
pre-start flush `EphemeralProcess`, and one long-lived swtpm `Process`.
Those child resources are semantic Core-managed effects, not alternate public
TPM ResourceTypes.

## Controllers / services / workers / binaries

`TpmController` owns the lifecycle order:
state-directory preparation and marker validation, mandatory flush, and
long-lived swtpm start. `TpmEffectPort` is the only effect boundary. The
Provider supplies opaque `FlushLaunchTicket` and
`SwtpmStartLaunchTicket` values to Core; it does not construct a broker
request.

The signed component supplies the swtpm and swtpm-ioctl binaries. A worker is
Ready only after preparation, flush, and swtpm start all succeed. Finalization
stops the owned worker and clears the Provider finalizer without deleting the
TPM Volume.

## Placement and dependencies

The controller and swtpm worker are Host-placed through Core's Process
controller. It reads its Zone `Device`, TPM `Volume`, and child Process
dependencies and writes only status and its own finalizer. The crate depends
on `d2b-contracts` and serde; privileged state and process effects are
implemented by Core and the broker.

## RBAC requirements

The Provider needs bounded read/watch access to its Device, Volume, and
Process dependencies and status/finalizer authority on its owned Device. It
does not receive a generic broker, filesystem, socket, executable, or host
mutation permission. Core derives and validates every effect token before
passing it to the Provider.

## Security posture

TPM state contains persistent NVRAM and identity material. A state directory
that was previously provisioned but no longer has its identity-bound marker
fails closed; it is never silently recreated. Owner/type mismatches and marker
identity mismatches also fail closed.

The controller receives no state path, socket path, UID, GID, pidfd, or raw
broker handle. `SignedBinaryRef`, state tokens, and launch tickets are opaque
and redact their values in `Debug`. The mandatory pre-start flush remains
before every swtpm launch.

## State and telemetry

The TPM payload remains in the Device-owned persistent Volume and is never
copied into Provider status or telemetry. Bounded lifecycle observations use
closed phase and error values. Metrics use fixed Provider, operation,
outcome, and error labels; Zone/resource names, paths, marker bytes, TPM
contents, and identity material are excluded. Core owns path-free audit
records for the resulting privileged effects.

## Build and test

```bash
cd packages
cargo test -p d2b-provider-device-tpm
cargo nextest run -p d2b-provider-device-tpm
cargo clippy -p d2b-provider-device-tpm --all-targets -- -D warnings
cargo run -p xtask -- check-provider-layout
```

The hermetic tests cover settings and argv shape, opaque ticket binding,
marker failure, flush ordering, worker failure, and finalizer Volume
preservation. The declared `integration/` scenarios require the existing
Host/Guest integration lane and are invoked through
`make test-host-integration`; no operator TPM is used by ordinary tests.

## Future standalone use

An extracted Provider repository would retain the Provider identity, signed
component descriptor, neutral contracts, and opaque Core effect adapter while
replacing workspace packaging and release metadata.
