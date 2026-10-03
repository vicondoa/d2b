---
type: fixed
area: daemon,credential
---

### Fixed

- **A committed `Credential` row now commits the delivery relationship it
  declares.** The `CredentialBinding` derivation existed and was tested, but no
  driver verb called it, so a boundary evaluating the accepted graph saw no
  committed source decision for any credential delivery and every binding
  admission refused with an absence. `CredentialDriver::reconcile` now derives
  the rows from the committed spec alone and commits each one through the
  manager-routed child ensure before any Provider effect runs, so the
  relationship exists exactly as the source declared it. A second pass over an
  unchanged row commits nothing new, and a spec that stops declaring the
  relationship - a Host-scoped row, a withdrawn delivery operation - retires the
  committed row instead of leaving it live.
- **A derived delivery row is refused rather than committed when it is outside
  the source row's own policy.** Every derived row is re-checked against the
  `Credential` spec's `allowedOperations`, its `maxLeaseLifetimeMs` ceiling, its
  `scope.executionRef` destination, its bound `Credential`, and the row name
  those identities derive, and one row outside those bounds refuses the whole
  set with a terminal `credential-binding-refused`. The delivered destination
  is the source row's own `scope.executionRef` and never its `consumerRef`,
  which names the Provider a delivery session is minted against.