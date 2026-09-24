# U30 d2b-provider

Lean already. Ship.

"Death by a thousand déchets" does not apply: every public item in this
~2,585-line crate has a live production caller outside itself. Verified
workspace-wide (caller-search per plan constraint 2, both Rust callers and
BUILD/nixos-module/toolkit references):

- registry.rs admission/registry/drain/manager/permits — consumed by
  d2bd provider_registry.rs + runtime admission + conformance (toolkit
  fixture uses the same builder/descriptor/registry surface)
- agent.rs dispatcher half — kept per prior finding #P7/#S1 (toolkit's
  FakeProvider implements ProviderAgentService; d2bd admission gates
  dispatch through the agent ring)
- operation_ledger.rs (with_capacity/admit/rebind/transition/row/rows) —
  dummy callers: d2b-provider-toolkit runtime.rs + runtime admission
  (admit/admission ledger) + `row` accessors used in runtime.rs:535 and
  broker conformance
- identity.rs (ProviderClass 11 families, ProviderImplementationId,
  ProviderMethodName, ProviderCapabilitySet, capability families) — ALL
  exercised by the preserved-eleven test + fixture + toolkit `from_specified`
- session.rs SessionIdentity matches—— matches_descriptor used by registry
  admission (registry.rs:421 uses identity.matches_descriptor); toolkit
  conformance
- CancellationToken/context.rs OwnedOperationContext new_linked — used by
  d2b-provider-toolkit base runtime + admission
- instance.rs ProviderInstance — used by d2bd provider instances + toolkit
  fixture
- error.rs — all variants constructed/displayed in agent + registry paths

No prior-flag refusals reopened (P7/S1/B2 Refusal stays Refusal); nothing new.

## Checked
Read every file in packages/d2b-provider/src + tests; verified workspace
callers via grep. net 0.
