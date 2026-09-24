# U28 d2b-process-conformance

net: -10 lines, -0 deps


- #P0 [applied] Dead stop-proof suite helper - `suite::children_have_verified_stop_proofs` at packages/d2b-process-conformance/src/suite.rs:297 (10-line pub body). Zero workspace callers: the only refs are its own crate's test asserts at suite.rs:415,420,425. Both Provider crates drive the identical obligation through the live sibling `assert_finalizer_requires_verified_stop` (conformance.rs:195/214) plus the shared `validate_stop_proof` helper the finalizer gates run. Deleted. [packages/d2b-process-conformance/src/suite.rs:297] *(leaf)* -10 lines, -0 deps

## Caller verification (mandatory, plan constraint 2)
Constraint-honored caller census, workspace-wide, both Provider crates + their conformance suites:
- `assert_finalizer_requires_verified_stop` : 4 external (provider-systemd conformance:195, provider-minijail conformance:214, +2 suite self-pins)
- `children_have_verified_stop_proofs` : **0 external** → the deleted finding above
- `children_have_verified_stop_proofs` (WaitReapOwner arg) : 0 external, own-crate test-only → same finding
- `validate_stop_proof` : 4 external (systemd:194, minijail:213, +2 in suite's own terminal.rs callers)
- all `error.rs` codes : unique, live 2-4 callers each
- `ProcessProviderProfile`/`LaunchIdentity`/`LaunchIdentityError`/`ProcessConformanceError` : all consumed by both Provider crates + this crate's own suite

## Refusal ledger (workspace-wide verification, cross-crate classes for type-layer crates)
- No "refused-stays-refused" class applies: this crate has no policy-required scaffold, no `integration/*.rs` surface of its own to pin, no generated shape, and no declared-provider surface - it is the neutral conformance suite shared by the two Provider crates aopening. Each does run it.
- Cross-crate hand-rolled UUID rendering (#P7/#S1 family) : N/A here - this crate renders no UUIDs; identity digests are shapeless bytes passed to d2b-contracts `ResourceUid::from_bytes`.
- Both Provider crates are stack-level consumers of this suite (leaf); no cross-crate scaffold to defend.

No prior findings. Clean, ship. Remaining -10 lines.
