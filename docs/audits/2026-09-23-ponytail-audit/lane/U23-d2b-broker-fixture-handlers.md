# U23 d2b-broker-fixture-handlers
net: -30 lines, -0 deps

- delete: four exported symbols with zero workspace callers - `PURE_ECHO` const (lib.rs:23), `DeclaredOperation` struct (lib.rs:26), the `OPERATIONS` table const (lib.rs:39), and `declared_operations()` (lib.rs:45). The composition root links the handler directly by qualified path (`d2b_broker_fixture_handlers::echo` at packages/d2b-broker-composition/src/seam.rs:347,546,695), so the machine-readable declaration table is never read; keep only the live `echo` handler and the direct invocation types it needs. Reference-search method: workspace-wide `rg` for `PURE_ECHO`, `DeclaredOperation`, `OPERATIONS`, `declared_operations`, and `fixture_handlers::{…}` across all file classes (Rust `*.rs`, `BUILD.bazel`, `*.bzl`, `nixos-modules/`, `tests/`, `docs/reference/`, `*.json` catalogs, `Cargo.lock`) - zero hits outside the crate's own lib.rs. [packages/d2b-broker-fixture-handlers/src/lib.rs] (leaf)
- shrink: lib.rs module doc claims the crate "registers its declared operations on the broker's handler table" - the composition root registers the handler by qualified path, not through `declared_operations()`; trim the doc to describe the crate as what it is, a fixture echo handler linked by the composition root. [packages/d2b-broker-fixture-handlers/src/lib.rs] (leaf)

## Consistency notes
N/A - not a contracts/types crate.

## Reopened refusals
None.

## Checked
Read the full crate (57 LOC, lib.rs only); confirmed `echo` has three live qualified call sites in d2b-broker-composition/src/seam.rs (347, 546, 695) via seam.rs and routing.rs; ran workspace-wide reference searches for `PURE_ECHO`, `DeclaredOperation`, `OPERATIONS`, `declared_operations`, `fixture_handlers::` in Rust + BUILD.bazel + xtask + nixos-modules + tests + docs + committed JSON catalogs + Cargo.lock - no external callers for the four exports; verified the fixture crate is not provider-prefixed so gen_broker_operations skips it (PROVIDER_PREFIX filter at gen_broker_operations.rs:635) and no committed broker-operations.json pins a fixture row; U23 has no prior findings, so no ledger items to honor.
