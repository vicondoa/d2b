### Fixed

- Ordinary Process launches are no longer refused by the broker. Every
  production runner row declares a read-only `/nix/store` plus default
  device-node hiding, and the `spawn-process` kernel hardcoded the
  namespace-first presentation realization, so `sys.rs` refused EVERY such
  launch with `presentation-requires-mount-realization` while every kernel test
  stayed green - each of them used an empty mount policy, which cannot reach
  the refusal. The realization is now DERIVED from the launch row's own mount
  policy (`kernel_ops::launch_presentation`): a row that declares anything the
  broker must realize in a mount namespace - a mount namespace of its own, a
  read-only or writable path, a device bind, a cross-domain bind, the read-only
  Nix closure, or default device-node hiding - launches under
  `FilesystemPresentation` with that policy actually applied, and a row that
  declares none keeps `NamespaceFirstServiceSource` with nothing to skip. The
  refusal itself is unchanged and still refuses any producer that pairs the
  two the other way round, so a requested mount is never silently skipped. The
  regression is covered by a kernel test that drives a byte-for-byte
  `mint_template_intent` mount policy and asserts the launch is served.
- `RunnerIsolationSpec::presentation` is documented as the non-optional value
  it now is; the changelog claim that it is `None` for an unclassified launch
  was no longer true of the code.
- A committed binding row is no longer dropped by the authority projection.
  The projection stored only `Role` and `RoleBinding` rows, so the source
  provider's own accepted decision about a relationship was never persisted
  and every relationship leg was an absence - which refuses. Such a row is now
  stored with the resolved identity it was published under (its source and
  consumer row uids), and the graph is read through that identity, so the
  accepted source is reachable under the exact key a later admission names.
  The two uids are identity rather than spec bytes: a relationship key is over
  committed identity, the row deliberately does not repeat them, and that is
  what stops a rename producing a second relationship without the projection
  becoming a second copy of the manager's desired store.

### Added

- The admitted-effect boundary has its production refusal vocabulary:
  `BrokerError::AdmittedEffectRefused` answers with the boundary's own closed
  refusal code as the wire `kind` and a fixed operator phrase per code, so a
  refusal is a decision a caller can act on rather than a malformed-wire drop,
  and no host path, command line, or numerical credential is ever reflected
  back. `screen_admitted_effect_frame` is the production refusal path, and
  `AuthorityProjection::accepted_graph` reads a Zone's published graph through
  the same constructor the fence decisions read - so an effect admission and a
  mutation admission can never see different authority - returning no graph at
  all for a Zone with no durable record or undecodable rows, so the caller
  refuses rather than deciding against a half-read authority.
- The admitted-effect gate is installed in the production accept loop, in
  front of the typed decode and behind the retired-variant gate. An
  `admittedEffect` frame is screened, admitted against the broker's own
  published authority projection, and answered; it can no longer fall out as
  a malformed-wire drop with the connection closed and no reply, and no other
  frame kind is routed into the boundary. The declared implementation table,
  the private execution values, and the chain-audit sink are installed once at
  serve time from the same inputs as the committed-operation envelope, so the
  gate never builds a table of its own for one call.

  It currently admits NO successful effect, and each reason is a named code
  rather than a silent skip. A Zone this broker holds no accepted, unfenced
  authority for refuses `unaccepted-projection`; an `Operation` no declared
  implementation serves refuses `unknown-implementation`; and a plan the
  broker resolves no private executable for refuses
  `untrusted-implementation`. The legacy raw-posture launch route is therefore
  still the only way a launch runs, and the gate stands in front of it
  refusing, until the private execution values have a declared source.
- The admitted-effect boundary now hands the descriptors an implementation
  returned to the caller instead of dropping them: the answer carries their
  declared name and observed kind, and the live descriptors travel beside it
  in the answer's own order, so the frame's `SCM_RIGHTS` list is
  position-aligned with what the answer names. A recorded outcome that
  declared descriptors is refused on replay rather than answered with entries
  the frame does not carry - the host effect still happens exactly once
  either way, and a partial success is a lie the caller would act on.
