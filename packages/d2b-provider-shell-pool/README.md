# `d2b-provider-shell-pool`

This is the crate root for the `shell-terminal.d2bus.org.ShellPool` ResourceType, one of the six
resource types of the interaction family. It owns the pool's driver, its spec decoder, its spec reference checks, and the driver declaration the v3 resource plane registers the type by; the driver verbs come from the interaction family's shared engine in `d2b-provider-wayland-policy`.

## Provider identity

| Field | Value |
| --- | --- |
| Provider identity | `shell-pool` (resource-type owner) |
| ResourceType | `shell-terminal.d2bus.org.ShellPool` |
| Package | `packages/d2b-provider-shell-pool/` |
| Driver declaration | `shell_pool_descriptor` -> `DriverDescriptor` |
| Registration | `packages/d2b-provider-shell-pool/tests/registration.rs` |

## Config schema

The Provider's reference document: `providerRef`, the execution reference (Host or Guest), the user reference (User), and the bounded `artifact://` login-shell reference. Validate refuses every other shape.

## Exported resource types

`ShellPool` is not exportable (`exportable: false`).

## Controllers / services / workers / binaries

One driver, `ShellPoolDriver`, built by `ShellPoolFactory`. The pool realizes no child resources; its sessions own the supervisor Processes.

## Placement and dependencies

The crate depends on the resource contracts, the runtime contracts, the declaration vocabulary, the shell Provider's interval constant, and the family engine crate.

## RBAC requirements

Reads follow the zone RBAC rows for the type; the driver opens no shell and holds no PTY.

## Security posture

The pool row must name `Provider/shell-terminal`, and the login shell must be a bounded artifact reference: a filesystem path is refused at validate.

## State and telemetry

The output ring and attachment ledger live with the Provider; the driver keeps an in-memory status projection only.

## Build and test

```bash
cargo test -p d2b-provider-shell-pool
bazel test //packages/d2b-provider-shell-pool:all-tests
```

`tests/registration.rs` proves the declaration, the two row reads, and every refused reference shape.
