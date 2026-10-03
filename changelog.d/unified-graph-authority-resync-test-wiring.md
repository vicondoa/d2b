### Fixed

- The broker resynchronization acceptance test now actually runs. It was
  committed and documented as a real check on the restart path - a broker
  restart fences every known Zone until the manager restates the projection it
  already accepted, and a daemon that cannot prove its projection is refused by
  name - but it had no Bazel target, so the only scheduler in this repository
  never selected it and no gate had ever executed it. The file sat in the tree
  reading as coverage while a whole class of restart behaviour went unverified.
- Added the missing `d2b_rust_test` target so the file is part of the daemon's
  aggregate test suite alongside the other acceptance checks, and every future
  change to the resynchronization path has to keep passing it.