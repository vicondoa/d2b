//! Minijail finalization ordering through the typed effect port.

use d2b_process_conformance::{
    ProcessConformanceError, StopProof, WaitReapOwner, validate_stop_proof,
};

/// Validate the broker-parent stop and cgroup-empty proofs.
pub fn validate_finalization(proof: StopProof) -> Result<(), ProcessConformanceError> {
    validate_stop_proof(WaitReapOwner::Local, proof)
}
