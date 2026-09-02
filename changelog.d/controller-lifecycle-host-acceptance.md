### Changed

- Replace fake Cloud Hypervisor host acceptance with a Zone-native,
  controller-owned Guest boot, signed Provider artifacts, closure-backed store
  views, restart adoption, and deletion proofs.

### Removed

- Remove the duplicate legacy VM restart acceptance fixture.
- Retire the obsolete realm-owned unsafe-local host fixture; current host
  admission is covered by the Zone resource and package-local tests.
