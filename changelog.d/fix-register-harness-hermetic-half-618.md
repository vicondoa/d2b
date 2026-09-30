### Fixed

- The VM host-integration harness now builds, lints, and runs its unit tests in
  the Layer-1 gate. `layer1` referenced no target under `bazel/checks/vm`, so the
  harness's roughly 22k lines were never compiled or linted on a pull request
  and four separate lane defects reached `main` behind a fully green gate. The
  KVM-backed half of the lane stays out of the gate by decision, and the lane
  suite now records that decision instead of leaving it looking like an
  oversight.
