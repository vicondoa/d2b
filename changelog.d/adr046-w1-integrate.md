### Changed

- The panel review process now ranks the three review surfaces explicitly:
  the binding ten-role wave panel, the per-round phase panel, and the
  in-flight five-seat council. The binding panel stays the only authority
  for sealing a wave, and a passing in-flight council cannot substitute for
  it.
- Contributor agent tooling is configured in-repo under `.opencode/`. The
  reviewing roles are pinned to the model the binding panel requires, so a
  silent model fallback can no longer produce a panel record that attestation
  would reject, and irreversible operations stay behind an explicit
  confirmation.
