### Removed

- Removed the pre-Zone Nix env, realm, VM, and gateway hierarchy, including
  legacy lifecycle emitters and per-realm service, socket, user, and state
  declarations.

### Changed

- Zone resource bundles and Provider artifacts are now the only active Nix
  lifecycle inputs; root daemon/broker admission, required OS groups, Network
  gateway fields, host-tool plumbing, and v2 launcher metadata remain.
