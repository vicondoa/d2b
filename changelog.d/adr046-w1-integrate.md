### Changed

- The panel review process now ranks the three review surfaces explicitly:
  the binding ten-role wave panel, the per-round phase panel, and the
  five-seat council. Work driven by an agent harness satisfies the per-round
  gate with the five-seat council instead of a full roster round, which
  matches the existing rule that the binding panel runs once at wave close
  and never per implementation round. The binding panel is unchanged and
  remains the only authority for sealing a wave.
- Contributor agent tooling is configured in-repo under `.opencode/`. The
  reviewing roles are pinned to the model the binding panel requires, so a
  silent model fallback can no longer produce a panel record that attestation
  would reject, and irreversible operations stay behind an explicit
  confirmation.
