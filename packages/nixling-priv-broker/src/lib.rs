#![deny(unsafe_code)]
// v1.2: workspace clippy lints inherited; some structural patterns warrant
// crate-level allows here to avoid churning broker-internal code that is
// otherwise correct and well-tested:
//
// - `deprecated`: cgroup vm_leaf_path migration is tracked but the deprecated
//   path is still referenced in legacy code paths kept for v1.1.x compat.
// - `clippy::dead_code`: helper functions (e.g. apply_mount_actions, apply)
//   are public API of internal modules that downstream callers may use.
// - `clippy::large_enum_variant`, `clippy::result_large_err`: TypedError
//   variants intentionally carry rich context; boxing tracked separately.
// - `clippy::too_many_arguments`: broker spawn pipeline has wide signatures
//   for safety (forgetting an arg = sandbox bypass).
// - `clippy::needless_borrows_for_generic_args`, `clippy::cmp_owned`,
//   `clippy::io_other`, `clippy::needless_borrow`: stylistic.
#![allow(deprecated)]
#![allow(dead_code)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::result_large_err)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_borrows_for_generic_args)]
#![allow(clippy::cmp_owned)]
#![allow(clippy::io_other_error)]
#![allow(clippy::needless_borrow)]

// The non-bootstrap runtime path is supported and wires against the
// real opaque-ID `nixling_contracts::broker_wire::BrokerRequest` contract via
// the `live_handlers` module. The bootstrap path remains available
// behind the `layer1-bootstrap` feature for the legacy probe-hello /
// probe-stub / probe-export-audit test harnesses; new code should
// target the real wire.
//
// `tests/broker-default-features-build.sh` was updated to
// reflect this clean break (default features now empty); the
// no-default-features gate at `tests/broker-no-default-features.sh`
// asserts the production binary compiles clean.

pub mod audit;
pub mod fd_passing;
// Live broker request handlers (pidfd_open + clone3-based spawn +
// reconcile-executor calls). Pure-shaped: take their inputs directly so
// the dispatch layer is the only mixer of wire decoding + bundle
// resolution + live execution.
pub mod live_handlers;
pub mod ops;
pub mod protocol;
pub mod runtime;
pub mod sys;

// Behavioral + regression seccomp BPF tests.
#[cfg(test)]
mod seccomp_compile_tests;

#[cfg(feature = "layer1-bootstrap")]
pub mod bootstrap;
