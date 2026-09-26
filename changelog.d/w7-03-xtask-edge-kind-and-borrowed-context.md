### Changed

- The package-policy generator carries a Cargo dependency kind as a
  closed enum instead of a free string: the locked-metadata boundary
  parses the kind once, the production and policy kind sets are enum
  sets, and the emitted spelling (including `proc-macro`) is
  byte-identical, so no checked-in closure changes. A kind outside the
  vocabulary is now reported instead of silently dropping the edge, and
  the emitted edge order still follows the wire spelling.
- Generating the package-policy inputs borrows the context spec it is
  computing instead of cloning it, so a run no longer copies each spec
  once per mode.
