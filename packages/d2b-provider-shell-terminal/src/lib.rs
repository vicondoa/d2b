//! Resource, controller, and supervisor contracts for `Provider/shell-terminal`.
//!
//! The Provider owns policy-backed shell pools and sessions.  It does not own
//! an ambient host shell, raw broker connection, or persistent controller
//! state.  Individual session supervisors own PTYs and run as the pool's
//! workload user.

#![deny(missing_docs)]

mod authz;
mod guest_rules;
mod host_rules;
mod migration;
mod observability;
mod process_lifecycle;
mod process_templates;
mod resources;
mod service;
mod session;

pub use authz::{Authorizer, CallerOrigin, Role, Subject};
pub use guest_rules::{GuestPlacement, validate_guest_placement};
pub use host_rules::{HostPlacement, IsolationPosture, validate_host_placement};
pub use migration::{MigrationDisposition, ProviderStateSet};
pub use observability::{DiagnosticAccumulator, DiagnosticKind, ExecutionKind, ShellMetrics};
pub use process_lifecycle::SupervisorProcessLifecycle;
pub use process_templates::{ProcessTemplate, TemplateDomain};
pub use resources::{
    DEFAULT_MAX_ATTACHED, DEFAULT_MAX_SESSIONS, DEFAULT_OUTPUT_RING_CAPACITY, ExecutionTarget,
    PoolSpec, SessionPhase, ShellPool, ShellSession, ShellTerminalError,
};
pub use service::{
    AttachReceipt, AttachRequest, Attachment, CONTROLLER_SERVICE, InMemoryShellAuthority,
    OpenSessionRequest, OpenSessionResult, SUPERVISOR_SERVICE, SessionCapability,
    SessionSupervisor, ShellAuthorityPort, ShellTerminalController, TERMINAL_STREAM,
};
pub use session::{
    AdoptionDecision, OutputRing, SupervisorCandidate, SupervisorIdentity, adopt_supervisor,
};
