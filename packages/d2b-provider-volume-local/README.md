# `d2b-provider-volume-local`

`Provider/volume-local` is the sole writer of the `Volume` ResourceType.

## Provider identity

| Field | Value |
| --- | --- |
| Provider name | `volume-local` |
| Publisher | first-party, `vicondoa/d2b` |
| Version | tracks the workspace version of this crate |
| Trust attestation | first-party admission; exact package digest resolved from the offline Nix artifact catalog |
| Conformance attestation | the hermetic conformance suite under `tests/` |
| ResourceTypes | `Volume` (layout, views, attachment admission) |
| Attachment transport | none; virtiofs attachments are admitted here and served by `volume-virtiofs` |
| Source kinds | `local-path`, `block-image`, `tmpfs` |
| Finalizers | `volume-local/layout` |
| Shared write | not declared |

## Config schema

The Provider root config declares an allowlist of host roots. Each entry
carries an `id` plus the actual root. A Volume references only the `id`
through `spec.source.settings.sourcePolicyId`; the root itself is private
catalog data that never reaches this crate.

| Field | Description | Default |
| --- | --- | --- |
| `sourcePolicies` | Allowlist of `{ id, root }` entries. `root` is private and is never returned to controller code. | empty; a Volume naming an unknown `id` fails closed |

## Exported resource types

| ResourceType | Role |
| --- | --- |
| `Volume` | sole writer: layout, views, store-view mode, TPM state mode, attachment admission |

## Controllers / services / workers / binaries

| Component | Type | Role |
| --- | --- | --- |
| `volume-local` controller | controller | reconciles `Volume` layout, views, and attachment admission |

The controller performs no privileged mutation. It calls two injected
typed ports and nothing else:

- `VolumeSourceEffectPort` resolves the opaque source policy ID against
  the private allowlist and returns a non-clonable `VolumeRootHandle`.
  The resolved path never reaches controller code.
- `VolumeLayoutEffectPort` observes, provisions, repairs, re-applies
  ACLs for, and removes exactly one declared entry at a time.

ProviderSupervisor alone maps a port call onto a broker operation, and
the broker remains the sole privileged executor and audit owner.

No service, worker template, or standalone binary is declared.

## Placement and dependencies

The controller is Host-placed: every effect it requests resolves against a
host filesystem root. It declares no synchronous Provider dependency. The
`volume-virtiofs` Provider watches `Volume` read-only to serve an export;
that direction is one-way and this crate does not depend on it.

## RBAC requirements

The Provider requires a pre-installed Role granting write on `Volume` and
read on the resources it admits attachments against, bound to the Provider's
own service identity. It requires no wildcard permission and no cross-Zone
grant.

## Security posture

- A Volume source is an opaque policy ID, never a raw host path.
- Layout paths are anchored inside the Volume; a leading separator,
  a `..` component, a backslash, and a NUL byte are all rejected by the
  base contract before this crate sees the entry.
- `noFollow` is honoured fail-closed: a symlink met on a `noFollow` walk
  aborts the entry and requests no mutation.
- Ambiguity quarantines. An entry whose live owner cannot be proven is
  held and reported; it is never deleted, recreated, or reused.
- A `create-if-never-provisioned` entry that is absent after its
  provisioning marker exists fails closed. Guest TPM state is never
  silently re-provisioned.
- Store-view mode serves the guest the closure-only hardlink farm at
  `live/` only, read-only, and never the host store. `gcroots/` and
  `state/` are host-only and sit at the store-view root.
- The controller holds no capability, opens no socket, and spawns no
  process.

## State and telemetry

Public status names an entry only by digest. No host path, source policy
ID, ACL value, numeric UID or GID, or socket path is public. Audit and
telemetry carry the same redaction: an entry is identified by digest and an
outcome by a closed reason token, never by a path or a resolved root.

## Layout

| Path | Contents |
| --- | --- |
| `src/` | controller, layout engine, views, store-view mode, TPM state mode, effect ports, colocated unit tests |
| `tests/` | hermetic layout, view, sharing, store-view, TPM, and status-redaction conformance |
| `integration/` | heavier Host-path and store-view filesystem fixtures |

## Build and test

```bash
cd packages && cargo test -p d2b-provider-volume-local
cd packages && cargo clippy -p d2b-provider-volume-local --all-targets
```
