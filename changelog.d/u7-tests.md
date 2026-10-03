### Fixed

- The broker's admitted authority projection keyed its durable row,
  reservation, and session-derivation indexes by `Display for ResourceRef`,
  which is the redacted diagnostic rendering. Every reference therefore
  resolved to the single key `ResourceRef(<redacted>)`: a published snapshot
  of a `Role` and a `RoleBinding` kept only the last row, so the prior
  accepted graph the admission decision reads was silently missing a grant;
  a retirement named a reference no index could hold; and two initiating
  subjects that differed only in their resource derived the same publication
  session. The indexes now key by the reference's canonical rendering.
- `publication_candidate_digest` hashed each row's reference through the same
  redacted rendering, so two candidates that differed only in which resource a
  row was for produced one digest and a transaction identity named bytes it
  did not pin. The digest now covers the reference's canonical rendering.
- `CommitChange` replaced the whole accepted authority row set with the rows
  the commit carried, and a commit carries the candidate its fence was
  prepared for. Committing one non-authority row therefore erased every
  accepted `Role` and `RoleBinding` and silently dropped the Zone's grants.
  The committed rows are now merged over the accepted set by reference, and
  the named retirements are removed from it.
- The accepted-lower-bound check compared whole cursors, so a document or a
  resynchronization floor naming the accepted sequence with a different digest
  passed instead of being refused. The digest is now compared at an equal
  sequence, as the store-incarnation rule requires.
- A refused `PrepareChange` was reported by the manager's publication
  coordinator as an unmatched completion, discarding the broker's closed
  refusal code and the Zone state. A refusal is an answer, and is now
  reported as one.

- The manager's publication coordinator deadlocked on its own session lock
  whenever a message had to open its own session: the guard on `session` was a
  temporary in the `match` scrutinee, so it was still held across the
  `open_session` call that takes the same lock. Every publication that did not
  open a session first hung, and a control action over it reported a timeout
  rather than a fault. The held session is now read into a local before the
  `match`.

### Added

- `//packages/d2b-broker:authority_publication`: the broker-side publication
  suite over the real durable projection. It covers candidate rejection
  (self-grant, wrong Zone, stale predecessor, missing and out-of-order chunk,
  every declared snapshot ceiling, duplicate conflicting transaction, digest
  mismatch, unexpected store incarnation), the session fence in value form,
  the bounded control lane and its binding requirement, the exec-release gate
  and the reducing proof obligation in both orderings, restart reconciliation
  and the resync floor, and every mutation recovery boundary.
- `//packages/d2bd:authority_publication`: the manager-side publication suite
  over the coordinator's own ordering and memory. It covers the
  freeze/publish/acknowledge order, the callback-free prepare and commit, the
  completion that applies only to a matching transaction and sequence, the
  bounded queue behind one pending transaction, the control timeout that keeps
  the fence, acceptance kept distinct from revocation convergence, the
  reducing-policy race in both orderings, and every mutation recovery row with
  the interruption injected at that boundary.
