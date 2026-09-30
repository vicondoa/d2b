### Added

- Added the canonical confinement, callable, and authority contracts for the
  unified resource graph. The new `ExecutionPolicy` resource states what an
  execution instance may be - required isolation classes, the Linux
  capability ceiling, the restrictions it may not weaken, the identity it may
  resolve to, and the syscall filter it must load - and carries no volume,
  device, network, endpoint, credential, mount, or host-path field, so
  resource access can no longer be granted by naming a policy. Admission
  composes field-wise and refuses on conflict: a missing required class, a
  capability outside the ceiling, a weakened mandatory restriction, an
  unauthorized identity, an incompatible syscall filter, a request over the
  admitted budget ceiling, or a mandatory confinement facet the target cannot
  enforce is refused with its enforcing stage, never intersected into a
  successful but unconfined launch. A long-running `Process` and a
  run-to-completion `EphemeralProcess` pass through that one path, and
  selecting a policy is a request: without Role and RoleBinding authorization
  evidence the selection is refused even when every other rule would admit it.
- Added the narrowed `SeccompProfile` contract, which carries the syscall
  filter and its default action and nothing else. Namespace, cgroup,
  device-node, and mount authority are not syscall-filter concerns, so a
  profile row that still carries them is rejected rather than decoded with
  its authority quietly dropped.
- Added the canonical `Operation` contract, where an operation binds one
  trusted implementation - a declared provider component method or a
  provider-owned executable template - instead of an owning `Command` row.
  A caller cannot select code, and a mutable provider row cannot introduce a
  compiled privileged handler by naming an artifact.
- Added the shared authority vocabulary every admission and effect surface
  speaks: the initiating subject, a per-row desired revision that advances on
  every committed desired mutation rather than only on a spec-generation
  change, a per-Zone desired sequence, a store incarnation that is an
  identity and never an ordering, the freshness tuple an effect is fenced
  against, the stage a decision was made at, and a field-free refusal reason
  that can be logged or audited verbatim.
- Added the authorization-only Role contract, which states which resource
  verbs a subject may use and which declared operations it may call. The
  retired posture, its path grants, and its command references are rejected
  by the decoder instead of being decoded with their authority ignored.

The existing `Host`/`Guest` execution-policy fragment, the `Command`-owning
operation row, and the role posture remain in place for the production entry
point that has not switched yet; the conversion units that own those callers
switch them to these contracts, and the integrated cutover removes what they
replace. Production schema and generation output are unchanged by this entry.
