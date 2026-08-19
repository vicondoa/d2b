### Added

- Add the unblocked U4 cutover runtime boundary: typed host cutover CLI and
  daemon admission, a single-use fd bootstrap capability, an out-of-band
  runner with durable journal and OFD lock ownership, lifecycle-authenticated
  status and hold/resume socket controls, and the narrow broker launch
  operation. Hold and resume fail closed until the privileged audit boundary
  returns durable evidence; trusted Nix registration remains coupled to PR #440.
