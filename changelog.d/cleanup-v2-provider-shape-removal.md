### Removed

- `BundleResolver` now accepts only the v3 Zone-native bundle contract
  (`schemaVersion:"v3"`, `bundleVersion:1`, `zones[]`). Loading any other
  schema version (`v2` and below) fails closed with
  `manifest-version-mismatch` instead of parsing the legacy v2 shape.
- Removed the legacy v2 `Bundle` load path and its surfaces: the
  `load_with_paths` + `from_artifacts*` constructors, the per-v2-artifact
  readers (closures, minijail-profiles, sync, allocator, realm-controllers,
  realm-identity, unsafe-local-workloads), and the v2 `Bundle` struct fields
  (`publicManifestPath`, `hostPath`, `processesPath`,
  `realmControllersPath`, `realmIdentityPath`, `unsafeLocalWorkloadsPath`,
  `syncPath`, `allocatorPath`, `closures`, `minijailProfiles`,
  `managedKeys`) plus their helper types.
- Removed the legacy host-runtime Provider catalog from `host.json`
  (`runtimeProviders` / `vmRuntimes` / `VmRuntimeRow`) and the
  host.json-driven VM-to-Zone binding loop.
- Removed the v2-only broker operations and their wire/CLI surfaces:
  `gc`, `migrate`, `host install`, `keys rotate`, `trust`,
  `rotate-known-host`, `keys list`/`keys show`, `store verify`, legacy
  legacy-swTPM migration, `setup-mount-namespace`, and
  bind-from-hardlink-farm, together with the d2b CLI `host check` verb and
  the CLI bundle-context loader. Host-prepare, console, qemu-media,
  host-prep DAG, cgroup delegation, and audio surfaces now read VM runtime
  metadata from the Zone-native Guest resources instead of the v2 manifest.
