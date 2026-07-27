### Fixed

- Aligned every Provider Process restart-policy example with the integer
  fixed-point multiplier contract, so authored Nix specs no longer contain
  rejected floating-point values.
- Corrected ResourceName and ZoneId documentation to enforce the canonical
  1-to-63-byte bound across resource envelopes, Nix validation, and Provider
  examples.
- Made the drift driver invoke regular gate files through Bash regardless of
  their executable bit, preventing a referenced gate from being silently
  skipped after a mode change.
- Wired the frozen process-marker universe checker into generated-artifact
  drift validation so active and retired marker pins are enforced in Layer 1.
- Made the Layer-1 lint job run its mandatory disk-space preflight regardless
  of executable mode and fail closed when the tracked preflight file is absent.
