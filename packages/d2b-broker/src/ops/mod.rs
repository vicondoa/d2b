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
//! these modules depend on `d2b-host` and `d2b-contracts`, but
//! nothing in the runtime depends on them beyond the integrator-managed
//! dispatch wiring.
//!
//! Only four of the arms below are `pub` - `network`, `audit_op`, `pidfd`,
//! and `endpoint_access` - because the crate's integration tests are
//! separate crates and address those arms by path
//! (`tests/bridge_lifecycle.rs`, `tests/persistent_tap_lifecycle.rs`,
//! `tests/security_key_broker.rs`, `tests/pidfd_handoff_scm_rights.rs`,
//! `tests/pidfd_real_spawner.rs`, `tests/endpoint_delivery.rs`);
//! `pub(crate)` would hide the arms from those oracles. Every other arm is
//! `pub(crate)`: no crate outside `d2b-broker` imports it, so its items
//! were reachable published surface for no consumer. Keep the split -
//! a new handler arm stays `pub(crate)` until something outside this
//! crate imports it, and the integration tests are the only thing that
//! reopens an arm.

// Cgroup v2 delegation + pidfd handoff ops.
pub(crate) mod cgroup;
// Public arm: `tests/pidfd_handoff_scm_rights.rs` and
// `tests/pidfd_real_spawner.rs` import it from outside the crate.
pub mod pidfd;
// Bridge / TAP / NM / IPv6 / IfName / state-dir ops.
pub(crate) mod hosts;
pub(crate) mod nm;
pub(crate) mod route;
pub(crate) mod state_dir;
pub(crate) mod storage_contract;
// Trusted identity + derived directories of one `w1-swtpm` launch.
pub(crate) mod swtpm_identity;
pub(crate) mod sysctl;
pub(crate) mod tap;
// Nftables + USBIP firewall skeleton ops.
// Public arm: `tests/bridge_lifecycle.rs` imports it from outside the
// crate.
pub mod network;
pub(crate) mod nft;
pub(crate) mod usbip_firewall;
// Per-busid USBIP exclusivity lock helper.
pub(crate) mod usbip_lock;
// Broker-side USBIP host inspection and physical-policy enforcement.
pub(crate) mod usbip_host;

// Kernel-module + device-fd handoff ops.
pub(crate) mod device;
// Trusted scope of one Device-owned worker launch (row -> Device -> Guest
// pin, per-Guest socket directory, Device row uid derivation).
pub(crate) mod device_worker;
pub(crate) mod consumer_principal;
// Exact-endpoint ACL wire (U18, R23): the private resolution of a socket name
// inside the broker's own runtime root, the consumer-principal derivation, and
// the pinned observe/grant/revoke answer.
// Public arm: `tests/endpoint_delivery.rs` drives the accept path from outside
// the crate, for the same reason the arms above are public.
pub mod endpoint_access;
// Broker-owned host-path bounds for the ACL grants that reach outside the
// broker runtime tree (served view roots, host session runtime directory).
pub(crate) mod launch_acl_bounds;
// GPU-specific role, allowlist, and restart identity preflight.
pub(crate) mod gpu;
pub(crate) mod modprobe;
// Security-key hidraw open op: resolves stable selector → opens
// hidraw fd for `d2bd`'s long-lived CTAPHID relay session.
pub(crate) mod security_key;
// Broker SpawnRunner preflight + spawn helper, and the effective Volume
// presentation (U11, KTD11) it resolves.
// Public arm: `tests/volume_presentation.rs` imports it from outside the
// crate, for the same reason the arms above are public.
pub mod spawn_runner;
// The broker's own authority for the private execution values an admitted
// effect resolves to, read from the verified private bundle.
pub(crate) mod private_execution;
// Broker reconcile executors (nft / sysctl / hosts / ip route) with
// FakeReconcileExecutor for unit tests + the SystemReconcileExecutor
// for production shellouts.
pub(crate) mod exec_reconcile;

// Audit-helper introduced by s2; reusable by s1/s3/s4 going forward.
// Public arm: `tests/persistent_tap_lifecycle.rs` and
// `tests/security_key_broker.rs` import it from outside the crate.
pub mod audit_op;
// Broker-owned source-to-target NixOS generation handoff journal and replay.
pub(crate) mod host_generation_handoff;
// The one-shot ownership-bounded reset runner (U32, KTD15): the offline
// `d2b host reset` path that removes the previous release's host state
// without opening its SpecStore and without a running daemon.
// Public arm: `tests/owned_reset.rs` imports it from outside the crate.
pub mod host_reset;

// Typed broker op that hardlink-farms per-VM closures into
// `/var/lib/d2b/vms/<vm>/store/` and atomically swaps the `current`
// symlink. Replaces the `d2b-<vm>-store-sync.service` bash oneshot.
// Public so the binding-scoped publication can be driven from the
// owning test target rather than from a second copy of the policy.
pub mod store_sync;

// Signed ADR 0027 terminal audit schema for `StoreSync` (enums +
// invariant-enforcing constructors + validation).
pub(crate) mod store_sync_audit;

// StoreSync-only observability JSONL export: a positive-allow-list
// projection of the host-confidential `StoreSync` terminal audit record
// (ADR 0027). Written to the alloy-readable export directory; never
// carries caller identity, retained generations, or any host path.
pub(crate) mod store_sync_export;

// Single-inode ownership/mode posture for broker-created store-view
// metadata paths. Never recursive into the hardlinked live pool.
pub(crate) mod store_view_posture;

// Out-of-process, mount-namespace-isolated store-view hardlink farm
// build. Used by `store_sync` so the farm hardlinks succeed even when
// `/nix/store` is a separate (bind) mount from `/var/lib/d2b`. Public
// so the admitted-export admission and the read-only build it guards
// are exercised where the mutation rule is observable.
pub mod store_view_farm;

// Per-VM writable store overlay disk-image provisioning. Runs before
// SpawnRunner when `DiskInit` plan-ops are present.
pub(crate) mod disk_init;

// qemu-media physical USB enrollment/open by opaque ref. Raw device identity
// stays in root-only registry/runtime artifacts outside the Nix store.
pub(crate) mod media;
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
    /// The audit-record `decision` spelling of this category.
    pub fn as_str(&self) -> &'static str {
        match self {
            AuditDecision::Allowed => "allowed",
            AuditDecision::DeniedRefused => "denied-refused",
            AuditDecision::DeniedUnknown => "denied-unknown",
            AuditDecision::Errored => "errored",
        }
    }
}
