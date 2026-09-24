//! Pure fixture operations for the broker handler injection seam (U6).
//!
//! The composition root links handler crates by qualified path and registers
//! the handler directly; this crate is the fixture stand-in for a provider
//! handler crate, a pure echo handler and nothing else. No census operation
//! maps to the in-broker leg this pass (KTD1), so this crate is the seam's
//! only linked handler source.
//!
//! The dependency-surface audit (`d2b_broker_composition::dependency_surface`)
//! treats this crate as a clean baseline: no syscall-surface dependency, no
//! raw-syscall code, no build-emitted or runtime entry machinery.

#![deny(unsafe_code)]

use d2b_broker::envelope::{DirectInvocation, DispatchOutcome, HandlerFuture};

/// Echo the validated payload back as the result.
pub fn echo<'a>(invocation: &'a DirectInvocation<'a>) -> HandlerFuture<'a> {
    Box::pin(async move {
        Ok(DispatchOutcome {
            result: invocation.payload.clone(),
            fds: Vec::new(),
        })
    })
}
