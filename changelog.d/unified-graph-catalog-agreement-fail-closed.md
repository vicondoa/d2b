### Fixed

- The offline Provider catalog and the declaration projection it must agree
  with are now compared unconditionally. The agreement assertion used to be
  emitted only when some artifact already published a declaration, so a
  packaging regression that dropped one deleted the check instead of failing
  it, and a catalog selecting any row at all went unreviewed. A selected row
  no declaration produces, and a declaration no row selects, are each refused
  with a message naming both closed sets. A configuration that selects nothing
  and declares nothing still passes, because both sets are then empty and
  there is no expectation left to state.
- Every graph-policy refusal case now asserts the specific refusal it is named
  for, not only that evaluation threw. The test-owned policy projection
  returns its refusal list beside the policy, so a case named for a retired
  privilege knob, role-scope table, family-scope table, broker wire-variant
  list, principal allocation, retired projected row, unroutable method,
  orphan registration, orphan service, or old contract version fails when the
  reason it is named for changes. The retired-knob and contract-version
  diagnostics are now single lines, so the refusal reads the same in the
  message as in the assertion.
