# Toolkit source contract

The client and provider toolkit distributions package canonical source from a
d2b release. They are distribution and integration repositories, not alternate
owners of d2b protocols.

The machine-readable inventory is
[`toolkit-source-contract.json`](./toolkit-source-contract.json). It names every
in-tree source file, generated binding, public contract artifact, Cargo feature
selection, and distribution fingerprint covered by this contract.

## Canonical source

`d2b-contracts` is the sole owner of serialized identities, session records,
provider records, service messages, and generated protobuf/ttrpc bindings.
`d2b-session` owns the portable authenticated session runtime.
`d2b-session-unix` owns Linux socket, peer-identity, and descriptor handling.
`d2b-client` owns target resolution, typed clients, retries, cancellation,
attachments, and named streams. `d2b-provider` and
`d2b-provider-toolkit` own provider runtime traits, registration, serving,
redaction, fixtures, and conformance.

A distribution must use path dependencies into one immutable d2b release source
artifact. It must not copy a `.proto` file, generated binding, request or
response DTO, session preface, frame codec, handshake, resolver, or provider
record. The canonical SDK crates remain `publish = false`; crates.io is not a
fallback.

The client distribution pins the content-frozen source revision
`9dc902243cdd7aba7ef269988b96f0aae6e037da`. Coordination records and contract
gates use that full immutable revision.

Source ownership is package-complete rather than module-selected. For each
canonical package, the inventory includes every non-ignored file reported by
Git below the package root. This deliberately covers the complete `src/` tree,
the manifest and any `build.rs`, bins, examples, tests, fixtures, protobuf
inputs, and feature-gated modules. Cargo metadata independently supplies every
package target source, and the policy test requires each target to appear in
the Git-derived list. A feature gate cannot make an unlisted compilation input
acceptable.

Each distribution owns its own lockfile and packaging metadata. It does not
copy or modify the d2b workspace lockfile. The d2b workspace manifest remains
part of the fingerprinted source context because canonical crate manifests
inherit workspace package and dependency metadata.

## Fingerprints

Every inventory file entry carries the SHA-256 digest of its bytes. Source-group
and distribution fingerprints use this canonical encoding:

1. initialize SHA-256 with the declared domain, a zero byte, the group or
   distribution ID, and another zero byte;
2. sort unique repository-relative UTF-8 paths lexicographically;
3. for each path, append its byte length as an unsigned 64-bit big-endian
   integer, the path bytes, the file length in the same encoding, and the file
   bytes;
4. render the digest as lowercase hexadecimal.

The distribution fingerprint covers the sorted union of its source groups.
Changing, adding, removing, or renaming an owned source file requires an
intentional inventory update. The inventory does not fingerprint itself.

Regenerate the inventory and coordination fingerprints from the repository
root with:

```text
cd packages
D2B_UPDATE_TOOLKIT_SOURCE_INVENTORY=1 \
  cargo test --locked -p d2b-contract-tests \
  --test policy_toolkit_sources regenerate_toolkit_source_inventory \
  -- --ignored --exact
```

The normal focused test runs the same Cargo-metadata and Git enumeration
without writing files and fails on any omission or stale digest.

## Distribution profiles

### Client toolkit

The client distribution contains the canonical client, session, Unix-session,
and complete contracts-package source groups plus their public contract
artifacts. Its portable profile selects no default features. Its Linux
local-session profile selects `d2b-client/host-socket`, which in turn selects
the Unix session substrate.

The distribution may retain presentation-only Wayland color and Waybar helpers.
It must remove the old client crate, public JSON framing, hello negotiation,
shell/workload DTOs, and duplicated error envelopes. It must also leave the
canonical `d2b-wayland-proxy` package and binary name to the d2b repository.

### Provider toolkit

The provider distribution contains the canonical provider runtime, provider
toolkit, session, and complete contracts-package source groups plus their
public contract artifacts. It may add templates, fake-SDK examples, conformance
entrypoints, provider-author documentation, and Nix packaging. Those additions
consume the canonical crates; they cannot redefine their DTOs or provider-agent
protocol.

## Redaction boundary

Provider-facing generic wrappers are
`d2b_provider_toolkit::{Redacted, Secret}`. Client errors and session, target,
attachment, identity, and provider values use type-specific canonical
`Debug`/`Display` redaction. A sibling may define presentation models, but a
wire-serializable redaction wrapper or opaque-handle DTO is a protocol copy and
is forbidden.

The canonical client does not currently expose a general-purpose
wire-serializable redaction wrapper. Consumers must not preserve the old
toolkit wrapper by copying it. If a new shared wrapper is required, it belongs
in the canonical d2b source before a sibling consumes it.

## Migration ownership

| Owner | Responsibility |
| --- | --- |
| d2b | Canonical SDK crates, contract features, generated bindings, public schemas and vectors, and this exact inventory |
| Client toolkit distribution | Immutable source pin, client re-export/package outputs, presentation-only helpers, release automation, and client documentation |
| Provider toolkit distribution | Immutable source pin, templates, examples, conformance commands, Nix integration, release automation, and provider-author documentation |
| Desktop and terminal consumers | Repository-local presentation models, UI, source pin, adapter composition, tests, lockfile, and migration documentation |

The content-frozen control and user/desktop service APIs are included in the
client distribution through the canonical crates. The canonical client accepts
an exact route, authenticated endpoint policy, credentials, and owned transport;
live endpoint and credential acquisition remain integration behavior and must
not be guessed by a distribution. Persistent-shell, notification, desktop
action, and Wayland helpers likewise remain outside the distribution until the
integrated runtime behavior is available.

The audited repository ownership and dependency split is recorded with ADR 0045
in
[`0045-toolkit-sibling-coordination.json`](../adr/0045-toolkit-sibling-coordination.json).
