### Fixed

- The Cloud Hypervisor Guest's `ch-api` Endpoint reaches `Ready` and the Guest
  converges with its controllers. The Endpoint driver's control family was
  derived from only one of the provider's two fixed child roles: it admitted
  the Guest-produced, cross-domain `guest-control` shape and refused the
  `ch-api` shape the provider commits on the VMM Process (producer
  `Process/<guest>-vmm`, materialized `locality: host-local`), so that row
  failed `validate` terminally (`endpoint-shape-unsupported`), the guest's
  endpoint-publication stage refused its `Failed` phase, and the Guest stuck
  `Pending`. The admitted family is now derived per purpose from the
  provider's own role vocabulary (producer ResourceType and materialized
  locality), and the Endpoint presence probe reads the evidence row that role
  declares: the producer row itself for `ch-api`, the guest's deterministic
  VMM Process child for `guest-control`.
- The endpoint-publication stage defers on a child's retryable `Failed` phase
  instead of refusing it. The `ch-api` Endpoint actor's bounded realize effect
  waits for the VMM evidence and fails retryably while the VMM is still coming
  up; treating every `Failed` phase as terminal failed the whole Guest effect
  (`CapabilityUnavailable`) rather than letting the endpoint's own requeue
  converge. A terminal child failure still refuses.
- The `guest-session-endpoint` read is manager-authority for the converted
  `Endpoint` type. The pre-v3 store fallback reported `ResourceNotFound` for a
  row it does not own (logged as a failed store read); a row the manager does
  not hold is now the honest not-committed answer, and a manager or plane
  failure stays a read failure the callers retry - never absence.
