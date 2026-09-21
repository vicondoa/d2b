### Changed

- Provider-owned family facts are now declared in the owning crate and
  generated into their shared consumers (U4). The process family declares its
  `ProcessRole` vocabulary in `packages/d2b-provider-process/resource-types.json`
  - each role with its owning Provider reference, its description, and the
  authority-bearing facts it carries - and the per-crate extension of the
  existing `resource-types.json` declaration is what the consumers read: the
  `ProcessRole` enum in `d2b-core` and the Nix `process-role-providers.nix`
  map that `resources-zones-processes.nix` folds are now generated from it,
  byte-pinned by the same drift gate as the other declaration artifacts.
- The declaration-to-descriptor parity gate now also covers the declared role
  vocabulary (a declared role the crate's sources do not spell fails naming
  both, and vice versa), and the committed per-crate authority bound covers
  both the authority-bearing role facts (operations, principals, storage
  roots, seccomp classes, device classes, capability grants) and the declared
  service facets a method carries (required privileges, state cells,
  descriptor-leg types and rights), so widening any of them fails as a gated
  change naming the widened fact.
