### Changed

- The legacy effect-port removal plan, its completion ledger, and the ADR-046
  decision record now state the per-family conversion state at inventory
  close: which families are converted and merged, which are converted and
  awaiting merge on their lane branches, where a port moved while the
  substantive work stayed daemon-side behind a declared runtime facet, and
  where a surface was retired rather than converted. The plan's unit table is
  the inventory, and no unit is counted as landed until its head is in the
  tree.
- The completion ledger and debt register record the residuals and the three
  known-open items: the daemon's Bazel test targets cannot build because
  rules_rs emits two configurations for one crate, the daemon's clippy target
  fails on a clean state for lints in untouched code, and the process-systemd
  provider's identity read cannot complete under systemd 260, whose cause is
  not yet established; the explanation that systemd removed the transient-unit
  main-pid and control-group properties is ruled out.