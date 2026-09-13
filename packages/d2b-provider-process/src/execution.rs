//! The execution targets a Process row may be reconciled in.
//!
//! A row's declared `executionRef` names where it runs; the daemon mode it is
//! reconciled under decides which of those targets this process can actually
//! drive. Nothing else in the family reads the mode.

use d2b_contracts_resource::v3::ResourceRef;
use d2bd_runtime::target_runtime::DaemonMode;

/// Whether one execution reference is admissible in the given daemon mode.
///
/// A Host daemon drives Host-executing rows only; a Guest daemon drives
/// Guest-executing rows only. A reference outside that pair is refused before
/// any effect runs.
pub fn execution_target_allowed(mode: DaemonMode, execution_ref: &ResourceRef) -> bool {
    match mode {
        DaemonMode::Host => execution_ref.resource_type().as_str() == "Host",
        DaemonMode::Guest => execution_ref.resource_type().as_str() == "Guest",
    }
}
