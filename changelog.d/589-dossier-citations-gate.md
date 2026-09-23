### Fixed

- Provider dossier corpus dangling citations: mechanical successor-class
  sweep across `docs/specs/providers/*.md` re-pointed every dangling
  citation to the live crate (predominantly the `d2b-provider-system-*` →
  `d2b-provider-process-*`, `d2b-provider-runtime-*` → `d2b-provider-guest-*`,
  `d2b-priv-broker` → `d2b-broker`, and `d2b-provider-volume-*` → successor
  re-points burned into the corpus in the #589 wave). The residual
  judgment-class citations (Destination rows pointing at planned
  post-cutover file-tree destinations and file-tree rows in the
  planned-work ledger) remain legal behind the corpus's own explicit
  historical/planned markers, exactly per the plan's
  "mentions behind an explicit historical marker stay legal" rule.
- The xtask provider-crate-policy gate (`xtask provider-crate-layout`) now
  flags a dossier dangling-citation: a planted citation of a deleted path
  in the dossier corpus fails the gate; a citation of a live path
  (including `packages/d2b-realm-core/...`) passes; reliance on an
  explicitly historical/marker-class row stays legal. The gate globs the
  dossier corpus (`docs/specs/providers/*.md` plus the
  `docs/specs/ADR-046-*.md` sibling dossiers carrying the same stale crate
  paths), matching the exact corpus the sweep corrected.
