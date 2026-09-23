### Fixed

- Provider dossiers now cite the current tree: the dossier-citation gate
  (`packages/xtask/src/provider_crate_policy.rs`) validates `Destination` /
  `Reuse path` / file-tree references in the dossier corpus against the tree,
  and the dossier sweep re-pointed every live-crate successor path (including
  `d2b-provider-process-*` / `d2b-provider-guest-*` crate renames and the
  `d2b-priv-broker → d2b-broker` move). Citations behind an explicit
  historical marker (baseline / planned-destination ledgers) stay legal.
