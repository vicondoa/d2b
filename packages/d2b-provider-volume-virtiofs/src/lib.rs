//! The volume-virtiofs attachment Provider.
//!
//! `Provider/volume-virtiofs` reconciles `VolumeBinding` resources, not
//! Volume resources. For each binding it ensures exactly one
//! binding-owned virtiofsd worker Process and one stable Endpoint,
//! observes their readiness, and writes the fenced binding status
//! projection (KTD3). volume-local alone reads that status and writes
//! the aggregated Volume attachment status.
//!
//! What this crate deliberately does not do, because
//! `ADR-046-resources-volume` forbids it: it never writes a Volume row,
//! never performs a privileged mutation, never spawns a process, never
//! binds a socket, and never resolves a host path. It calls the injected
//! [`VirtiofsBindingEffectPort`]; ProviderSupervisor alone maps that call
//! onto the broker.
//!
//! The virtiofsd sandbox posture is frozen by ADR 0021 and is asserted
//! before any launch: zero host capabilities, no start as root, a chroot
//! sandbox, a read-only root, and `--inode-file-handles=never`. There is
//! no free-form virtiofsd argument channel.
//!
//! The binding socket path is a generated private implementation detail.
//! Only its opaque identity is public; the path never appears in a spec,
//! a status field, an audit record, or CLI output.

#![deny(missing_docs)]

/// The canonical `Provider/<name>` reference this Provider owns.
///
/// The VolumeBinding rows this Provider serves select it, and the
/// binding-owned worker Process and its Endpoint are minted under it, so the
/// declaring crates read the reference instead of spelling it again.
pub const PROVIDER_REF: &str = "Provider/volume-virtiofs";

mod controller;
mod error;
mod bindings;
mod port;
mod socket_path;
mod worker;

pub mod testing;

pub use controller::{
    VirtiofsBindingController, VirtiofsRunnerContract, binding_phase, resolve_view,
    virtiofs_runner_contract,
};
pub use error::VirtiofsBindingError;
pub use bindings::{
    VOLUME_BINDING_FINALIZER, VOLUME_BINDING_RESOURCE_TYPE, SocketIdentity, StoredBinding,
};
pub use port::{
    BindingPhase, BindingStatusReport, LaunchedWorker, VirtiofsBindingEffectPort,
};
pub use socket_path::MAX_SOCKET_PATH_BYTES;
pub use worker::{
    INODE_FILE_HANDLES, SANDBOX_MODE, USER_NAMESPACE_MAPPING_CLASS, VirtiofsdWorkerPlan,
    WORKER_TEMPLATE, WorkerSandbox,
};
