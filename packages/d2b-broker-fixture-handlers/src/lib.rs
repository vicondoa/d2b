//! Pure fixture operations for the broker handler injection seam (U6).
//!
//! The composition root links handler crates and registers their declared
//! operations on the broker's handler table. This crate is the fixture
//! stand-in for a provider handler crate: it declares operations that are
//! pure, reviewable transforms over the invocation's capability object, and
//! nothing else. No census operation maps to the in-broker leg this pass
//! (KTD1), so this crate is the seam's only linked handler source.
//!
//! The dependency-surface audit (`d2b_broker_composition::dependency_surface`)
//! treats this crate as a clean baseline: no syscall-surface dependency, no
//! raw-syscall code, no build-emitted or runtime entry machinery.

#![deny(unsafe_code)]

use d2b_broker::envelope::{DirectInvocation, DispatchFailure, DispatchOutcome};

/// The fixture operation serving a pure echo transform.
///
/// The handler echoes the validated payload back unmodified; the envelope's
/// audit identity derives from the invocation id the broker minted, exactly
/// as it does for a forwarded operation's record.
pub const PURE_ECHO: &str = "d2b.fixture.pure.echo";

/// One declared operation: its committed name and its handler.
pub struct DeclaredOperation {
    /// The operation name (the committed row it serves).
    pub operation: &'static str,
    /// The pure handler, typed over the capability object the broker hands
    /// an in-broker handler: the invocation context, the validated payload,
    /// the attested context block, and the attached descriptors. Broker
    /// internals are unnameable from this crate.
    pub handler: fn(&DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure>,
}

/// Every operation this fixture handler crate declares.
pub const OPERATIONS: &[DeclaredOperation] = &[DeclaredOperation {
    operation: PURE_ECHO,
    handler: echo,
}];

/// The declared-operation table the composition root registers.
pub fn declared_operations() -> &'static [DeclaredOperation] {
    OPERATIONS
}

/// Echo the validated payload back as the result.
pub fn echo(invocation: &DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> {
    Ok(DispatchOutcome {
        result: invocation.payload.clone(),
        fds: Vec::new(),
    })
}