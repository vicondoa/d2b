//! Provider-neutral Process conformance contracts and shared suite.
//!
//! Every Process Provider (`system-systemd`, `system-minijail`, and any
//! future one) implements the same ResourceTypes and the same status and
//! error conformance. This crate owns the neutral half of that contract:
//! the launch ticket, the effect-port seam, the identity and pidfd
//! evidence types, the public status projection, and the shared suite both
//! Provider crates run.
//!
//! What this crate deliberately does not do, because
//! `ADR-046-components-processes-and-sandbox` forbids it for a Provider:
//! it performs no privileged mutation, opens no systemd, D-Bus, or broker
//! socket, resolves no host path, and issues no `clone3` or `pidfd_open`.
//! A Provider validates semantics and calls the injected
//! [`ProcessLaunchEffectPort`]; the fixed core effect adapter
//! (ProviderSupervisor) alone maps that call onto the broker or the systemd
//! effect owner, and the broker remains the sole privileged executor and
//! audit owner.
//!
//! No raw UID or GID, executable path, cgroup path, unit name, argv,
//! environment, socket address, or credential byte appears in any type
//! here. Identity travels as opaque digests and typed resource references.

#![deny(missing_docs)]

mod error;
mod identity;
mod port;
mod provider;
mod sandbox;
mod status;
mod terminal;
mod ticket;

pub mod process_provider;
pub mod suite;
pub mod testing;

pub use error::ProcessConformanceError;
pub use identity::{
    ConfigurationDigest, IdentityBinding, ObservedIdentity, PidfdEvidence, ProcessIdentityDigest,
    WaitReapOwner,
};
pub use port::{AdoptionCandidate, LaunchedProcess, ProcessLaunchEffectPort, StopClass};
pub use provider::{AdoptionOutcome, ProcessProvider, ProcessProviderProfile};
pub use sandbox::{CompiledSandbox, SandboxCompiler, StopProof, validate_stop_proof};
pub use status::{
    AdoptionCondition, ExitClass, ExitObservation, ProcessPhaseClass, ProcessStatusReport,
};
pub use terminal::ExitClass as ProcessExitClass;
pub use terminal::{
    BrokerTerminalResult, ExitClass as BrokerExitClass, ParentWaitEvidence, ProcessOutcome,
};
pub use ticket::{
    CancellationBinding, CompiledDigests, InheritedFdTable, LaunchTicket, MAX_INHERITED_FDS,
    MAX_LAUNCH_DEADLINE_MS, OperationBinding, ReadinessExpectation,
};
