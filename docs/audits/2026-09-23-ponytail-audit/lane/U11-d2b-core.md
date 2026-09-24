# U11 d2b-core

Types-layer crate (~15,848 LOC, 27 modules incl. the huge `bundle_resolver.rs` + `bundle_resolver` subfamily). Types-layer crates include the Consistency notes section. Callsite verification mandatory; every zero-caller claim backed by a workspace-wide caller-family search.

## Findings (biggest cut first)

- `delete` **The one-dead-member half of the six-shim compat surface.** `packages/d2b-core/src/privileges_w3.rs` (41B, one line `pub use d2b_contracts::privileges_w3::*;`) plus its `lib.rs` mod arm (runtime.rs re-export turned out to only arm the shim modules). Zero callers anywhere in the workspace: `d2b_core::privileges_w3` and `d2b_core::privileges_w3::<name>` both search to zero matches over `packages/**` Rust + BUILD.bazel; the W3BrokerOperation vocabulary the crate-shim re-exports is read via `d2b_contracts::privileges_w3::*` / `d2b_contracts_broker::privileges_w3::*` in live broker/catalog crates (d2b-broker/src/catalog.rs:732, d2b-contracts-broker/src/lib.rs:11), and the daemon-side peers (`privileges_w3.rs` in `packages/d2b-provider-guest-*` crates and `d2bd-runtime/`) each import from `d2b_contracts` directly. Deleting the dead shim leaves the compat story intact for the five live shims. [packages/d2b-core/src/privileges_w3.rs + packages/d2b-core/src/lib.rs (mod arm)] (leaf)
- `stdlib:` **base64_codec.rs hand-rolls RFC 4648 padding rules where the workspace declares `base64 = "0.22"` (workspace Cargo.toml:182) - no wrapper onto that dependency landed.** Live wire surface: used by d2bd composition.rs (public_wire::…::data_base64), d2bd-runtime exec_session.rs/console_session.rs/shell_backend.rs, d2b-broker context.rs, and the broker wire envelope (`data_base64`, `chunk_base64`, `stdin_chunk_base64`). The codec is the canonical wire codec crossing daemon/broker/CLI boundaries - replacement (wrapper onto workspace base64) is wire-surface work owned at the broker family; refused in prior pass (dependency-free bootstrap-crate intent), no new evidence of a changed blocker. [packages/d2b-core/src/base64_codec.rs] (family)

## Refusal ledger / prior rows (crate rows from U1 constraints packet)

| Row | Status | State at HEAD |
| --- | --- | --- |
| #A3 runtime.rs re-exports 8 runtime DTOs | [applied] | Verified: `packages/d2b-core/src/runtime.rs:1-6` re-exports the eight names from `d2b-contracts` (all eight names + the remaining local runtime types) - applied, field-for-field duplicates gone |
| #A4 6 one-line compat shim modules | [not applied] | Five live (error.rs, contract_id.rs, configured_argv.rs, workload_identity.rs, unsafe_local_workloads.rs - callers cited in findings); `privileges_w3.rs` has zero workspace callers → reported above under `delete`. The "MODULE_NAME const" premise **does not hold at HEAD** - `MODULE_NAME` appears nowhere in `packages/d2b-core/` (verified clean single-purpose grep; no MODULE_NAME in the crate, and no MODULE_NAME shim arm in lib.rs). |
| #A8 base64_codec hand-rolls RFC 4648 | [not applied] | Present as reported; dependency-free intent documented in the module doc comment; refused (see finding above). |

## Consistency notes

Types-layer crate - the canonical home for each DTO family is `packages/d2b-contracts/`. The five live shims are one-line `pub use d2b_contracts::<name>::*;` compat arms onto that canonical surface and do **not** re-declare DTO bodies (no hand-rolled field-for-field duplicates re-landed for the shim family itself). `runtime.rs` similarly re-exports the eight runtime DTO names from `d2b-contracts` rather than re-declaring them (row #A3 [applied] - verified). No duplicate type definitions, naming drift, or wire-shape skew found within the shim/runtime surface; `src/generated/` and the contacts booklet are out of per-crate scope.

## Checks performed

- Read `packages/d2b-core/src/{lib.rs,error.rs,contract_id.rs,configured_argv.rs,privileges_w3.rs,workload_identity.rs,unsafe_local_workloads.rs,runtime.rs}` at HEAD.
- Caller searches (each mode mentioned): `d2b_core::privileges_w3::<name>` → no matches; `d2b_core::privileges_w3` → no matches over `packages/**` Rust + BUILD.bazel (workspace-wide grep, gitignore-true); live-shim callers verified in d2bd composition.rs, d2bd-runtime, d2b-broker (citations above); MODULE_NAME grep across `packages/d2b-core` → no matches.
- Ledger rows #A3/#A4/#A8 verified at HEAD as tabulated.

## Ledger items honored

- #A3 [applied] - verified applied (runtime.rs now re-exports the eight names).
- #A4 [not applied] - re-verified: five live shims stay (compat surface), `privileges_w3` reported as the one dead member, MODULE_NAME const premise not present at HEAD.
- #A8 [not applied] - re-verified present; refused (dependency-free wire-codec intent, no blocker change).

**Net:** -1 line, 0 deps. **Note:** MODULE_NAME const found absent in this crate at HEAD (the prior row's const premise did not survive to HEAD), narrowing the compat surface by one real member.
