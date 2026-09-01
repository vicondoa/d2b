### Fixed

- Routed Guest daemon and broker package selection through the injected
  `d2bHostToolOverrides` map so host-integration VM evaluations do not fall
  back to Nix source packages for those d2b binaries.
- Staged the Cloud Hypervisor Provider controller from Bazel separately from
  the eight host tools so VM acceptance no longer recompiles it through Nix.
