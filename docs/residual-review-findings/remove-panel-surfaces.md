# Residual Review Findings

## Residual Review Findings

- P2 - `packages/d2b-bus/tests/public_mint_surface.rs:36` - API mint coverage is no longer workspace-exhaustive. Conflicting decision: remove the compiler-derived API census in favor of existing defining-crate compiler assertions, compile-fail tests, public mint tests, mutation seals, and wire contracts. The narrower coverage was user-directed and proceeds under this recorded tradeoff.

Source: PR #415, branch `remove-panel-surfaces`, review run `20260814-171812-47096b60`.
