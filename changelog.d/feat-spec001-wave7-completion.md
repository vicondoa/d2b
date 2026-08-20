### Fixed

- Normalize cutover runner responses through the public contract, keep doctor
  available during daemon drain, bind host-generation handoffs to admitted
  preview and consent artifact identities, validate handoffs before host drain,
  validate both candidate and rollback artifacts through the trusted helper
  before host drain, recover ambiguous apply responses from runner state, use
  explicit mutating resource transport for cutover writes, and transfer
  bounded runner bootstraps without pipe backpressure.
- Return precise artifact-binding failures, release failed launch capabilities,
  register cutover runner pidfds for broker reaping, and make live cutover
  assertions consume public JSON without forbidden `jq -e` predicates.
- Require runner admission readiness before d2bd reports an apply launch,
  clean up registered runners when launch auditing fails, and reject missing
  operation ids on every runner-owned continuation command.
- Validate cutover artifact identities with the canonical v3 grammar and
  retain runner process identity in the broker capability registry so an
  exited runner does not permanently block a retry.
- Permit dead-runner capability rotation during journal adoption and have
  repeated apply commands resume through an already admitted runner.
- Refresh CI-owned cutover completions and daemon API artifacts, restore the
  cutover policy markers and broker disposition rows, preserve admin and
  launcher lifecycle-group membership, and declare the direct Bazel contract
  dependency for the vsock framing test.
