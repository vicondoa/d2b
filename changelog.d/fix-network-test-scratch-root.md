### Fixed

- Three broker network realization tests derive their scratch directory from the
  test helper instead of the caller's working directory, so they run in a fresh
  checkout rather than failing by construction, and no longer leak scratch
  directories under a package-relative target path.
