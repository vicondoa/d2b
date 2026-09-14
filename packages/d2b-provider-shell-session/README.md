# `d2b-provider-shell-session`

This is the crate root for the `shell-terminal.d2bus.org.ShellSession` ResourceType, one of the six
resource types of the interaction family. It owns the session's driver, its spec decoder, its supervisor-child derivation, its spec reference checks, and the driver declaration the v3 resource plane registers the type by; the driver verbs come from the interaction family's shared engine in `d2b-provider-wayland-policy`.

## Provider identity

| Field | Value |
| --- | --- |
| Provider identity | `shell-session` (resource-type owner) |
| ResourceType | `shell-terminal.d2bus.org.ShellSession` |
| Package | `packages/d2b-provider-shell-session/` |
| Driver declaration | `shell_session_descriptor` -> `DriverDescriptor` |
| Registration | `packages/d2b-provider-shell-session/tests/registration.rs` |

## Config schema

The Provider's reference document: `providerRef`, the pool reference (a `ShellPool`), the execution reference (Host or Guest), an optional user reference (User), and the bounded `artifact://` login-shell reference.

## Exported resource types

`ShellSession` is not exportable (`exportable: false`).

## Controllers / services / workers / binaries

One driver, `ShellSessionDriver`, built by `ShellSessionFactory`. The session owns one supervisor Process child, derived from the signed template the Process Provider launches; the driver has no spawn surface.

## Placement and dependencies

The crate depends on the resource contracts, the runtime contracts, the declaration vocabulary, the shell Provider's interval constant, and the family engine crate. The supervisor Process Provider reference is a frozen constant here, not a cross-family dependency.

## RBAC requirements

Reads follow the zone RBAC rows for the type; the driver opens no shell, mints no launch ticket, and assembles no argument vector.

## Security posture

The session row must name `Provider/shell-terminal`, its pool reference must name a `ShellPool`, and the login shell must be a bounded artifact reference. Refusals are closed codes and carry no secret.

## State and telemetry

Session resumption evidence and the output ring live with the Provider; the driver keeps an in-memory status projection only and emits no telemetry.

## Build and test

```bash
cargo test -p d2b-provider-shell-session
bazel test //packages/d2b-provider-shell-session:all-tests
```

`tests/registration.rs` proves the declaration, the pool/execution/user reads, the supervisor child derivation, and the refused reference shapes.
