### Added

- Provider artifacts are now declared in Nix under `d2b.artifacts.<id>`, giving
  each Provider package a derivation plus its catalog metadata. A Provider
  resource selects one with `artifactId = "<id>"`. Nix compiles those
  declarations into an offline catalog that is sorted by identifier and
  selected by exact digest: there is no runtime marketplace, no download, no
  PATH or directory discovery, no `latest`, and no version-range solving. An
  artifact that was not declared does not exist. Malformed identifiers, missing
  or unknown catalog fields, and inexact digests are rejected at evaluation
  time with a message naming the field. The catalog's public projection carries
  no Nix store path.
- A Provider crate policy now enforces the packaging conventions across the
  workspace: every Provider crate carries `src/`, `tests/`, `integration/` and a
  `README.md` with the nine required sections, each `integration/*.rs` file
  declares exactly one orchestration target, one crate is exactly one Provider
  identity, and a Provider crate depends only on the public contract, the
  toolkits and the SDK rather than on the daemon, the broker, the store, or
  another Provider's internals. Two pre-existing crates scheduled for
  replacement are exempt with a recorded reason.
- A new build-level check proves the Provider catalog emitter is deterministic:
  two independent evaluations of the same declarations, constructed
  differently, must produce byte-identical output, and a negative control must
  produce different output so the comparison cannot pass vacuously.

### Changed

- The `d2b-provider-volume-local` and `d2b-provider-volume-virtiofs` READMEs are
  restructured onto the nine standard Provider documentation sections, so every
  Provider is documented in the same shape. All previous content is retained.
