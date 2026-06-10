//! Broker operation handlers.
//!
//! Every handler in this module follows the broker contract:
//!
//! - re-derives every operating path from the trusted bundle, never
//!   from caller input;
//! - emits an audit record per the schema in
//!   `docs/reference/cgroup-delegation.md` § "Audit records" and the
//!   per-variant fields;
//! - returns a typed `OpError` that maps cleanly to the wire-level
//!   `BrokerResponse` shape used by `runtime::dispatch_request`.
//!
//! The integrator wires these handlers into `runtime::dispatch_request`
//! in a separate commit. Scopes keep the integration surface one-way:
//! these modules depend on `nixling-host` and `nixling-ipc`, but
//! nothing in the runtime depends on them beyond the integrator-managed
//! dispatch wiring.

// Cgroup v2 delegation + pidfd handoff ops.
pub mod cgroup;
pub mod pidfd;
// Bridge / TAP / NM / IPv6 / IfName / state-dir ops.
pub mod hosts;
pub mod nm;
pub mod route;
pub mod state_dir;
pub mod sysctl;
pub mod tap;
// Nftables + USBIP firewall skeleton ops.
pub mod nft;
pub mod usbip_firewall;
// Per-busid USBIP exclusivity lock helper.
pub mod usbip_lock;

// Kernel-module + device-fd handoff ops.
pub mod device;
pub mod modprobe;
// Broker SpawnRunner preflight + spawn helper.
pub mod spawn_runner;
// Broker reconcile executors (nft / sysctl / hosts / ip route) with
// FakeReconcileExecutor for unit tests + the SystemReconcileExecutor
// for production shellouts.
pub mod exec_reconcile;

// Audit-helper introduced by s2; reusable by s1/s3/s4 going forward.
pub mod audit_op;

// Typed broker op that hardlink-farms per-VM closures into
// `/var/lib/nixling/vms/<vm>/store/` and atomically swaps the `current`
// symlink. Replaces the `nixling-<vm>-store-sync.service` bash oneshot.
pub mod store_sync;

// Signed ADR 0027 terminal audit schema for `StoreSync` (enums +
// invariant-enforcing constructors + validation).
pub mod store_sync_audit;

// StoreSync-only observability JSONL export: a positive-allow-list
// projection of the host-confidential `StoreSync` terminal audit record
// (ADR 0027). Written to the alloy-readable export directory; never
// carries caller identity, retained generations, or any host path.
pub mod store_sync_export;

// Explicit StoreVerify operator surface for top-level live-pool
// verification + host-only integrity state.
pub mod store_verify;

// Single-inode ownership/mode posture for broker-created store-view
// metadata paths. Never recursive into the hardlinked live pool.
pub mod store_view_posture;

// Out-of-process, mount-namespace-isolated store-view hardlink farm
// build. Used by `store_sync` and `exec_reconcile::prepare_store_view`
// so the farm hardlinks succeed even when `/nix/store` is a separate
// (bind) mount from `/var/lib/nixling`.
pub mod store_view_farm;

// Per-VM writable store overlay disk-image provisioning. Runs before
// SpawnRunner when `DiskInit` plan-ops are present.
pub mod disk_init;

use std::fmt;
use std::path::PathBuf;

/// Common error shape for broker handlers.
///
/// Future submodules add their typed sub-errors here as new `OpError::*`
/// variants so the runtime dispatch layer can map every audited handler
/// outcome onto the wire-level `BrokerResponse`.
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
/// schema.
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
