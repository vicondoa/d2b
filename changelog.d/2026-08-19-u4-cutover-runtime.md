### Added

- Add the unblocked U4 cutover runtime boundary: typed host cutover CLI and
  daemon admission, a single-use fd bootstrap capability, an out-of-band
  runner with durable journal and OFD lock ownership, lifecycle-authenticated
  status and hold/resume socket controls, and the narrow broker launch
  operation. Hold and resume fail closed until the privileged audit boundary
  returns durable evidence. Register the runner as a non-persistent host tool
  and route typed effects and closure activation through the adapted broker
  capability boundary and typed host-generation handoff; existing storage,
  activation, provider-start, and verification operations are adapted with
  durable evidence. Add marker-bound quarantine, separately consented
  finalization, and durable-Volume destroy adapters; scoped reset admission
  uses a separate scope-bound operation capability and consent.
