# `bundle.json` schema (`v2`)

Schema: [`bundle.json`](./bundle.json)

`bundle.json` is the private bundle index installed beside the public
`vms.json` manifest. It gives the daemon and broker one stable place to
find the current `host.json`, `processes.json`, `privileges.json`,
`closures/*.json`, `storage.json`, `sync.json`, `allocator.json`,
`realm-controllers.json`, `realm-identity.json`, and
`realm-workloads-launcher-v2.json`, `unsafe-local-workloads.json`, and
`minijail-profile.json` artifacts.

## Top-level fields

- `schemaVersion` — schema directory/version for every referenced artifact.
- `bundleVersion` — additive bundle contract rev (`10` in the current tree).
- `publicManifestPath` — path to the public `vms.json` manifest.
- `hostPath` — path to the private `host.json` artifact.
- `processesPath` — path to the private `processes.json` artifact.
- `storagePath` — path to the private `storage.json` artifact.
- `syncPath` — path to the private `sync.json` artifact.
- `allocatorPath` — path to the private `allocator.json` artifact.
- `realmControllersPath` — path to the private `realm-controllers.json`
  artifact.
- `realmIdentityPath` — path to the private `realm-identity.json` artifact.
- `realmWorkloadsLauncherV2Path` — path to the argv-free launcher metadata
  served through the authorized public daemon API.
- `unsafeLocalWorkloadsPath` — optional path to the private configured
  unsafe-local workload artifact.
- `privilegesPath` — path to the private `privileges.json` artifact.
- `closures` — per-VM closure artifact paths.
- `minijailProfiles` — shipped minijail profile metadata paths.
- `generation` — source/build provenance for drift auditing.

## Contract notes

- The bundle is private host-side state; callers consume `vms.json`
  publicly and dereference the rest through this index.
- `bundleVersion` can advance without changing `schemaVersion` when the
  change is additive.
