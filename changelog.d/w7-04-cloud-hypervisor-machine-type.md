### Changed

- `runtime-cloud-hypervisor` Provider root configuration types
  `defaultMachineType` as a closed `q35` | `microvm` value instead of a
  free-form opaque identifier string. A root configuration naming any
  other machine type is refused when the config is decoded, and the
  artifact's published `config-schema.json` advertises the same closed
  value set, so a bad machine type fails at parse time rather than as an
  opaque `cloud-hypervisor-config-invalid` after installation.
