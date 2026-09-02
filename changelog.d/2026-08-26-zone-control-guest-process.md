### Changed

- Compile Guest and Provider-owned Process resources from canonical Zone-scoped
  Nix inputs and retain Guest system evaluations outside daemon authority.

### Removed

- Removed legacy Guest system lookup aliases in favor of
  `d2b.guestSystems.<zone>.<guest>`.
