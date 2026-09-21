### Fixed

- CI retries the Bazel fetch and analysis phase when a third-party release
  download returns a transient server error, so an artifact host having a bad
  minute no longer fails a pull request before a single test runs. A missing
  artifact, a hash mismatch, an analysis error or a test failure still fails on
  the first attempt.
