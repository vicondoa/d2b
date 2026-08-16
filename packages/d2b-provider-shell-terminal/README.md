# `d2b-provider-shell-terminal`

This crate implements the policy and lifecycle contracts for
`Provider/shell-terminal`. It models qualified pool and session resources,
current-request authorization, workload-user supervisor placement, bounded PTY
replay, stale-generation refusal, and verified restart adoption.

See [Create a Provider](../../docs/how-to/create-provider.md) and the
[shell-terminal dossier](../../docs/specs/providers/ADR-046-provider-shell-terminal.md)
for the implementation contract.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `shell-terminal` |
| Provider reference | `Provider/shell-terminal` |
| Package | `packages/d2b-provider-shell-terminal/` |

## Config schema

- `shell-terminal.d2bus.org.ShellPool` binds one Host or Guest target, one
  workload user, a manifest-fixed login shell artifact, and bounded capacity.
- `shell-terminal.d2bus.org.ShellSession` freezes those inherited fields and
  owns one workload-user session supervisor.
- The controller owns no Provider state volume. PTY bytes and attachment state
  stay in the supervisor's bounded in-memory ring.

## Exported resource types

- `shell-terminal.d2bus.org.ShellPool`
- `shell-terminal.d2bus.org.ShellSession`

## Controllers / services / workers / binaries

- `ShellTerminalController` creates pool-derived sessions after authorizing
  the current request. It requires a `ShellAuthorityPort` supplied by `d2bd`;
  the port owns durable generation fencing, one-shot capability consumption,
  and pool-wide attachment admission across controller and supervisor
  processes.
- `SessionSupervisor` owns one session's bounded PTY replay and attachments.
- `InMemoryShellAuthority` is a hermetic test fake only. Production uses a
  daemon authority client reconstructed from reconciled resource status and
  the operation ledger after restart.
- This provider crate declares no workers or binaries.

## Placement and dependencies

Session supervisors use the workload user's systemd user domain on the
Pool-selected Host or Guest target. The crate depends only on typed contracts
and process-conformance effects; it cannot open broker or system-manager
connections.

## RBAC requirements

All service verbs authorize the current Zone-scoped request before capacity or
route lookup. Relay-authenticated callers cannot use Host user-domain pools.
`ShellAdmin` and `ZoneAdmin` are the only accepted roles.

## Security posture

- Session supervisors use the typed `Provider/system-systemd` user-domain
  process conformance seam. The provider cannot open a broker or
  system-manager connection, inherit credentials, or receive raw process
  identifiers.
- Provider-local `Arc` values are transport clients only. They cannot own
  session generations, capabilities, or attachment quotas, which are always
  checked by the daemon authority port.
- The public controller service is `shell-terminal.v3`; supervisors expose
  `shell-session-supervisor.v1` and the named `terminal` stream.
- Reconnect replays only bounded in-memory output. Stale generations and
  reused capabilities refuse. Detach does not terminate a shell.

## State and telemetry

Diagnostics and metrics use closed labels and bounded retention. Terminal
bytes, paths, process identifiers, user names, session names, credentials, and
opaque handles do not enter debug output or provider observations.

## Build and test

```bash
cargo check -p d2b-provider-shell-terminal
cargo test -p d2b-provider-shell-terminal
```

The hermetic suite covers resource shape, authorization, process conformance,
placement, ring buffering, restart adoption, stream contracts, and bounded
observability.
