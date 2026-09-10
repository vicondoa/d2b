//! Runtime error taxonomy shared by actors, drivers, and the store (U4).
//!
//! Two layers:
//!
//! - [`ResourceError`]: the cross-surface taxonomy (spec decode, store,
//!   manager RPC, driver failures, deleting conflict). Store errors wrap
//!   verbatim except [`SpecStoreError::ResourceDeleting`], which is re-mapped
//!   to [`ResourceError::DeletingConflict`] so callers handle the deleting
//!   conflict once, by one variant (R10).
//! - [`DriverFailure`]: the closed, redacted classification of driver
//!   failures (pattern: `HandlerFailure` in
//!   `packages/d2b-controller-toolkit/src/runner.rs:400-437`). Drivers map
//!   their typed errors onto it at the erased driver boundary; the actor owns
//!   retry/backoff from the closed class alone (R13: retry state is
//!   runtime-only), and rich provider error text never crosses the contract.

pub const MODULE_NAME: &str = "error";

use crate::identity::ResourceKey;
use crate::spec_store::SpecStoreError;

/// Which driver operation failed. Closed: no provider detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverOp {
    Validate,
    Recover,
    Reconcile,
    Delete,
}

impl DriverOp {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::Recover => "recover",
            Self::Reconcile => "reconcile",
            Self::Delete => "delete",
        }
    }
}

impl std::fmt::Display for DriverOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Closed retry class (mirrors `HandlerErrorClass` in the old runner).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// Transient: the actor requeues with backoff (runtime-only, R13).
    Retryable,
    /// Terminal: no retry; the actor surfaces the failure and stops.
    Terminal,
}

impl FailureClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Retryable => "retryable",
            Self::Terminal => "terminal",
        }
    }
}

impl std::fmt::Display for FailureClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Closed, redacted classification of one driver failure.
///
/// Constructed only by [`crate::driver::ResourceDriver::classify_error`] and
/// reported through the erased driver boundary; the display carries the
/// operation and the retry class and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("driver {op} failed ({class})")]
pub struct DriverFailure {
    class: FailureClass,
    op: DriverOp,
}

impl DriverFailure {
    /// Classify the failure as retryable for the given operation.
    pub const fn retryable(op: DriverOp) -> Self {
        Self { class: FailureClass::Retryable, op }
    }

    /// Classify the failure as terminal for the given operation.
    pub const fn terminal(op: DriverOp) -> Self {
        Self { class: FailureClass::Terminal, op }
    }

    /// Return the closed retry class.
    pub const fn class(self) -> FailureClass {
        self.class
    }

    /// Return the operation that failed.
    pub const fn op(self) -> DriverOp {
        self.op
    }
}

/// Cross-surface runtime error taxonomy.
#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    /// The stored spec envelope could not be decoded into the concrete
    /// driver spec type: the wired decode hook failed, or the decoded type
    /// did not match the requested one.
    #[error("spec decode for {key}: {source}")]
    SpecDecode {
        key: ResourceKey,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Spec store failure, wrapped verbatim (the manager translates store
    /// results for drivers; `ResourceDeleting` never reaches this variant).
    #[error(transparent)]
    Store(SpecStoreError),
    /// A call routed to the manager actor failed: its channel closed, the
    /// request was dropped, or the manager rejected it.
    #[error("manager rpc: {0}")]
    ManagerRpc(String),
    /// A driver reported a failure through the closed classification.
    #[error(transparent)]
    Driver(#[from] DriverFailure),
    /// Operation against a resource whose durable `deleting` mark is set
    /// (R10). Mapped from `SpecStoreError::ResourceDeleting`.
    #[error("resource {zone}/{type_name}/{name} is marked deleting")]
    DeletingConflict {
        zone: String,
        type_name: String,
        name: String,
    },
}

impl From<SpecStoreError> for ResourceError {
    fn from(error: SpecStoreError) -> Self {
        match error {
            SpecStoreError::ResourceDeleting { zone, type_name, name } => {
                Self::DeletingConflict { zone, type_name, name }
            }
            other => Self::Store(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleting_store_error_maps_to_deleting_conflict() {
        let error: ResourceError = SpecStoreError::ResourceDeleting {
            zone: "z".into(),
            type_name: "Volume".into(),
            name: "data".into(),
        }
        .into();
        match error {
            ResourceError::DeletingConflict { zone, type_name, name } => {
                assert_eq!(zone, "z");
                assert_eq!(type_name, "Volume");
                assert_eq!(name, "data");
            }
            other => panic!("unexpected mapping: {other:?}"),
        }
    }

    #[test]
    fn other_store_errors_wrap_as_store() {
        let error: ResourceError = SpecStoreError::NotFound {
            zone: "z".into(),
            type_name: "Volume".into(),
            name: "data".into(),
        }
        .into();
        assert!(matches!(error, ResourceError::Store(_)));
    }

    #[test]
    fn driver_failure_is_closed_and_redacted() {
        let retryable = DriverFailure::retryable(DriverOp::Reconcile);
        assert_eq!(retryable.class(), FailureClass::Retryable);
        assert_eq!(retryable.op(), DriverOp::Reconcile);
        let terminal = DriverFailure::terminal(DriverOp::Delete);
        assert_eq!(terminal.class(), FailureClass::Terminal);
        assert_eq!(terminal.op(), DriverOp::Delete);
        // Redacted: the display carries only operation + class, never the
        // underlying provider error detail.
        assert_eq!(retryable.to_string(), "driver reconcile failed (retryable)");
        // Through the taxonomy the closed classification is preserved.
        let error: ResourceError = terminal.into();
        assert!(matches!(error, ResourceError::Driver(f) if f.class() == FailureClass::Terminal));
    }
}