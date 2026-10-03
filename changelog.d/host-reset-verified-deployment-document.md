### Fixed

- `d2b host reset` can now succeed on a real host. It read a
  `deployment-graph.json` document with a `d2b-deployment-graph/1` schema that
  no installation path ever wrote, while the deployment publishes
  `deployment-bootstrap.json` with a `d2b-deployment-bootstrap/1` schema, so
  every invocation refused. The reset now verifies the published deployment
  document itself, through the same shared contract the daemon verifies, and
  leaves it in place: it is the installation's artifact, not runtime state.
- The reset's admission could not fail. It built the graph it admitted against
  with the request's own subject as the deployment root, so the evaluator's
  root check compared a value with itself and admitted whatever the runner
  asked for. Explicit local operator authority is no longer a deployment root:
  the graph is rooted at the verified deployment document, and the identity the
  reset acts as is resolved from that document's accepted `RoleBinding` rows.
  A deployment that grants nobody, grants a different verb, or grants a
  different `Operation` now refuses the reset instead of admitting it.
- A deployment root is identified by the deployment document it publishes
  rather than by a `d2b-ownership` marker file that no installation wrote. A
  root with no verifying document is not a d2b deployment root, and a document
  that changed between admission and removal never authorizes an overwrite.

### Changed

- The reset's deletion inventory is a code-owned, deployment-root-relative
  declaration of the surfaces the deployment root itself owns: the broker's
  and the daemon's runtime state, the guest-side state they share, and the
  per-Guest and per-Zone roots. Family-owned subtrees are absent on purpose: a
  subtree belongs to the provider that owns it and the broker is pinned
  provider-free. The declaration narrows - an undeclared surface is never
  unlinked, whether it sits inside the deployment root or beside it - so a
  family that needs one of its subtrees retired does it through its own typed
  operation with its own ownership proof.
- The reset report names the verified document's self-hash as the ownership
  id and the identity the document's accepted grants admitted, and no longer
  reports a declared set of external Volume sources: the boundary is the
  declaration, so nothing outside it is ever a candidate.