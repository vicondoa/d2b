//! The ExecutionPolicy provider crate: the `ExecutionPolicy` resource type's
//! driver declaration and the decode boundary its rows are read through.
//!
//! An `ExecutionPolicy` row states reusable confinement: the namespace
//! classes an instance must run behind, the capability ceiling, privilege and
//! root restrictions, the identity rules, the selected syscall filter, and
//! the umask. It states no host path, no mount, no device node, and no
//! resource reference other than the identity and syscall-filter rows it
//! selects, so the row cannot become a second, independently granted access
//! list beside the typed binding relationships that own attachment.
//!
//! The crate owns two facts and no schema:
//!
//! - the type's identity, as the `DriverDescriptor` the plane registers it by
//!   (`execution_policy_descriptor`), and
//! - the decode boundary a committed row is read through
//!   (`decode_policy_row`).
//!
//! The desired spec itself is the canonical `ExecutionPolicySpec` in
//! `d2b-contracts-resource`, and the decision is `admit_execution`, the one
//! pure evaluator in the same crate. Neither is restated here: a provider
//! crate that carried its own copy of either could drift from the contract
//! the graph and the broker both read, and would then be a second source of
//! confinement truth.
//!
//! Registering the type - the standard ResourceType catalog entry, the
//! generated type authority, and the daemon composition root - is the cutover
//! unit's change. Until that lands, an `ExecutionPolicy/<name>` reference
//! cannot be constructed at all, which is the correct shape for a type no
//! consumer may select yet.

#![deny(missing_docs)]

mod driver;

pub use driver::{PolicyRowError, decode_policy_row, execution_policy_descriptor};
