# `allocator.json` schema (`v2`)

Schema: [`allocator.json`](./allocator.json)

`allocator.json` is private, metadata-only local-root allocator configuration.
It is rooted in the normalized `d2b.realms` index and does not start a service,
bind a socket, mutate host state, or change current `d2b.envs` behavior.

## Top-level fields

- `schemaVersion` — schema version for this artifact.
- `allocator` — future allocator root socket/state/audit paths plus explicit
  `metadata-only` runtime state.
- `realms` — enabled realm rows selected from `d2b._index.realms`.
- `resourceRequests` — schema-friendly host-resource request metadata for
  path/socket partitions, network namespaces, shared env bridges, and optional
  host-mutation partitions.
- `pathPartitions` — per-realm state/run/audit/public-socket/broker-socket
  paths.
- `providerPlacements` — provider placement metadata copied from enabled realm
  provider rows. Optional provider `kind` values are bounded lowercase slugs.
- `envBridge` — transitional mapping from realm paths to existing env bridges and
  net VMs. `mode` is the closed realm network mode enum:
  `none`, `inherit-env`, `declared`, or `external`.
- `processLaunch` — at most 64 strictly sorted, unique paired child-controller
  and child-broker launch-authority rows. The key is
  `(realmId, controllerGeneration)`.
- `invariants` — booleans asserting this artifact is private metadata only and
  preserves the existing env runtime source of truth.

## Process launch authority

Each `processLaunch` row has this exact shape:

```json
{
  "realmId": "work",
  "realmPath": "work",
  "controllerGeneration": "generation-1",
  "launchRecordDigest": "sha256:3027627c860ce8979511d9a58a75fb250762ca05f109524c28ec7b69f5eac31a",
  "controller": {
    "role": "controller",
    "processId": "controller-process-1",
    "executableRef": "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-d2bd/bin/d2bd",
    "executableDigest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
    "configRef": "controller-config-v2",
    "configDigest": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
    "uid": 61001,
    "gid": 61001,
    "listenerRef": "controller-listener",
    "bootstrapSessionRef": "controller-bootstrap",
    "cgroupRef": "controller-cgroup",
    "cgroupDigest": "sha256:3333333333333333333333333333333333333333333333333333333333333333",
    "stateRootRef": "controller-state-root",
    "auditRootRef": "controller-audit-root",
    "namespaces": {
      "user": { "refId": "controller-user-ns", "digest": "sha256:4444444444444444444444444444444444444444444444444444444444444444" },
      "mount": { "refId": "controller-mount-ns", "digest": "sha256:4444444444444444444444444444444444444444444444444444444444444444" },
      "network": { "refId": "controller-network-ns", "digest": "sha256:4444444444444444444444444444444444444444444444444444444444444444" },
      "ipc": { "refId": "controller-ipc-ns", "digest": "sha256:4444444444444444444444444444444444444444444444444444444444444444" },
      "pid": { "refId": "controller-pid-ns", "digest": "sha256:4444444444444444444444444444444444444444444444444444444444444444" },
      "cgroup": { "refId": "controller-cgroup-ns", "digest": "sha256:4444444444444444444444444444444444444444444444444444444444444444" }
    },
    "resourceRefs": ["controller-resource-a"],
    "leaseRefs": ["controller-lease-a"],
    "spawn": {
      "clone3WithPidfd": true,
      "directCgroupPlacement": true,
      "noNewPrivileges": true,
      "emptyInitialCapabilities": true,
      "executableOnlyArgv": true,
      "closedEnvironment": true,
      "inheritedFdAuthorityOnly": true
    }
  },
  "broker": {
    "role": "broker",
    "processId": "broker-process-1",
    "executableRef": "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-d2b-priv-broker/bin/d2b-priv-broker",
    "executableDigest": "sha256:5555555555555555555555555555555555555555555555555555555555555555",
    "configRef": "broker-config-v2",
    "configDigest": "sha256:6666666666666666666666666666666666666666666666666666666666666666",
    "uid": 61002,
    "gid": 61002,
    "listenerRef": "broker-listener",
    "bootstrapSessionRef": "broker-bootstrap",
    "cgroupRef": "broker-cgroup",
    "cgroupDigest": "sha256:7777777777777777777777777777777777777777777777777777777777777777",
    "stateRootRef": "broker-state-root",
    "auditRootRef": "broker-audit-root",
    "namespaces": {
      "user": { "refId": "broker-user-ns", "digest": "sha256:8888888888888888888888888888888888888888888888888888888888888888" },
      "mount": { "refId": "broker-mount-ns", "digest": "sha256:8888888888888888888888888888888888888888888888888888888888888888" },
      "network": { "refId": "broker-network-ns", "digest": "sha256:8888888888888888888888888888888888888888888888888888888888888888" },
      "ipc": { "refId": "broker-ipc-ns", "digest": "sha256:8888888888888888888888888888888888888888888888888888888888888888" },
      "pid": { "refId": "broker-pid-ns", "digest": "sha256:8888888888888888888888888888888888888888888888888888888888888888" },
      "cgroup": { "refId": "broker-cgroup-ns", "digest": "sha256:8888888888888888888888888888888888888888888888888888888888888888" }
    },
    "resourceRefs": ["broker-resource-a"],
    "leaseRefs": ["broker-lease-a"],
    "spawn": {
      "clone3WithPidfd": true,
      "directCgroupPlacement": true,
      "noNewPrivileges": true,
      "emptyInitialCapabilities": true,
      "executableOnlyArgv": true,
      "closedEnvironment": true,
      "inheritedFdAuthorityOnly": true
    }
  }
}
```

Unknown or missing fields fail decoding. Opaque references are 1–128 safe
token bytes and reject path- and credential-shaped values. `executableRef` is
the only path-bearing field and is restricted to `/nix/store/...` or
`/run/current-system/sw/bin/...`; no argv, environment, credential, UID map, or
ambient host path is represented. `resourceRefs` and `leaseRefs` are each
strictly sorted, unique, and bounded to 32 entries.

`launchRecordDigest` is SHA-256 over canonical compact JSON containing exactly
`realmId`, `realmPath`, `controllerGeneration`, `controller`, and `broker`
(lexicographically sorted object keys), encoded as `sha256:<hex64>`. Decode
recomputes it. Controller and broker roles must be canonical, process IDs and
nonzero UIDs must be distinct, every spawn boolean must be true, and the realm
ID/path must identify exactly one enabled `host-local` realm row.

## Contract notes

- `runtimeState` is `metadata-only`; there is no allocator service/socket unit in
  this scope.
- Resource identifiers are opaque metadata. They are not host paths, interface
  names, nftables object names, file descriptors, or credentials.
- Every `realmPath` is a bounded DNS-style realm path: 1-16 lowercase
  dot-separated labels, maximum 255 bytes.
- Existing `d2b.envs` and `d2b.vms.<vm>.env` remain the runtime source of truth
  until a later allocator implementation lands.
- Bundle resolution opens `allocator.json` under the private-artifact ownership
  policy, verifies its `artifactHashes` entry, validates the complete typed
  document, and then permits exact `(realmId, controllerGeneration)` lookup.
  Missing or malformed records have no ambient fallback.
