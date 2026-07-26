### Fixed

- Rejected resource envelopes unless they declare exactly one type discriminator,
  including duplicate keys that structured parsers previously collapsed or ignored.
- Traversed every structural Nix child that can contain a resource envelope,
  including conditions, assertion predicates, lambda defaults, interpolation, and
  inherit sources.
- Replaced the editable process-marker exemption budget with a frozen path-universe
  pin whose active exemptions can only move into the retired set.
