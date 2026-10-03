### Changed

- Shared binding machinery is split into three modules instead of one:
  `v3::binding_slot` owns consumer slot allocation (`BindingSlot`,
  `BindingSlotAddress`, `BindingSpecFingerprint`, `BindingSlotDecision`,
  `BindingSlotEntry`, `BindingSlotIndex`, and its declaration, payload-change,
  and observation rules), `v3::binding_lifecycle` owns the observed lifecycle
  and realization vocabulary (`BindingLifecycleState`, `CompletionCondition`,
  `ReleaseOutcome`, `BindingRealizationFacet`, `BindingRealizationSupport`), and
  `v3::binding` keeps the request and authorization core. Both are re-exported
  from `v3::binding`, so every `d2b_contracts_resource::v3::...` and
  `v3::binding::...` import path is unchanged and no contract type, schema, or
  wire encoding moves.