//! W3 broker operation handlers.
//!
//! Each W3 scope agent owns a disjoint subset of files under this
//! directory. Section markers below (`// W3 sN begin` / `// W3 sN end`)
//! identify ownership boundaries so parallel scope commits do not
//! collide with each other or with future hardening rounds.
//!
//! Every handler in this module follows the contract from plan.md
//! §"W3 broker variant additions":
//!
//! - re-derives every operating path from the trusted bundle, never
//!   from caller input;
//! - emits an audit record per the schema in
//!   `docs/reference/cgroup-delegation.md` § "Audit records" and the
//!   plan-named per-variant fields;
//! - returns a typed `OpError` that maps cleanly to the wire-level
//!   `BrokerResponse` shape used by `runtime::dispatch_request`.
//!
//! The integrator wires these handlers into `runtime::dispatch_request`
//! in a separate commit. Scopes keep the integration surface one-way:
//! these modules depend on `nixling-host` and `nixling-ipc`, but
//! nothing in the runtime depends on them beyond the integrator-managed
//! dispatch wiring.

// W3 s1 begin — cgroup v2 delegation + pidfd handoff ops.
pub mod cgroup;
pub mod pidfd;
// W3 s1 end.

// W3 s2 begin — bridge / TAP / NM / IPv6 / IfName / state-dir ops.
pub mod hosts;
pub mod nm;
pub mod route;
pub mod state_dir;
pub mod sysctl;
pub mod tap;
// W3 s2 end.

// W3 s3 begin — nftables + USBIP firewall skeleton ops.
pub mod nft;
pub mod usbip_firewall;
// W3 s3 end.

// W13 (W6-fu) — per-busid USBIP exclusivity lock helper.
pub mod usbip_lock;

// W3 s4 begin — kernel-module + device-fd handoff ops.
pub mod device;
pub mod modprobe;
// W3 s4 end.

// W4-fu — broker SpawnRunner preflight + spawn helper.
pub mod spawn_runner;
// W4-fu — broker reconcile executors (nft / sysctl / hosts /
// ip route) with FakeReconcileExecutor for unit tests + the
// SystemReconcileExecutor for production shellouts.
pub mod exec_reconcile;

// Audit-helper introduced by s2; reusable by s1/s3/s4 going forward.
pub mod audit_op;

// P2 ph2-store-sync — typed broker op that hardlink-farms per-VM
// closures into `/var/lib/nixling/vms/<vm>/store/` and atomically
// swaps the `current` symlink. Replaces the
// `nixling-<vm>-store-sync.service` bash oneshot.
pub mod store_sync;

use std::fmt;
use std::path::PathBuf;

/// Common error shape for W3 broker handlers.
///
/// Introduced by s1; future scopes add their typed sub-errors here as
/// new `OpError::*` variants (one per scope) so the runtime dispatch
/// layer can map every audited handler outcome onto the wire-level
/// `BrokerResponse`.
#[derive(Debug)]
pub enum OpError {
    /// The caller asked for a subject/scope absent from the trusted
    /// bundle. Audited with `defaultForUnknown: deny`.
    UnknownSubject {
        operation: &'static str,
        subject: String,
    },
    /// Path-safety violation (symlink swap, foreign-owned parent,
    /// world-writable parent, etc.).
    PathSafetyViolation {
        operation: &'static str,
        detail: String,
    },
    /// Requested operation is structurally invalid.
    InvalidInput { detail: String },
    /// Requested operation is denied by bundle policy.
    Refused {
        operation: &'static str,
        reason: String,
    },
    /// I/O failed while accessing a host path.
    Io { path: PathBuf, detail: String },
    /// Audited cgroup-specific error (see [`cgroup::CgroupOpError`]).
    Cgroup(cgroup::CgroupOpError),
    /// Audited pidfd-specific error.
    Pidfd(pidfd::PidfdOpError),
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpError::UnknownSubject { operation, subject } => {
                write!(f, "{operation}: unknown subject {subject:?}")
            }
            OpError::PathSafetyViolation { operation, detail } => {
                write!(f, "{operation}: path-safety-violation: {detail}")
            }
            OpError::InvalidInput { detail } => write!(f, "invalid input: {detail}"),
            OpError::Refused { operation, reason } => write!(f, "{operation}: refused: {reason}"),
            OpError::Io { path, detail } => write!(f, "I/O error on {}: {detail}", path.display()),
            OpError::Cgroup(err) => write!(f, "cgroup-op: {err}"),
            OpError::Pidfd(err) => write!(f, "pidfd-op: {err}"),
        }
    }
}

impl std::error::Error for OpError {}

/// Audit decision categories used by the broker handlers. The variant
/// name maps 1:1 to the `decision` field in the broker audit record
/// schema (plan.md §"W3 broker variant additions" → audit baseline).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditDecision {
    Allowed,
    DeniedRefused,
    DeniedUnknown,
    Errored,
}

impl AuditDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuditDecision::Allowed => "allowed",
            AuditDecision::DeniedRefused => "denied-refused",
            AuditDecision::DeniedUnknown => "denied-unknown",
            AuditDecision::Errored => "errored",
        }
    }
}
