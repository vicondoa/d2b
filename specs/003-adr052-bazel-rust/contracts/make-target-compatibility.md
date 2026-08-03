# Make Target Compatibility Contract

## Shadow stage

`make test-rust` and its eight existing leaf names remain Cargo-authoritative.
W1 adds:

```text
make test-bazel-rust
make test-bazel-rust-main
make test-bazel-rust-api
make test-bazel-rust-broker
make test-bazel-rust-aux
make bazel-shutdown
```

The aggregate invokes all eighteen baseline surfaces. Slice targets invoke
only their coverage-map rows. `bazel-shutdown` uses the same startup options
as validation, deletes nothing, and returns nonzero with
`D2B-BZLSERVER-STUCK` if its own bound expires.

Every workflow calls one of these approved Make targets, never Bazel directly.
All targets run from repository root, preserve carrier-attributed diagnostics,
and return nonzero when any selected enforcing carrier fails.

`D2B_EXECUTION_MANIFEST` accepts the requested evidence path and binds either
aggregate executor to existing manifest v1. `D2B_RUST_BUDGET` accepts a
positive integer and remains the only resource budget. Cold-local evidence
preparation is an internal, temporary W2 xtask helper, not a Make target or
environment contract, and is removed in W5.

## Promotion

- Required context remains `test-rust`.
- `make test-rust` routes eighteen surfaces through Bazel and retains the
  Cargo/Nix fixture leaf.
- All eight existing `make test-rust-*` leaf names keep working as thin
  mappings to authoritative carriers.
- Bazel-specific names become forwarding aliases. Each prints one stderr line
  naming its replacement and exits with exactly the forwarded status.
- No workflow may call a deprecated alias.
- `make bazel-shutdown` remains while Bazel is authoritative.

## Removal

Bazel-specific aliases may be removed only in a separate change after a
release tag contains promotion and the promotion deprecation shipped. Cargo
leaf implementation may be removed only in a later change after ten
consecutive green promoted `v3` runs. `fixture-contracts` remains.

## Guard

Existing `packages/xtask/tests/policy_ci.rs` owns approved target and workflow
shape policy. Positive and negative fixtures must prove recognition of direct,
indirect, post-step, and unknown workflow writers. No new Make gate is added.
