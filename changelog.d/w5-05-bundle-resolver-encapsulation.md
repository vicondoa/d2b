### Fixed

- BundleResolver now encapsulates its trusted bundle artifacts: the bundle, host, processes, storage, site, realm-workloads-launcher-v2, and manifest fields are private, read through typed accessors, and storage replacement goes through a single set_storage setter. All consumers ind2bd, d2bd-runtime, d2b-broker, and the provider crates now read through the accessors.
- The broker's storage-contract reconciliation now writes the resolved contract back via set_storage instead of mutating the resolver's storage field directly.