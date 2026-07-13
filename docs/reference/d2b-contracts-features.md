# `d2b-contracts` feature matrix

`d2b-contracts` has no default API surface. Every consumer must disable default
features through the workspace dependency and select only the contract domains
it imports. Production consumers must not use the `schema` union.

## Current contract families

| Feature | Exposed modules and root API | Includes |
| --- | --- | --- |
| `common` | Current framing, handshake, socket constants, and version-negotiation root API | — |
| `guest-auth` | `guest_auth` | — |
| `usbip` | `usbip` | — |
| `security-key` | `security_key` | — |
| `guest` | `generated`, `guest_proto`, `guest_wire` | `common`, `guest-auth`, `usbip` |
| `broker` | `broker_wire`, `types` | `common`, `guest-auth`, `security-key`, `usbip` |
| `public` | `public_wire`, `terminal_wire` | `broker`, `guest` |
| `cli-output` | `cli_output` | `public` |
| `unsafe-local` | `unsafe_local_wire` | `public` |
| `schema` | Maintained schema-generation and contract-test union | `cli-output`, `unsafe-local` |

The `public` family contains the current terminal module because those modules
refer to each other's current DTOs. Keeping them in one acyclic family makes
that coupling explicit without inventing aliases. A future contract can split
them only after removing the source dependency.

`protobuf` is optional and activated only by `guest`. The generated
guest-control message bindings remain checked in under `generated`; normal
builds do not run protobuf or ttRPC code generation. `schemars`, `serde`, and
`serde_json` are optional and activated only by families whose current DTOs use
them.

## Empty d2b 2.0 ownership rails

| Feature | Module | Owner |
| --- | --- | --- |
| `v2-identity` | `v2_identity` | Identity contracts |
| `v2-component-session` | `v2_component_session` | Component-session contracts |
| `v2-services` | `v2_services` | Service contracts |
| `v2-provider` | `v2_provider` | Provider contracts |
| `v2-state` | `v2_state` | State and audit contracts |

Each rail is independent and intentionally empty. It must not alias or
re-export a current contract. The owning implementation can add DTOs in its
dedicated module without editing central feature wiring.

## Maintained consumers

| Consumer | Features |
| --- | --- |
| `d2b-realm-codec-protobuf` | `common` |
| `d2b-host` | `broker` |
| `d2b-guestd`, `d2b-userd` | `guest` |
| `d2b-daemon-access` | `public` |
| `d2b` | `cli-output` |
| `d2b-priv-broker` | `broker`, `guest` |
| `d2bd`, `d2b-unsafe-local-helper` | `unsafe-local` |
| `xtask`, `d2b-contract-tests` | `schema` |

The Rust Layer-1 gate compiles the empty default, each leaf family,
representative composed families, every ownership rail, and the schema union
with `--no-default-features`. Policy tests pin the acyclic feature graph,
optional dependency posture, explicit consumer selections, and empty-rail
rules.
