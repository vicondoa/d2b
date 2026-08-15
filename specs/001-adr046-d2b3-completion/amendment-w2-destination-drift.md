# Historical W2 destination drift

This artifact preserves a recorded destination discrepancy for rationale only. The former
process-package destination was retired; no current task may write that path. Existing code
and the owning product contract determine the implementation boundary.

The relevant Process contracts remain represented by
`packages/d2b-process-conformance/` and the typed Provider/EffectPort surfaces that consume
them. A destination name in historical prose does not create a new crate, service, or
implementation requirement.

Current changes use focused source, contract, and negative-policy tests to resolve this
historical note.
