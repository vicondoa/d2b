### Changed

- `Provider/shell-terminal`'s pool, session, and terminal service now consume
  the admitted graph instead of a shell-specific launch path. The supervisor
  `Process` child is derived from the Provider's own declared contract (one
  `Process` provider and one trusted executable template, published once as
  `SUPERVISOR_PROCESS_PROVIDER_REF` / `SUPERVISOR_PROCESS_TEMPLATE` and
  built by `supervisor_execution_spec`), so the session driver no longer
  composes its own Process provider, template, or execution shape. Pools and
  sessions may declare the `Endpoint` their interactive stream is admitted
  on, and that relationship is validated and watched like their execution
  target and `User`.
- Terminal stream admission is graph-bound. A `TerminalStreamBinding` names
  the exact `Process` that consumes the stream, the exact `Endpoint` it is
  attached to, and the reconnect generation below which the admission no
  longer speaks; `SessionSupervisor::attach_admitted` measures an incoming
  stream's `TerminalAttachEvidence` against it before the attachment census
  moves, and the bounded replay ring stays a data-plane payload on the
  receipt rather than part of the control decision.
- Restart adoption is a measurement against live evidence rather than a name
  comparison. `adopt_supervisor` now takes the session and its admitted
  stream binding and retains a candidate only when it proves the session's
  own supervisor `Process`, the admitted endpoint, a reconnect generation the
  fence still accepts, and exactly the expected identity. Foreign-`Process`,
  foreign-endpoint, and stale-reconnect observations are separate
  `AdoptionDecision` values instead of one "ambiguous".
- A user-domain supervisor is started through `start_supervisor_for` /
  `SupervisorProcessResource::admitted_for_session`, which require the
  `WorkloadIdentity` the process adapter proved the launch actually ran
  under and refuse any `User` other than the session's admitted one before
  the supervisor is claimed.

### Fixed

- Session and pool removal drain only what they own. The authority's
  user-domain census keeps every `Process` row in the workload user's
  domain, and removal deletes exactly the retired session's own row: another
  session's supervisor, an unrelated process of the same `User`, and another
  user's process all survive. Removal is still refused while an attachment
  is open.