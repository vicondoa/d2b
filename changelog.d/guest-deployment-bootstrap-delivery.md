### Fixed

- A Guest now receives its own Zone's verified deployment graph. The graph is
  built by the same constructor as the Host's publication and differs from it
  only in the Zone it names, and each Guest image carries it in its own
  closure, delivered to the fixed path the Guest target agent reads it from.
  Before this, nothing ever produced that document for a Guest, so
  `d2bd guest` refused to serve on every start.

### Changed

- The verified deployment graph is constructed in one place
  (`nixos-modules/deployment-bootstrap.nix`), so the Host publication and the
  per-Zone Guest publication cannot drift apart in schema, canonical
  encoding, or framed-digest domain.
