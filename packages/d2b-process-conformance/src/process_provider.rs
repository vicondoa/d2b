//! Stable Process Provider contract namespace.
//!
//! The crate originally split these values into small implementation modules
//! so the conformance suite could evolve independently.  This public
//! namespace is the destination-compatible boundary for Provider crates and
//! keeps all launch, adoption, pidfd, and terminal-result types on one
//! documented surface.

pub use crate::{
    AdoptionCandidate, BrokerTerminalResult, CancellationBinding, ConfigurationDigest,
    IdentityBinding, InheritedFdTable, LaunchTicket, OperationBinding, ParentWaitEvidence,
    PidfdEvidence, ProcessExitClass, ProcessIdentityDigest, ProcessOutcome, ReadinessExpectation,
    WaitReapOwner,
};
