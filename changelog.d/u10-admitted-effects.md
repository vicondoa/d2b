### Added

- Admitted-effect boundary for privileged broker effects (U10, KTD8). A
  privileged effect now crosses the origination leg as one closed carrier
  that names an `Operation`, the initiating subject, the relationship legs it
  runs on, typed non-authority parameters, the dependency versions the caller
  expects, and an idempotency key. The carrier has no field for a host path,
  a numerical credential, a mount policy, or a launch command line, and it
  admits no legacy wire variant: `admittedEffect` is its only frame kind.
- `d2b_core::execution_plan`: the resolved private plan - the exact sources,
  destinations, views, identities, executable, and dependency versions one
  admitted effect runs against - derived from the binding contracts and the
  accepted graph and fenced by the store incarnation and desired revisions.
  The plan derives no `Serialize`, so it cannot be persisted, replayed, or
  returned to a caller, and a dependency the broker holds no private value for
  is refused rather than approximated.
- A closed authority-bearing parameter screen. An operation whose own declared
  payload schema names a host path, a command line, an environment map, a
  numerical credential, a mount policy, or a launch posture is refused by name
  at admission, and so is a supplied payload carrying one whether or not a
  schema declared it. Authorization is no longer inferred from argv,
  environment, or naming conventions.
- The broker's admitted-effect admission: an unaccepted or fenced Zone
  projection refuses first, an `Operation` no declared implementation serves
  is refused by name with no fallback route, a nested leg is admitted only
  against the subject its own root was admitted for, a caller-expected
  dependency version the broker no longer observes is fenced, and returned
  descriptors are checked against the operation's declared response contract
  for count, name, and kernel kind before the reply is built.
- An idempotent effect ledger: a retry is answered from the recorded outcome
  with the original invocation's answer, and one key presented again with
  different parameters is a conflict rather than a second effect.
- `d2b-provider-process` publishes its launch effect as a canonical
  `Operation` contract whose payload declares one typed non-authority
  parameter and whose response declares the pidfd it returns.
- `d2b_broker::kernel_ops::launch_posture` resolves a launch's posture from
  the admitted plan plus a verified deployment template, replacing the
  posture a request payload used to carry. A presentation the plan's resolved
  destinations do not realize is refused rather than skipped.
