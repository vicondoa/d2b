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

mod controller;
mod error;
mod bindings;
mod port;
mod readiness;
mod socket_path;
mod user_ns;
mod virtiofsd_argv;
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
pub use readiness::{
    GuestMountObservation, SocketObservation, StoreViewMarkerObservation, classify_readiness,
    require_store_view_marker,
};
pub use socket_path::{MAX_SOCKET_PATH_BYTES, PrivateSocketPath, SocketPathError};
pub use user_ns::{
    CLONE_NEWNS_FLAG, CLONE_NEWUSER_FLAG, MappingStep, UserNamespaceError, UserNamespaceTemplate,
    validate_clone3_flags, validate_mapping_order,
};
pub use virtiofsd_argv::{
    SocketGroup, VirtiofsdArgvError, VirtiofsdArgvInput, VirtiofsdCacheMode,
    generate_virtiofsd_argv,
};
pub use worker::{
    INODE_FILE_HANDLES, SANDBOX_MODE, USER_NAMESPACE_MAPPING_CLASS, VirtiofsdWorkerPlan,
    WORKER_TEMPLATE, WorkerSandbox,
};
