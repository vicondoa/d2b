# Guest session runtime credential

`GuestSessionCredentialV1` is the private runtime authority used to establish
authenticated `d2b.guest.v2` ComponentSessions. It replaces the long-lived
guest-control token. It is not a manifest field, bundle artifact, Nix
derivation input, or privileged signing operation.

The credential schema version is exactly `1`; `d2b-guest-session-v2` is the
fixed runtime encoding and systemd credential name, not a second schema
version.

## Integration status

This declarative contract is not a standalone cutover. Its `LoadCredential`
and canonical `--workload-id` wiring must land together with the shared runtime
codec, realm-controller encoder, guest decoder, and removal of the live
privileged guest-token signing dispatch. Until then the unit remains
dependency-blocked and privilege parity is expected to report the live signing
row missing from the emitted declarative matrix.

## Payload

Every credential contains exactly these required bindings:

- a nonzero authoritative session generation;
- the parent static public key;
- the exact transport channel binding;
- the enrolled guest identity and static public key.

Bootstrap credentials may additionally contain one operation ID, replay nonce,
absolute expiry, complete operation/runtime/transport binding, and the
single-use PSK secret. The binding and secret are both required when this
optional section is present. A parent private key or guest private key is never
valid credential content.

The guest private key is generated inside the guest and sealed to its vTPM and
persistent guest state. The parent retains only the enrolled guest public key
and bounded enrollment state.

## Runtime authority and lifecycle

For a host-local child realm, `d2bd-r-<realm-id>` owns generation and binding
selection. `d2bbr-r-<realm-id>` may only materialize the supplied bytes at the
declared workload runtime path:

```text
/run/d2b/r/<realm-id>/w/<workload-id>/guest-session/d2b-guest-session-v2
```

The directory is `root:d2b-gctlfs-<workload-id>` mode `0750`; the credential is
mode `0440` with the same owner/group. It is process-scoped, broker-created,
and quarantined rather than adopted when owner, generation, identity, channel,
or runtime evidence is ambiguous.

The gctlfs numeric principal receives execute-only traversal, never directory
listing, on each ancestor that its group mode does not already cover:

| Path | Owner/group | Mode | gctlfs access |
| --- | --- | --- | --- |
| `/run/d2b` | `root:d2b` | `1770` | exact named `x` ACL |
| `/run/d2b/r` | `root:d2bd` | `0710` | exact named `x` ACL |
| `/run/d2b/r/<realm-id>` | `root:d2bcg-r-<realm-id>` | `0750` | exact named `x` ACL |
| `.../w` | realm controller/internal group | `0750` | exact named `x` ACL |
| `.../w/<workload-id>` | realm controller/internal group | `0750` | exact named `x` ACL |
| `.../guest-session` | `root:<gctlfs-gid>` | `0750` | group `r-x` |
| `.../d2b-guest-session-v2` | `root:<gctlfs-gid>` | `0440` | group read |

The grants name only that workload's stable gctlfs GID. The fixed-root grant is
an exact tmpfiles declaration and descendant grants are storage rows; neither
uses recursive chmod nor a catch-all ACL repair.

The controller transfers the encoded credential to its realm broker only as an
exact-storage-reference-bound, sealed, close-on-exec memory-file descriptor over
their authenticated ComponentSession. The request is
`d2b.broker.v2/BrokerService/Apply` (stable method ID `2253834528`) with the
credential storage ID as `resource_id` and exactly one read-only
`request-input` memfd attachment. A path or byte payload is not accepted.

The controller rotates the credential before publishing a route after a
controller/workload generation change, runtime or transport replacement, or
guest re-enrollment. Stale or duplicate evidence fails closed. A bootstrap PSK
is withdrawn after its single use; normal enrolled sessions carry no PSK.

## Guest delivery

The dedicated read-only virtiofs share exposes only the workload runtime
credential directory at `/run/d2b-guest-control-host`. The guest service uses
the fixed systemd credential mapping:

```text
d2b-guest-session-v2:/run/d2b-guest-control-host/d2b-guest-session-v2
```

Systemd presents `d2b-guest-session-v2` to `d2b-guestd` as a root-owned mode
`0400` file in `CREDENTIALS_DIRECTORY`. Missing credentials, unknown encoding,
zero or mismatched bindings, stale generations, and replayed or expired PSKs
are fatal. There is no environment, command-line, token-file, host-global
broker, or unauthenticated fallback.

The runtime encoder and guest decoder must agree on the
`d2b-guest-session-v2` encoding for `GuestSessionCredentialV1`, including all
required guest identity/public-key fields and the all-or-none optional PSK
section.
