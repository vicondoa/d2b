# Public workload manifest compatibility schema

The public manifest remains at:

```text
/run/current-system/sw/share/d2b/vms.json
```

Its frozen compatibility version is `manifestVersion = 7`. The machine-readable
source of truth is [`manifest-schema.json`](./manifest-schema.json).

## Realm-native projection

The two reserved keys remain `_manifest` and `_observability`. Every other key
is populated with a canonical workload ID — the 20-character
`^[a-z2-7]{20}$` short ID that `nixos-modules/v2-identity.nix` derives for
every workload — and maps to the frozen version-7 entry shape. The `name`
field equals that key. The frozen v7 schema's `patternProperties` key regex
(`^[a-z][a-z0-9-]*$`, see [`manifest-schema.json`](./manifest-schema.json)) is
the broader compatibility pattern the locked schema accepts; it is not
retightened to the narrower canonical short-ID shape because doing so would
be a breaking change to a version-locked contract.

The compatibility entry projects realm-native data:

- `env` carries the canonical realm ID;
- `stateDir` uses `/var/lib/d2b/r/<realm-id>/w/<workload-id>`;
- network fields come from the realm network resource rows;
- runtime/provider fields come from the selected runtime binding; and
- component booleans come from normalized role rows.

Human realm and workload names are not authority inputs. Canonical targets and
presentation labels are exposed separately through
`realm-workloads-launcher-v2.json`.

## Reserved metadata

```json
{
  "_manifest": { "manifestVersion": 7 },
  "_observability": {
    "enabled": false,
    "vmName": "sys-obs",
    "obsVsockCid": 1000,
    "obsVsockHostSocket": "/var/lib/d2b/r/local-root/observability/vsock.sock",
    "signozUrl": "http://127.0.0.1:8080",
    "signozOtlpGrpcPort": 4317,
    "signozOtlpHttpPort": 4318
  }
}
```

The version is intentionally unchanged because the Rust parser and public CLI
contract are frozen. New realm-native authority lives in the integrity-pinned
private bundle; this manifest is a bounded compatibility projection only.

## Compatibility rules

- Additive optional fields require synchronized schema, emitter, and consumer
  updates.
- Removing, renaming, or narrowing a field requires a manifest-version bump.
- Unknown reserved keys and unknown fields fail closed.
- Configured argv, credentials, host command output, and private bundle paths
  never appear here.

The private bundle contract and visibility classifications are documented in
[`manifest-bundle.md`](./manifest-bundle.md).
