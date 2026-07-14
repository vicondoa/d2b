# Canonical v2 identities

This reference defines the canonical human names and runtime identifiers used
by the d2b 2.0 control plane. The machine-readable conformance fixture is
[`v2-identity-vectors.json`](./v2-identity-vectors.json).

## Canonical human inputs

Realm labels, workload names, and configured provider instance IDs match
`^[a-z][a-z0-9-]{0,62}$`. They are exact lowercase printable-ASCII values;
there is no Unicode normalization or locale case folding.

A realm path is leaf-to-root and ends in the literal `local-root`:

```text
local-root
dev.local-root
personal-dev.dev.local-root
```

Empty labels, repeated or trailing separators, root-to-leaf paths, Unicode, and
the public target suffix `.d2b` are invalid. Renaming any canonical input
creates a new identity.

`ProviderType` is closed to:

```text
runtime infrastructure transport substrate credential display
network storage device audio observability
```

`RoleKind` is closed to:

```text
store-virtiofs-preflight swtpm-pre-start-flush swtpm virtiofsd video
gpu gpu-render-node audio cloud-hypervisor qemu-media vsock-relay
guest-control-health usbip security-key-frontend wayland-proxy
```

## Length-prefixed grammar

The hash input is exactly:

```text
encoded = "d2b-id-v2;" decimal ":" domain ";" decimal ";"
          *(decimal ":" part ";")
decimal = "0" / (nonzero-digit *digit)
```

The domain-length decimal counts printable-ASCII bytes. The next decimal is
the exact part count, and every part has its own byte length. Domains and parts
are non-empty. Leading-zero decimals, control bytes, NUL, Unicode, missing or
extra fields, and trailing bytes are rejected.

| Runtime ID | Domain | Parts |
| --- | --- | --- |
| `RealmId` | `d2b-v2:realm` | canonical realm path |
| `WorkloadId` | `d2b-v2:workload` | `RealmId`, canonical workload name |
| `ProviderId` | `d2b-v2:provider` | `RealmId`, `ProviderType`, configured provider instance ID |
| `RoleId` | `d2b-v2:role` | `RealmId`, `WorkloadId`, `RoleKind` |

The first 96 SHA-256 bits are encoded most-significant-bit first with the
lowercase unpadded RFC 4648 alphabet:

```text
abcdefghijklmnopqrstuvwxyz234567
```

The result is exactly 20 ASCII bytes. Its final symbol is `a` or `q`, because
the last symbol contains one digest bit followed by four zero padding bits.

## Validation boundary

Rust and Nix independently serialize, hash, and encode the same committed
fixture. Runtime loading recomputes IDs from canonical inputs. Before mutable
resources are opened, complete generated configurations reject duplicate
globally scoped `ProviderId` values and every repeated 20-character short ID.
Collision detection is mandatory; probability is not a fallback.

Errors are fixed, bounded categories and do not include rejected input. The
Rust wire types validate during deserialization and do not accept legacy enum
spellings or aliases.

## Path proof primitive

Linux pathname Unix sockets allow at most 107 pathname bytes before the
terminating NUL. A short ID is 20 bytes, contains no NUL, and leaves 87 bytes
before considering the fixed template. Both implementations expose a generic
pathname headroom check for later generated endpoint tables. This identity
contract does not define endpoint templates.

## Canonical fixture

The JSON fixture contains:

- 33 valid semantic vectors spanning all four domains, every provider type,
  and every initial role;
- two structurally valid partition-boundary vectors proving that `["ab","c"]`
  and `["a","bc"]` serialize and hash differently;
- 24 malformed/noncanonical encoding and semantic cases;
- six malformed short-ID representations; and
- the generic short-ID and Linux pathname constants.

Every valid row records the domain, parts, exact encoded ASCII, encoded bytes
as lowercase hexadecimal, full SHA-256 digest, and exact short ID.
