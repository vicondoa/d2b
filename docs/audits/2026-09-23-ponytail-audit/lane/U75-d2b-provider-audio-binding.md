# U75 d2b-provider-audio-binding

net: -0 lines, -0 deps (crate is the audio-binding family's binding-type crate; ledger rows #S3/#S4 honored, no in-scope cut remains)

## Findings

1. **shrink [refused row honored] - Cross-cutting interaction-family declaration boilerplate (#S3).** The audio-binding banner crate's per-crate surface is limited to the `AudioBinding` type declaration plus its driver/factory aliases. The broader family boilerplate (approx. 250 lines claimed) was measured at ~200 lines across six crates with only ~70 safely removable; this crate's own drive half is the already-shrunk 20-22 line driver; the *_spec_decoder()/alias surface is public and tested. Measured: **0 safely removable lines remain in-scope**. Honored as [refused]; no new evidence to reopen.
2. **shrink [partial row honored] - Cross-cutting dead per-type constants (#S4).** The six *_CONTROLLER_REF constants deleted with their re-export arms. The paired *_RESYNC constants refused - each is the return value of its own InteractionType::resync() (audio-binding/src/audio_binding.rs:444-451 calls resync() as the binding's cadence); deletion would remove an exercised path. Refused rows stay; no new evidence.

## Measured (this crate)

- src/audio_binding.rs - the `AudioBinding` type, `AudioBindingFactory`/`AudioBindingDescriptor` declaration, and the crate's own driver impl. The crate is a leaf in the binding family with the shared execution engine (d2b-provider-wayland-policy) and the shared metadata driver (d2b-resource-runtime/src/metadata.rs:535).
- Cargo.toml - depends on d2b-provider-audio-pipewire (the pipewire provider ref), d2b-contracts-resource (shared identity + redacted_debug!), d2b-provider-wayland-policy (the family interaction engine). No telemetry-crate dependency remains (removed in the #S4 partial).
- Debug surface: `redacted_debug!` is importable both here and in the sibling audio crates; this crate's hand-rolled surfaces measured at zero after the family-wide redaction migration (#A2 family arm: audio crates' redacted_debug! applied in prior passes).

## Consistency notes

- **Audio family typed session identity:** the audio-binding family renders its identity through `d2b_contracts_resource::v3::identity`'s shared `AuthenticatedSubjectContext` - no local duplication of the AuthzSubjectContext shape in this crate.
- **Wire-shape extension skew:** none found - `AudioBindingDescriptor` (audio-binding.rs:120-160) agrees with the shared interaction-family wire; the family's `InteractionType` declarations are the refused boilerplate class above.

## Refusal ledger honored

- #S3 [refused] interaction-family declaration boilerplate - no new evidence to reopen.
- #S4 [partial] per-type *_RESYNC constants - refused half stays; no new evidence.

## Checked

Read packages/d2b-provider-audio-binding/src/{audio_binding.rs,lib.rs}, Cargo.toml, tests/; honored U1 packet rows; no zero-caller claims made without workspace caller verification (family's _*_CONTROLLER_REF deleted arms and paired _RESYNC return-value constants verified via workspace grep).