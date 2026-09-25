### Fixed

- Align the activation-runner wire token with the frozen role vocabulary: the
  `RunnerRole::ActivationNixos` frame value is now `activation-nixos-runner`,
  matching the role id, the bundle schema, and the provider template; the
  pre-rename spelling remains accepted on the read side.
- Report the root caller role as `d2b-root` in the broker's audit label and in
  the bootstrap caller-role mirror, matching every sibling `d2b-*` label.
- Remove the unreachable `ZoneAdmissionError::ZoneInvalid` and
  `SystemCoreError::BudgetOvercommit` variants.
- Build the volume `has-layout` effect response from its value instead of a
  parse path whose failure branch could not be reached.
- Share one metrics request validator between the two daemon metrics handlers
  so their method and path refusals cannot drift.
