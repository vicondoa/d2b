//! The execution targets a Process row may be reconciled in.
//!
//! A row's declared `executionRef` names where it runs; the execution domain
//! the driver was constructed under decides which of those targets this
//! process can actually drive. Nothing else in the family reads the domain:
//! the daemon maps its own mode onto it at the boundary.

use d2b_contracts_resource::v3::ResourceRef;

/// The Host/Guest execution domain a Process driver admits.
///
/// The family declares this vocabulary itself; the daemon maps its own mode
/// onto it where it builds the driver arguments, so the family never depends
/// on the daemon's runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    /// The driver runs on the Host and drives Host-executing rows only.
    Host,
    /// The driver runs in the Guest and drives Guest-executing rows only.
    Guest,
}

/// Whether one execution reference is admissible in the given execution
/// domain.
///
/// A Host driver drives Host-executing rows only; a Guest driver drives
/// Guest-executing rows only. A reference outside that pair is refused before
/// any effect runs.
pub fn execution_target_allowed(mode: ExecutionMode, execution_ref: &ResourceRef) -> bool {
    match mode {
        ExecutionMode::Host => execution_ref.resource_type().as_str() == "Host",
        ExecutionMode::Guest => execution_ref.resource_type().as_str() == "Guest",
    }
}
