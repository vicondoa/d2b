# Guest configured-launch catalog

`GuestConfiguredLaunchesV1` is the canonical broker-to-guest codec for one
workload's configured launch items. The broker selects entries only from the
integrity-verified private bundle and filters them to the exact bound realm and
workload before encoding. Host `unsafe-local` items and entries belonging to
another workload are not representable in the catalog.

The codec is binary-only. It has no serde or JSON schema representation, and
Debug output for the catalog, entries, encoded bytes, and errors is redacted.
All integers are unsigned big-endian values.

## Header

The fixed header is 96 bytes:

| Offset | Size | Value |
| ---: | ---: | --- |
| 0 | 8 | `D2BCLV2\0` magic |
| 8 | 2 | schema version `1` |
| 10 | 2 | codec version `1` |
| 12 | 2 | catalog flags, zero |
| 14 | 2 | reserved, zero |
| 16 | 4 | total encoded byte length |
| 20 | 20 | canonical realm short ID |
| 40 | 20 | canonical workload short ID |
| 60 | 32 | nonzero integrity digest of the bound workload definition |
| 92 | 2 | configured-item count, `1..=64` |
| 94 | 2 | reserved, zero |

The complete catalog is capped at 2 MiB. The SHA-256 helper hashes the exact
encoded bytes, including the header and every length field.

## Entry

Each entry starts with a four-byte length covering the remainder of that entry,
followed by:

| Size | Value |
| ---: | --- |
| 2 | configured-item ID byte length |
| variable | bounded configured-item ID |
| 2 | flags; bit 0 is `graphical`, all other bits are rejected |
| 2 | argument count, `1..=128` |
| 2 | reserved, zero |
| repeated | two-byte argument length followed by UTF-8 argument bytes |

Configured-item IDs use the existing bounded 64-byte `ProtocolToken` grammar.
Argument count, per-argument length, and aggregate argument bytes use the canonical
`ConfiguredArgv` limits: 128 arguments, 4096 bytes per argument, and 16 KiB
total. The program argument must not be empty. No path, environment, working
directory, host identity, or per-entry workload selector is present.

Decode rejects duplicate item IDs, invalid UTF-8, NUL bytes, empty identifiers,
an empty program or argument vector, unknown flags or versions, nonzero
reserved fields, inconsistent lengths, truncation, trailing bytes, zero
workload digests, and every count or byte-limit violation. The guest resolves
only IDs from the decoded catalog.

`GuestConfiguredLaunchesBytes` exposes bounded borrowed bytes and a write
method, has redacted Debug, is not cloneable or serializable, and wipes its
backing allocation on release.
