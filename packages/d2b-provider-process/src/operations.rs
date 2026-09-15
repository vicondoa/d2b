//! The family's declared broker operations and their handlers.
//!
//! The Process family declares the ten process-family operations (U10): the
//! broker-generic kernels serve each operation's privileged, resource-agnostic
//! core in-broker as a committed row, while the family operation itself stays
//! forwarded to the declaring process. Each handler here validates the typed
//! request the envelope forwarded (the same wire shape the retired typed
//! broker arms consumed), resolves the trusted inputs it needs from the
//! Zone's bundle through the U10 family seam, and invokes the matching kernel
//! as a nested envelope call: the evidence chain the forwarded invocation runs
//! on (root invocation id plus ordered identities) is re-presented with the
//! handler's own caller identity appended, so the graft rule authorizes the
//! kernel call against the chain's initiating principal and the in-broker leg
//! records the correlation leg (KTD6).
//!
//! The handler table is the declaration itself: the descriptor this crate
//! publishes carries [`process_family_operations`], and the daemon's registry
//! serves the handlers from there. There is no second registration step.

use std::collections::BTreeSet;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_broker::broker_wire::{
    CgroupKillRequest, DeregisterRunnerPidfdRequest, DeregisterRunnerPidfdResponse,
    GuestExecutionBinding, ObserveRunnerRequest, ObserveRunnerResponse,
    OpenPeerPidfdFromAcceptedSocketRequest, OpenPeerPidfdFromAcceptedSocketResponse,
    OpenPidfdRequest, OpenPidfdResponse, PollChildReapedResponse, PrepareDirRequest,
    RunnerRole, RunnerSignal, SignalRunnerRequest, SignalRunnerResponse, SpawnRunnerRequest,
    SpawnRunnerResponse,
};
use d2b_contracts_broker::kernel_client::{
    KernelInvocation, KernelInvokeError, KernelReply, envelope_invoke_kernel,
};
use d2b_contracts_resource::v3::{
    CanonicalJsonObject, CanonicalJsonValue, ResourceRef, ResourceUid, canonical_json_bytes,
    execution_policy::ExecutionDomain,
};
use d2b_core::bundle_resolver::{
    BundleResolver, ResolvedRunnerIntent, is_device_worker_role,
};
use d2b_core::minijail_profile::CgroupPlacement;
use d2b_core::processes::ProcessRole;
use d2b_resource_types::{
    KernelCaller, OperationCtx, OperationDef, OperationFailure, OperationHandler,
    OperationResult, ValidatedPayload, WellKnownType,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::driver::{PROCESS_FAMILY_EXECUTION_DOMAINS, PROCESS_FAMILY_READS, PROCESS_FAMILY_VERBS};

/// The operation the family declares first.
///
/// The name is the committed row's name and the operation reference's own
/// name: a `ResourceRef` name is a lowercase label, and the broker names a
/// committed operation by exactly the string its declaring crate declares.
/// There is no second spelling to translate between.
pub const INSPECT_PROCESS_FAMILY: &str = "inspect-process-family";

/// The refusal of a payload that names no member type of this family.
pub const INVALID_PROCESS_TYPE: &str = "invalid-process-type";

/// The family's `OpenPidfd` operation (U10).
pub const OPEN_PIDFD: &str = "OpenPidfd";

/// The family's `OpenPeerPidfdFromAcceptedSocket` operation (U10).
pub const OPEN_PEER_PIDFD_FROM_ACCEPTED_SOCKET: &str = "OpenPeerPidfdFromAcceptedSocket";

/// The family's `ObserveRunner` operation (U10).
pub const OBSERVE_RUNNER: &str = "ObserveRunner";

/// The family's `PollChildReaped` operation (U10).
pub const POLL_CHILD_REAPED: &str = "PollChildReaped";

/// The family's `PrepareRuntimeDir` operation (U10).
pub const PREPARE_RUNTIME_DIR: &str = "PrepareRuntimeDir";

/// The family's `PrepareStateDir` operation (U10).
pub const PREPARE_STATE_DIR: &str = "PrepareStateDir";

/// The family's `CgroupKill` operation (U10).
pub const CGROUP_KILL: &str = "CgroupKill";

/// The family's `SignalRunner` operation (U10).
pub const SIGNAL_RUNNER: &str = "SignalRunner";

/// The family's `DeregisterRunnerPidfd` operation (U10).
pub const DEREGISTER_RUNNER_PIDFD: &str = "DeregisterRunnerPidfd";

/// The family's `SpawnRunner` operation (U10).
pub const SPAWN_RUNNER: &str = "SpawnRunner";

/// The refusal of a family operation whose kernel seam was never wired.
///
/// The composition point wires one [`KernelCaller`] per Zone alongside the
/// provider publication; a Zone whose seam was never wired serves forwarded
/// family operations without a kernel leg and every handler that needs one
/// refuses with this code.
pub const KERNEL_SEAM_UNWIRED: &str = "kernel-seam-unwired";

/// The refusal of a family operation whose nested kernel leg refused or
/// errored.
///
/// The kernel's own closed code ("errored" for a kernel error,
/// "handler-refused" for a kernel refusal) is preserved so the caller's
/// classification keeps working; the detail names the kernel and its reason.
pub const KERNEL_REFUSED: &str = "handler-refused";

/// The refusal of a typed request that disagrees with the trusted bundle.
///
/// Mirrors the retired broker arms' `SpawnRunnerIntentMismatch` posture: the
/// family handler resolves the trusted intent from the Zone's bundle and
/// refuses a request that names a different identity by field.
pub const INTENT_MISMATCH: &str = "handler-refused";

/// The refusal of a request that carries inherited descriptors the family
/// row's fd-less contract cannot transport (U10).
///
/// The family rows declare no fd facet, so a forwarded family call carries no
/// SCM_RIGHTS attachments; a `SpawnRunnerRequest` that declares inherited
/// descriptors (the retired ProviderController escrow contract) cannot be
/// served and is refused fail-closed rather than spawning a controller
/// without its bootstrap descriptor.
pub const INHERITED_FDS_UNSUPPORTED: &str = "handler-refused";

/// The refusal of a request whose typed identity is incomplete or invalid.
pub const PROCESS_IDENTITY_INVALID: &str = "handler-refused";

/// The refusal of a request naming a runner the daemon does not track.
pub const RUNNER_UNKNOWN: &str = "handler-refused";

/// The broker kernel IO budget one nested invocation may take.
const KERNEL_IO_TIMEOUT: Duration = Duration::from_secs(10);

/// The resource types the family's descriptors cover.
const MEMBER_TYPES: [WellKnownType; 2] = [WellKnownType::PROCESS, WellKnownType::EPHEMERAL_PROCESS];

/// The family's declared operations, assembled once.
///
/// The operation reference is an owned validated value, so the table is built
/// on first use rather than declared `const`; every descriptor the family
/// publishes shares the one table.
pub fn process_family_operations() -> &'static [OperationDef] {
    &PROCESS_FAMILY_OPERATIONS[..]
}

// The committed rows spell the family operations in the catalog's
// PascalCase wire names (`OpenPidfd`, `SpawnRunner`, ...), while a
// `ResourceRef` name is a lowercase label. The envelope matches the
// forwarded wire name case-insensitively (the U10 seam), so the table
// declares each operation under its parseable lowercase reference; the
// public constants above keep the catalog's wire spellings.
static PROCESS_FAMILY_OPERATIONS: LazyLock<[OperationDef; 11]> = LazyLock::new(|| {
    [
        OperationDef {
            operation_ref: ResourceRef::parse(&format!("Operation/{INSPECT_PROCESS_FAMILY}"))
                .expect("the family's operation reference is canonical"),
            handler: &INSPECT_PROCESS_FAMILY_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/open-pidfd")
                .expect("the family's operation reference is canonical"),
            handler: &OPEN_PIDFD_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/open-peer-pidfd-from-accepted-socket")
                .expect("the family's operation reference is canonical"),
            handler: &OPEN_PEER_PIDFD_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/observe-runner")
                .expect("the family's operation reference is canonical"),
            handler: &OBSERVE_RUNNER_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/poll-child-reaped")
                .expect("the family's operation reference is canonical"),
            handler: &POLL_CHILD_REAPED_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/prepare-runtime-dir")
                .expect("the family's operation reference is canonical"),
            handler: &PREPARE_RUNTIME_DIR_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/prepare-state-dir")
                .expect("the family's operation reference is canonical"),
            handler: &PREPARE_STATE_DIR_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/cgroup-kill")
                .expect("the family's operation reference is canonical"),
            handler: &CGROUP_KILL_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/signal-runner")
                .expect("the family's operation reference is canonical"),
            handler: &SIGNAL_RUNNER_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/deregister-runner-pidfd")
                .expect("the family's operation reference is canonical"),
            handler: &DEREGISTER_RUNNER_PIDFD_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/spawn-runner")
                .expect("the family's operation reference is canonical"),
            handler: &SPAWN_RUNNER_HANDLER,
        },
    ]
});

static INSPECT_PROCESS_FAMILY_HANDLER: InspectProcessFamilyHandler = InspectProcessFamilyHandler;
static OPEN_PIDFD_HANDLER: OpenPidfdHandler = OpenPidfdHandler;
static OPEN_PEER_PIDFD_HANDLER: OpenPeerPidfdHandler = OpenPeerPidfdHandler;
static OBSERVE_RUNNER_HANDLER: ObserveRunnerHandler = ObserveRunnerHandler;
static POLL_CHILD_REAPED_HANDLER: PollChildReapedHandler = PollChildReapedHandler;
static PREPARE_RUNTIME_DIR_HANDLER: PrepareRuntimeDirHandler = PrepareRuntimeDirHandler;
static PREPARE_STATE_DIR_HANDLER: PrepareStateDirHandler = PrepareStateDirHandler;
static CGROUP_KILL_HANDLER: CgroupKillHandler = CgroupKillHandler;
static SIGNAL_RUNNER_HANDLER: SignalRunnerHandler = SignalRunnerHandler;
static DEREGISTER_RUNNER_PIDFD_HANDLER: DeregisterRunnerPidfdHandler = DeregisterRunnerPidfdHandler;
static SPAWN_RUNNER_HANDLER: SpawnRunnerHandler = SpawnRunnerHandler;

/// The handler of [`INSPECT_PROCESS_FAMILY`].
struct InspectProcessFamilyHandler;

#[async_trait]
impl OperationHandler for InspectProcessFamilyHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let requested = match payload.object().get("resourceType") {
            Some(CanonicalJsonValue::String(name)) => name.as_str(),
            _ => return Err(OperationFailure::new(INVALID_PROCESS_TYPE)),
        };
        let resource_type = MEMBER_TYPES
            .iter()
            .find(|member| member.to_resource_type_name().as_str() == requested)
            .ok_or_else(|| OperationFailure::new(INVALID_PROCESS_TYPE))?;
        let member_types: Vec<String> = MEMBER_TYPES
            .iter()
            .map(|member| member.to_resource_type_name().as_str().to_owned())
            .collect();
        let reads: Vec<String> = PROCESS_FAMILY_READS
            .iter()
            .map(|read| read.to_resource_type_name().as_str().to_owned())
            .collect();
        let operations = [
            INSPECT_PROCESS_FAMILY,
            OPEN_PIDFD,
            OPEN_PEER_PIDFD_FROM_ACCEPTED_SOCKET,
            OBSERVE_RUNNER,
            POLL_CHILD_REAPED,
            PREPARE_RUNTIME_DIR,
            PREPARE_STATE_DIR,
            CGROUP_KILL,
            SIGNAL_RUNNER,
            DEREGISTER_RUNNER_PIDFD,
            SPAWN_RUNNER,
        ];
        let bytes = canonical_json_bytes(&serde_json::json!({
            "family": "process",
            "resourceType": resource_type.to_resource_type_name().as_str(),
            "memberTypes": member_types,
            "verbs": PROCESS_FAMILY_VERBS,
            "execution": PROCESS_FAMILY_EXECUTION_DOMAINS,
            "reads": reads,
            "operations": operations,
            "zone": ctx.zone.as_str(),
            "invocation": ctx.invocation_id,
        }))
        .expect("the inspection result is canonical JSON");
        let result =
            CanonicalJsonObject::parse(&bytes).expect("the inspection result is a JSON object");
        Ok(OperationResult::new(result))
    }
}

// ---------------------------------------------------------------------------
// The nested kernel leg
// ---------------------------------------------------------------------------

/// Invoke one broker-generic kernel as the nested core of a family operation.
///
/// The kernel call re-presents the evidence chain the forwarded invocation
/// runs on - the root invocation id plus the ordered identities - with the
/// handler's own caller identity appended, so the graft rule authorizes the
/// kernel call against the chain's initiating principal and the in-broker
/// leg records the correlation leg keyed by the same root invocation id
/// (KTD6). The kernel's reply carries the envelope response plus the
/// descriptors the kernel minted, in frame order.
async fn invoke_kernel_nested(
    ctx: &OperationCtx<'_>,
    kernel_operation: &'static str,
    payload: serde_json::Value,
    fds: Vec<OwnedFd>,
) -> Result<KernelReply, OperationFailure> {
    let kernel = ctx
        .kernel
        .ok_or_else(|| OperationFailure::new(KERNEL_SEAM_UNWIRED))?;
    let socket_path = kernel.socket_path.clone();
    let caller_role = kernel.caller_role.clone();
    let zone = ctx.zone.as_str().to_owned();
    let invocation_id = ctx.invocation_id.to_owned();
    let mut chain_identities = ctx.chain_identities.to_vec();
    chain_identities.push(ctx.caller.to_canonical_string());
    tokio::task::spawn_blocking(move || {
        envelope_invoke_kernel(
            &socket_path,
            KERNEL_IO_TIMEOUT,
            caller_role,
            KernelInvocation {
                operation: kernel_operation,
                zone: &zone,
                payload,
                fds: &fds,
                chain_root_invocation_id: Some(&invocation_id),
                chain_identities: Some(&chain_identities),
            },
        )
    })
    .await
    .map_err(|error| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("{kernel_operation}: kernel task join failed: {error}"),
        )
    })?
    .map_err(|error| {
        // The kernel's own closed code is preserved: "errored" for a kernel
        // error, "handler-refused" for a kernel refusal, so the caller's
        // classification of the leg keeps working exactly as it did for the
        // retired typed arms.
        let (code, detail) = match error {
            KernelInvokeError::Refused { code, detail } => (code, detail),
            other => ("errored".to_owned(), Some(other.to_string())),
        };
        OperationFailure::with_detail(
            if code == "errored" {
                "errored"
            } else {
                KERNEL_REFUSED
            },
            format!(
                "{kernel_operation}: {code}{}",
                detail
                    .map(|detail| format!(" ({detail})"))
                    .unwrap_or_default()
            ),
        )
    })
}

/// The canonical result object of one kernel reply.
///
/// A kernel reply that carries no result object is a protocol violation on
/// the broker side; the handler refuses rather than inventing a result.
fn kernel_result(reply: &KernelReply) -> Result<&serde_json::Value, OperationFailure> {
    reply.response.result.as_ref().ok_or_else(|| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("{}: kernel reply carries no result", reply.response.operation),
        )
    })
}

/// Duplicate one raw descriptor into a fresh owned descriptor.
///
/// The descriptor is duplicated through the process's own pidfd
/// (`pidfd_open` + `pidfd_getfd`), so the conversion is safe: the returned
/// descriptor is a new owned descriptor, and the raw descriptor stays owned
/// by its original holder.
fn duplicate_fd(fd: i32, context: &str) -> Result<OwnedFd, OperationFailure> {
    let pid = rustix::process::Pid::from_raw(std::process::id() as i32).ok_or_else(|| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("{context}: current pid is invalid"),
        )
    })?;
    let self_pidfd =
        rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()).map_err(|error| {
            OperationFailure::with_detail(
                KERNEL_REFUSED,
                format!("{context}: pidfd_open failed: {error}"),
            )
        })?;
    rustix::process::pidfd_getfd(&self_pidfd, fd, rustix::process::PidfdGetfdFlags::empty())
        .map_err(|error| {
            OperationFailure::with_detail(
                KERNEL_REFUSED,
                format!("{context}: duplicate descriptor failed: {error}"),
            )
        })
}

/// Take one descriptor out of a kernel reply's fd vector by index.
///
/// The reply's descriptors are owned by the reply; taking one by index
/// leaves the rest to drop with the reply.
fn take_reply_fd(reply: KernelReply, index: usize) -> Result<OwnedFd, OperationFailure> {
    reply.fds.into_iter().nth(index).ok_or_else(|| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("{}: fd index {index} missing", reply.response.operation),
        )
    })
}

/// Deserialize one typed request from the forwarded payload.
fn typed_request<T: serde::de::DeserializeOwned>(
    operation: &str,
    payload: &ValidatedPayload,
) -> Result<T, OperationFailure> {
    let value = serde_json::to_value(payload.object()).map_err(|error| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("{operation}: payload conversion failed: {error}"),
        )
    })?;
    serde_json::from_value(value).map_err(|error| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("{operation}: invalid typed request: {error}"),
        )
    })
}

/// Serialize one typed response into the canonical result payload.
fn typed_result<T: Serialize>(operation: &str, response: &T) -> Result<CanonicalJsonObject, OperationFailure> {
    let value = serde_json::to_value(response).map_err(|error| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("{operation}: response serialization failed: {error}"),
        )
    })?;
    let bytes = canonical_json_bytes(&value).map_err(|error| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("{operation}: response is not canonical JSON: {error}"),
        )
    })?;
    CanonicalJsonObject::parse(&bytes).map_err(|error| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("{operation}: response is not a canonical object: {error}"),
        )
    })
}

/// The daemon-side runner lookup of one `(vm, role)`.
fn runner_lookup(
    kernel: &KernelCaller,
    vm: &str,
    role: &str,
) -> Option<(i32, u64)> {
    kernel
        .runner_lookup
        .as_ref()
        .and_then(|lookup| lookup.lookup(vm, role))
}

/// Resolve one trusted runner intent from the Zone's bundle.
fn resolve_intent(
    kernel: &KernelCaller,
    operation: &str,
    bundle_runner_intent_ref: &str,
) -> Result<ResolvedRunnerIntent, OperationFailure> {
    kernel
        .bundle
        .find_runner_intent(bundle_runner_intent_ref)
        .cloned()
        .ok_or_else(|| {
            OperationFailure::with_detail(
                KERNEL_REFUSED,
                format!("{operation}: bundle runner intent {bundle_runner_intent_ref} is missing"),
            )
        })
}

/// The wire `RunnerRole` one trusted process role fences against.
///
/// Mirrors the retired broker arms' role mapping; a process role with no wire
/// runner role is not a spawnable runner.
fn runner_role_for_process_role(role: &ProcessRole) -> Option<RunnerRole> {
    match role {
        ProcessRole::ProviderController => Some(RunnerRole::ProviderController),
        ProcessRole::SwtpmPreStartFlush => Some(RunnerRole::SwtpmFlush),
        ProcessRole::Swtpm => Some(RunnerRole::Swtpm),
        ProcessRole::Virtiofsd => Some(RunnerRole::Virtiofsd),
        ProcessRole::Video => Some(RunnerRole::Video),
        ProcessRole::Gpu | ProcessRole::GpuRenderNode => Some(RunnerRole::Gpu),
        ProcessRole::Audio => Some(RunnerRole::Audio),
        ProcessRole::CloudHypervisorRunner => Some(RunnerRole::CloudHypervisor),
        ProcessRole::QemuMediaRunner => Some(RunnerRole::QemuMedia),
        ProcessRole::ActivationNixosRunner => Some(RunnerRole::ActivationNixos),
        ProcessRole::VsockRelay => Some(RunnerRole::VsockRelay),
        ProcessRole::OtelHostBridge => Some(RunnerRole::OtelHostBridge),
        ProcessRole::Usbip => Some(RunnerRole::Usbip),
        ProcessRole::WaylandProxy => Some(RunnerRole::WaylandProxy),
        ProcessRole::HostReconcile
        | ProcessRole::StoreVirtiofsPreflight
        | ProcessRole::ComponentSessionHealth
        | ProcessRole::SecurityKeyFrontend => None,
    }
}

/// The wire `role_id` one trusted runner intent fences against.
///
/// The cloud-hypervisor runner keeps its daemon-side `ch-runner` alias; every
/// other intent uses its own `role_id` (the retired broker arms' single
/// evaluation point for the alias).
fn wire_role_id_for_intent(intent: &ResolvedRunnerIntent) -> &str {
    match intent.role {
        ProcessRole::CloudHypervisorRunner => "ch-runner",
        _ => intent.role_id.as_str(),
    }
}

// ---------------------------------------------------------------------------
// The launch posture (ported from the retired broker arm)
// ---------------------------------------------------------------------------

/// The launch posture of one runner request (U10).
///
/// Mirrors the retired broker arm's `LaunchPosture`: the posture is resolved
/// exactly once from the trusted intent, never from caller-supplied fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchPosture {
    /// Ordinary runner launch: no controller escrow descriptor.
    Standard,
    /// Non-serving `ProviderController` launch: historically carried the
    /// controller bootstrap escrow descriptor. The U10 family rows declare no
    /// fd facet, so the escrow cannot cross the forward carrier today; such a
    /// launch is refused fail-closed by the fd-count fence.
    ControllerEscrow,
    /// Binding-owned serving worker: carries no broker escrow descriptor.
    ServingWorker,
}

impl LaunchPosture {
    /// Resolve the posture once, from the trusted intent, at the point the
    /// handler resolves that intent.
    fn resolve(role: RunnerRole, intent: &ResolvedRunnerIntent) -> Self {
        match (role, intent_is_serving_worker_template(intent)) {
            (RunnerRole::ProviderController, true) => Self::ServingWorker,
            (RunnerRole::ProviderController, false) => Self::ControllerEscrow,
            _ => Self::Standard,
        }
    }

    fn is_provider_controller(self) -> bool {
        !matches!(self, Self::Standard)
    }

    fn is_serving_worker(self) -> bool {
        matches!(self, Self::ServingWorker)
    }

    /// Whether the request's declared inherited-descriptor count matches the
    /// descriptors the family row's fd-less contract can actually transport.
    ///
    /// The family rows declare no fd facet (U10), so a forwarded family call
    /// carries no SCM_RIGHTS attachments. A request that declares inherited
    /// descriptors - the retired ProviderController escrow contract - cannot
    /// be served and is refused fail-closed.
    fn validate_request_fds(
        self,
        inherited_fd_count: u16,
        attached_fd_count: usize,
    ) -> Result<(), OperationFailure> {
        const MAX_REQUEST_INHERITED_FDS: u16 = 256;
        // The retired arm required a ProviderController spawn to carry its
        // bootstrap escrow descriptor (1..=2 inherited + 1 attached) and
        // refused the (0, 0) shape fail-closed. The family rows declare no
        // fd facet (U10), so the escrow can never cross the forward carrier
        // today: the posture is unservable outright, and admitting a
        // (0, 0) controller launch would spawn a controller without its
        // bootstrap fd - the exact state the retired arm refused.
        if self == Self::ControllerEscrow {
            return Err(OperationFailure::with_detail(
                INHERITED_FDS_UNSUPPORTED,
                "SpawnRunner: the ControllerEscrow posture needs its bootstrap \
                 escrow descriptor, which the family row's fd-less contract \
                 cannot transport",
            ));
        }
        if inherited_fd_count > MAX_REQUEST_INHERITED_FDS {
            return Err(OperationFailure::with_detail(
                KERNEL_REFUSED,
                format!(
                    "SpawnRunner: inherited fd count {inherited_fd_count} exceeds the bounded maximum"
                ),
            ));
        }
        if inherited_fd_count != 0 {
            return Err(OperationFailure::with_detail(
                INHERITED_FDS_UNSUPPORTED,
                format!(
                    "SpawnRunner: the family row's fd-less contract cannot transport \
                     {inherited_fd_count} inherited descriptors (posture {self:?})"
                ),
            ));
        }
        if attached_fd_count != 0 {
            return Err(OperationFailure::with_detail(
                KERNEL_REFUSED,
                format!(
                    "SpawnRunner: the family row's fd-less contract cannot transport \
                     {attached_fd_count} attached descriptors"
                ),
            ));
        }
        Ok(())
    }
}

/// Whether one trusted intent is a binding-owned serving worker template.
fn intent_is_serving_worker_template(intent: &ResolvedRunnerIntent) -> bool {
    intent.role == ProcessRole::ProviderController
        && intent.profile_id == "virtiofsd-worker"
        && intent.owner_ref.as_deref() == Some("Provider/volume-virtiofs")
}

// ---------------------------------------------------------------------------
// Typed identity fences (ported from the retired broker arm)
// ---------------------------------------------------------------------------

/// The private runtime scope commitment of one typed Process row.
fn private_runtime_scope(
    zone_uid: &ResourceUid,
    guest_uid: Option<&ResourceUid>,
    resource_ref: &ResourceRef,
    resource_uid: &ResourceUid,
    role_id: &str,
    generation: u64,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b-process-runtime-scope-v1");
    let resource_ref = resource_ref.to_canonical_string();
    for value in [
        zone_uid.as_str(),
        guest_uid.map(|uid| uid.as_str()).unwrap_or(""),
        resource_ref.as_str(),
        resource_uid.as_str(),
        role_id,
    ] {
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    digest.update(generation.to_le_bytes());
    digest.finalize().into()
}

/// Whether the request carries a typed Process identity, and whether that
/// identity is complete and internally consistent.
fn typed_process_identity(
    resource_ref: Option<&ResourceRef>,
    resource_uid: Option<&ResourceUid>,
    zone_uid: Option<&ResourceUid>,
    generation: Option<u64>,
    runtime_scope: Option<[u8; 32]>,
    intent_role_id: &str,
    guest_execution: Option<&GuestExecutionBinding>,
) -> Result<bool, OperationFailure> {
    let typed = resource_ref.is_some()
        || resource_uid.is_some()
        || zone_uid.is_some()
        || runtime_scope.is_some();
    if !typed {
        return Ok(false);
    }
    let (Some(resource_ref), Some(resource_uid), Some(zone_uid), Some(generation), Some(scope)) = (
        resource_ref,
        resource_uid,
        zone_uid,
        generation,
        runtime_scope,
    ) else {
        return Err(OperationFailure::with_detail(
            PROCESS_IDENTITY_INVALID,
            "process_identity: resource/zone/uid/generation/scope-required".to_owned(),
        ));
    };
    if generation == 0 || scope == [0; 32] {
        return Err(OperationFailure::with_detail(
            PROCESS_IDENTITY_INVALID,
            "process_identity: nonzero-resource/zone/uid/generation/scope".to_owned(),
        ));
    }
    if !matches!(
        resource_ref.resource_type().as_str(),
        "Process" | "EphemeralProcess"
    ) {
        return Err(OperationFailure::with_detail(
            PROCESS_IDENTITY_INVALID,
            format!(
                "resource_ref: {} is not a Process or EphemeralProcess",
                resource_ref.to_canonical_string()
            ),
        ));
    }
    let expected = private_runtime_scope(
        zone_uid,
        guest_execution.map(|binding| &binding.target_uid),
        resource_ref,
        resource_uid,
        intent_role_id,
        generation,
    );
    if scope != expected {
        return Err(OperationFailure::with_detail(
            PROCESS_IDENTITY_INVALID,
            "runtime_scope: zone-resource-generation-role-commitment".to_owned(),
        ));
    }
    Ok(true)
}

/// The typed control identity completeness fence of the control operations.
#[allow(clippy::too_many_arguments)]
fn typed_control_identity_complete(
    resource_ref: Option<&ResourceRef>,
    resource_uid: Option<&ResourceUid>,
    zone_uid: Option<&ResourceUid>,
    generation: Option<u64>,
    runtime_scope: Option<[u8; 32]>,
    provider_ref: Option<&ResourceRef>,
    provider_identity: Option<[u8; 32]>,
    template_identity: Option<[u8; 32]>,
) -> bool {
    let typed = resource_ref.is_some()
        || resource_uid.is_some()
        || zone_uid.is_some()
        || generation.is_some()
        || runtime_scope.is_some()
        || provider_ref.is_some()
        || provider_identity.is_some()
        || template_identity.is_some();
    if !typed {
        return true;
    }
    resource_ref.is_some()
        && resource_uid.is_some()
        && zone_uid.is_some()
        && generation.is_some_and(|generation| generation > 0)
        && runtime_scope.is_some_and(|scope| scope != [0; 32])
        && provider_ref.is_some()
        && provider_identity.is_some_and(|identity| identity != [0; 32])
        && template_identity.is_some_and(|identity| identity != [0; 32])
}

/// The typed Process metadata fences of one spawn request.
#[allow(clippy::too_many_arguments)]
fn validate_typed_process_metadata(
    typed: bool,
    owner_ref: Option<&ResourceRef>,
    provider_ref: Option<&ResourceRef>,
    provider_identity: Option<[u8; 32]>,
    template_identity: Option<[u8; 32]>,
    guest_execution: Option<&GuestExecutionBinding>,
    serving_worker: bool,
    intent: &ResolvedRunnerIntent,
    device_worker_scope: Option<&DeviceWorkerScope>,
) -> Result<(), OperationFailure> {
    if !typed {
        return Ok(());
    }
    let execution_is_guest = intent.execution_ref.starts_with("Guest/");
    if execution_is_guest != guest_execution.is_some()
        || guest_execution.is_some_and(|binding| !binding.is_valid())
    {
        return Err(OperationFailure::with_detail(
            INTENT_MISMATCH,
            "guest_execution: bundle-execution-target".to_owned(),
        ));
    }
    let Some(provider_identity) = provider_identity else {
        return Err(OperationFailure::with_detail(
            INTENT_MISMATCH,
            "provider_identity: signed-process-provider".to_owned(),
        ));
    };
    if provider_identity == [0; 32] {
        return Err(OperationFailure::with_detail(
            INTENT_MISMATCH,
            "provider_identity: signed-process-provider".to_owned(),
        ));
    }
    let Some(provider_ref) = provider_ref else {
        return Err(OperationFailure::with_detail(
            INTENT_MISMATCH,
            "provider_ref: signed-process-provider".to_owned(),
        ));
    };
    if provider_ref.resource_type().as_str() != "Provider"
        || !matches!(
            provider_ref.name().as_str(),
            "system-minijail" | "system-systemd"
        )
    {
        return Err(OperationFailure::with_detail(
            INTENT_MISMATCH,
            "provider_ref: fixed-process-provider".to_owned(),
        ));
    }
    let mut provider_digest = Sha256::new();
    provider_digest.update(b"d2b-process-provider-v1");
    provider_digest.update(provider_ref.name().as_str().as_bytes());
    let expected_provider_identity: [u8; 32] = provider_digest.finalize().into();
    if provider_identity != expected_provider_identity {
        return Err(OperationFailure::with_detail(
            INTENT_MISMATCH,
            "provider_identity: signed-process-provider".to_owned(),
        ));
    }
    let expected_template = if intent.role == ProcessRole::ProviderController
        || is_device_worker_role(&intent.role)
    {
        intent.profile_id.as_str()
    } else {
        intent.role_id.as_str()
    };
    let mut digest = Sha256::new();
    digest.update(b"d2b-process-template-v1");
    digest.update(expected_template.as_bytes());
    let expected_template: [u8; 32] = digest.finalize().into();
    if template_identity != Some(expected_template) {
        return Err(OperationFailure::with_detail(
            INTENT_MISMATCH,
            "template_identity: bundle-process-template".to_owned(),
        ));
    }
    if let Some(owner) = owner_ref {
        if let Some(scope) = device_worker_scope
            && owner != &scope.device_ref
        {
            return Err(OperationFailure::with_detail(
                INTENT_MISMATCH,
                format!(
                    "owner_ref: {} does not name the owning Device {}",
                    owner.to_canonical_string(),
                    scope.device_ref.to_canonical_string()
                ),
            ));
        }
        let owner_type = owner.resource_type().as_str();
        let admitted = if is_device_worker_role(&intent.role) {
            owner_type == "Device"
                && device_worker_scope.is_none_or(|scope| owner == &scope.device_ref)
        } else {
            matches!(owner_type, "Guest" | "Host" | "Provider" | "VolumeBinding")
        };
        if !admitted {
            return Err(OperationFailure::with_detail(
                INTENT_MISMATCH,
                "owner_ref: Guest-or-Host-or-Provider-or-VolumeBinding-or-Device-worker".to_owned(),
            ));
        }
    } else if is_device_worker_role(&intent.role) {
        return Err(OperationFailure::with_detail(
            INTENT_MISMATCH,
            "owner_ref: Device".to_owned(),
        ));
    }
    if intent.role == ProcessRole::ProviderController {
        if serving_worker {
            if !owner_ref.is_some_and(|owner| owner.resource_type().as_str() == "VolumeBinding") {
                return Err(OperationFailure::with_detail(
                    INTENT_MISMATCH,
                    "owner_ref: VolumeBinding".to_owned(),
                ));
            }
        } else {
            let expected_owner = intent
                .owner_ref
                .as_deref()
                .map(ResourceRef::parse)
                .transpose()
                .map_err(|_| {
                    OperationFailure::with_detail(
                        INTENT_MISMATCH,
                        "owner_ref: invalid-bundle-owner".to_owned(),
                    )
                })?;
            if owner_ref != expected_owner.as_ref() {
                return Err(OperationFailure::with_detail(
                    INTENT_MISMATCH,
                    "owner_ref: bundle-owner".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

/// The private cgroup placement of one typed Process launch.
///
/// A typed launch is placed under a private cgroup scope that deliberately
/// carries no VM name; the placement is derived from the trusted intent's
/// placement and the request's runtime-scope commitment (the retired broker
/// arm's fence).
fn private_cgroup_placement(
    placement: &CgroupPlacement,
    vm_name: &str,
    runtime_scope: Option<[u8; 32]>,
    typed: bool,
) -> Result<CgroupPlacement, OperationFailure> {
    if !typed {
        return Ok(placement.clone());
    }
    let Some(runtime_scope) = runtime_scope else {
        return Err(OperationFailure::with_detail(
            INTENT_MISMATCH,
            "cgroup_commitment: runtime-scope-required".to_owned(),
        ));
    };
    let components = Path::new(&placement.subtree)
        .components()
        .collect::<Vec<_>>();
    if components
        .iter()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
        || !matches!(
            components.first(),
            Some(std::path::Component::Normal(value)) if *value == "d2b.slice"
        )
    {
        return Err(OperationFailure::with_detail(
            INTENT_MISMATCH,
            format!(
                "cgroup_commitment: {} is not d2b.slice/<zone>/<guest>/<role>",
                placement.subtree
            ),
        ));
    }
    let segment = |index: usize| {
        components.get(index).and_then(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
    };
    let role_start = if components.len() >= 4 && segment(2) == Some(vm_name) {
        3
    } else if components.len() >= 3 && segment(1) == Some(vm_name) {
        2
    } else {
        return Err(OperationFailure::with_detail(
            INTENT_MISMATCH,
            format!(
                "cgroup_commitment: {} does not name {vm_name}",
                placement.subtree
            ),
        ));
    };
    let role_path = components[role_start..]
        .iter()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if role_path.is_empty() {
        return Err(OperationFailure::with_detail(
            INTENT_MISMATCH,
            format!("cgroup_commitment: {} has an empty role path", placement.subtree),
        ));
    }
    let mut private = placement.clone();
    private.subtree = format!(
        "d2b.slice/process-{}/{}",
        runtime_scope
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        role_path
    );
    Ok(private)
}

/// Validate one sandbox DTO against the trusted intent (the retired broker
/// arm's `validate_sandbox_launch_plan` fence).
fn validate_sandbox_launch_plan(
    req: &SpawnRunnerRequest,
    intent: &ResolvedRunnerIntent,
    plan: &d2b_contracts_broker::broker_wire::SandboxLaunchPlan,
) -> Result<(), OperationFailure> {
    use d2b_contracts_resource::v3::process::{CapabilityClass, EnvironmentClass, NamespaceClass};

    let mismatch = |field: &str, requested: String, resolved: &str| {
        OperationFailure::with_detail(
            INTENT_MISMATCH,
            format!("sandbox_plan.{field}: {requested} vs {resolved}"),
        )
    };
    if plan.domain
        != req
            .execution_domain
            .unwrap_or(ExecutionDomain::System)
    {
        return Err(mismatch("domain", format!("{:?}", plan.domain), "request-domain"));
    }
    if !plan.no_new_privileges {
        return Err(mismatch("no_new_privileges", "false".to_owned(), "true"));
    }
    if plan.start_root != intent.root_carve_out {
        return Err(mismatch(
            "start_root",
            plan.start_root.to_string(),
            if intent.root_carve_out { "true" } else { "false" },
        ));
    }
    if !plan.read_only_root {
        return Err(mismatch("read_only_root", "false".to_owned(), "unsupported"));
    }
    if plan.oom_score_adj != 0 {
        return Err(mismatch(
            "oom_score_adj",
            plan.oom_score_adj.to_string(),
            "unsupported",
        ));
    }
    if plan.environment_class != EnvironmentClass::Minimal {
        return Err(mismatch(
            "environment_class",
            format!("{:?}", plan.environment_class),
            "minimal-only",
        ));
    }
    let expected_namespaces = &intent.namespaces;
    let requested_namespaces = |class| match class {
        NamespaceClass::User => expected_namespaces.user,
        NamespaceClass::Pid => expected_namespaces.pid,
        NamespaceClass::Mount => expected_namespaces.mount,
        NamespaceClass::Ipc => expected_namespaces.ipc,
        NamespaceClass::Uts => expected_namespaces.uts,
        NamespaceClass::Network => expected_namespaces.net,
        NamespaceClass::Cgroup | NamespaceClass::Time => false,
    };
    for class in &plan.namespace_classes {
        if !requested_namespaces(*class) {
            return Err(mismatch(
                "namespace_classes",
                format!("{class:?}"),
                "bundle-profile",
            ));
        }
    }
    if plan.namespace_classes.iter().any(|class| {
        matches!(class, NamespaceClass::User)
    }) != expected_namespaces.user
    {
        return Err(mismatch("namespace_classes", "user".to_owned(), "bundle-profile"));
    }
    if plan.user_namespace.is_some() != intent.user_namespace.is_some()
        || plan.user_namespace.is_some_and(|spec| {
            spec.mapping_class
                != d2b_contracts_resource::v3::process::MappingClass::ProcessPrincipalRoot
        })
    {
        return Err(mismatch(
            "user_namespace",
            format!("{:?}", plan.user_namespace),
            "bundle-profile",
        ));
    }
    if let Some(umask) = &plan.umask {
        let parsed = u32::from_str_radix(umask, 8).map_err(|_| {
            mismatch("umask", umask.clone(), "valid-octal")
        })?;
        if intent.umask != Some(parsed) {
            return Err(mismatch(
                "umask",
                umask.clone(),
                &intent
                    .umask
                    .map_or_else(|| "inherit".to_owned(), |value| format!("{value:o}")),
            ));
        }
    } else if intent.umask.is_some() {
        return Err(mismatch("umask", "inherit".to_owned(), "bundle-profile"));
    }
    if plan.capability_classes.iter().any(|class| {
        !intent
            .capabilities
            .iter()
            .any(|capability| capability_matches(*class, capability))
    }) {
        return Err(mismatch(
            "capability_classes",
            "mismatch".to_owned(),
            "bundle-profile",
        ));
    }
    if plan.seccomp_class.as_str() != "strict" || intent.seccomp_policy_ref.is_none() {
        return Err(mismatch(
            "seccomp_class",
            plan.seccomp_class.as_str().to_owned(),
            "strict",
        ));
    }
    let expected_namespaces = [
        (NamespaceClass::User, expected_namespaces.user),
        (NamespaceClass::Pid, expected_namespaces.pid),
        (NamespaceClass::Mount, expected_namespaces.mount),
        (NamespaceClass::Ipc, expected_namespaces.ipc),
        (NamespaceClass::Uts, expected_namespaces.uts),
        (NamespaceClass::Network, expected_namespaces.net),
    ]
    .into_iter()
    .filter_map(|(class, enabled)| enabled.then_some(class))
    .collect::<BTreeSet<_>>();
    let actual_namespaces = plan
        .namespace_classes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if actual_namespaces != expected_namespaces
        || actual_namespaces.len() != plan.namespace_classes.len()
    {
        return Err(mismatch(
            "namespace_classes",
            "mismatch".to_owned(),
            "bundle-profile",
        ));
    }
    let expected_capabilities = intent
        .capabilities
        .iter()
        .filter_map(|capability| match capability.as_str() {
            "CAP_NET_BIND_SERVICE" => Some(CapabilityClass::NetworkBind),
            "CAP_NET_RAW" => Some(CapabilityClass::NetworkRaw),
            "CAP_NET_ADMIN" => Some(CapabilityClass::NetworkAdmin),
            "CAP_SYS_TIME" => Some(CapabilityClass::SysTime),
            "CAP_SYS_PTRACE" => Some(CapabilityClass::SysPtrace),
            "CAP_SYS_ADMIN" => Some(CapabilityClass::SysAdmin),
            "CAP_DAC_OVERRIDE" => Some(CapabilityClass::DacOverride),
            "CAP_FOWNER" => Some(CapabilityClass::Fowner),
            "CAP_CHOWN" => Some(CapabilityClass::Chown),
            "CAP_SETUID" => Some(CapabilityClass::Setuid),
            "CAP_SETGID" => Some(CapabilityClass::Setgid),
            "CAP_AUDIT_WRITE" => Some(CapabilityClass::AuditWrite),
            "CAP_KILL" => Some(CapabilityClass::Kill),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let actual_capabilities = plan
        .capability_classes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if actual_capabilities != expected_capabilities
        || actual_capabilities.len() != plan.capability_classes.len()
    {
        return Err(mismatch(
            "capability_classes",
            "mismatch".to_owned(),
            "bundle-profile",
        ));
    }
    Ok(())
}

/// Whether one capability class names one capability string.
fn capability_matches(
    class: d2b_contracts_resource::v3::process::CapabilityClass,
    capability: &str,
) -> bool {
    let expected = match class {
        d2b_contracts_resource::v3::process::CapabilityClass::NetworkBind => "CAP_NET_BIND_SERVICE",
        d2b_contracts_resource::v3::process::CapabilityClass::NetworkRaw => "CAP_NET_RAW",
        d2b_contracts_resource::v3::process::CapabilityClass::NetworkAdmin => "CAP_NET_ADMIN",
        d2b_contracts_resource::v3::process::CapabilityClass::SysTime => "CAP_SYS_TIME",
        d2b_contracts_resource::v3::process::CapabilityClass::SysPtrace => "CAP_SYS_PTRACE",
        d2b_contracts_resource::v3::process::CapabilityClass::SysAdmin => "CAP_SYS_ADMIN",
        d2b_contracts_resource::v3::process::CapabilityClass::DacOverride => "CAP_DAC_OVERRIDE",
        d2b_contracts_resource::v3::process::CapabilityClass::Fowner => "CAP_FOWNER",
        d2b_contracts_resource::v3::process::CapabilityClass::Chown => "CAP_CHOWN",
        d2b_contracts_resource::v3::process::CapabilityClass::Setuid => "CAP_SETUID",
        d2b_contracts_resource::v3::process::CapabilityClass::Setgid => "CAP_SETGID",
        d2b_contracts_resource::v3::process::CapabilityClass::AuditWrite => "CAP_AUDIT_WRITE",
        d2b_contracts_resource::v3::process::CapabilityClass::Kill => "CAP_KILL",
    };
    capability == expected
}

/// Validate one spawn request's identity fields against the trusted intent
/// (the retired broker arm's `validate_spawn_runner_request_matches_intent`
/// fence).
#[allow(clippy::too_many_arguments)]
fn validate_spawn_runner_request_matches_intent(
    req: &SpawnRunnerRequest,
    intent: &ResolvedRunnerIntent,
    posture: LaunchPosture,
    device_worker_scope: Option<&DeviceWorkerScope>,
) -> Result<(), OperationFailure> {
    let mismatch = |field: &str, requested: String, resolved: &str| {
        OperationFailure::with_detail(
            INTENT_MISMATCH,
            format!("{field}: {requested} vs {resolved}"),
        )
    };
    if req.vm_id.as_str() != intent.vm_name {
        return Err(mismatch(
            "vm_id",
            req.vm_id.as_str().to_owned(),
            &intent.vm_name,
        ));
    }
    if let Some(execution_ref) = &req.execution_ref {
        let expected = ResourceRef::parse(&intent.execution_ref).map_err(|_| {
            mismatch("execution_ref", execution_ref.to_canonical_string(), "invalid-bundle-reference")
        })?;
        if execution_ref != &expected {
            return Err(mismatch(
                "execution_ref",
                execution_ref.to_canonical_string(),
                &expected.to_canonical_string(),
            ));
        }
    }
    let expected_domain = match intent.execution_domain {
        d2b_core::processes::ProcessExecutionDomain::System => ExecutionDomain::System,
        d2b_core::processes::ProcessExecutionDomain::User => ExecutionDomain::User,
    };
    if req
        .execution_domain
        .is_some_and(|domain| domain != expected_domain)
    {
        return Err(mismatch(
            "execution_domain",
            format!("{:?}", req.execution_domain),
            &format!("{expected_domain:?}"),
        ));
    }
    let expected_user = intent
        .user_ref
        .as_deref()
        .map(ResourceRef::parse)
        .transpose()
        .map_err(|_| mismatch("user_ref", "invalid-bundle-reference".to_owned(), "invalid-bundle-reference"))?;
    if req.user_ref != expected_user {
        return Err(mismatch(
            "user_ref",
            format!("{:?}", req.user_ref),
            &format!("{expected_user:?}"),
        ));
    }
    let expected_role_id = wire_role_id_for_intent(intent);
    if req.role_id.as_str() != expected_role_id {
        return Err(mismatch(
            "role_id",
            req.role_id.as_str().to_owned(),
            expected_role_id,
        ));
    }
    let Some(expected_role) = runner_role_for_process_role(&intent.role) else {
        return Err(mismatch(
            "role",
            req.role.as_str().to_owned(),
            &format!("{:?}", intent.role),
        ));
    };
    if req.role != expected_role {
        return Err(mismatch(
            "role",
            req.role.as_str().to_owned(),
            expected_role.as_str(),
        ));
    }
    if req.launch_args.is_some() && !intent.accepts_launch_args {
        return Err(mismatch(
            "launch_args",
            "present".to_owned(),
            "template-does-not-admit-controller-arguments",
        ));
    }
    let typed = typed_process_identity(
        req.resource_ref.as_ref(),
        req.resource_uid.as_ref(),
        req.zone_uid.as_ref(),
        req.generation,
        req.runtime_scope,
        &intent.role_id,
        req.guest_execution.as_ref(),
    )?;
    if !typed && posture.is_provider_controller() {
        return Err(mismatch(
            "process_identity",
            "untyped".to_owned(),
            "typed-provider-controller-required",
        ));
    }
    validate_typed_process_metadata(
        typed,
        req.owner_ref.as_ref(),
        req.provider_ref.as_ref(),
        req.provider_identity,
        req.template_identity,
        req.guest_execution.as_ref(),
        posture.is_serving_worker(),
        intent,
        device_worker_scope,
    )?;
    if typed
        && (!req.runtime_allocations.is_empty()
            || req.workload_identity.is_some()
            || req.network_tap_context.is_some())
    {
        return Err(mismatch(
            "runtime_allocations",
            "caller-supplied-runtime-state".to_owned(),
            "bundle-authoritative-runtime",
        ));
    }
    Ok(())
}

/// Bind the cloud-hypervisor guest uid into the launch argv (the retired
/// broker arm's fence).
fn bind_cloud_hypervisor_guest_uid(
    role: RunnerRole,
    owner_uid: Option<&ResourceUid>,
    argv: &[String],
) -> Result<Vec<String>, OperationFailure> {
    if role != RunnerRole::CloudHypervisor {
        return Ok(argv.to_vec());
    }
    let owner_uid = owner_uid.ok_or_else(|| {
        OperationFailure::with_detail(
            INTENT_MISMATCH,
            "owner_uid: required-for-cloud-hypervisor".to_owned(),
        )
    })?;
    let mut bound = argv.to_vec();
    let cmdline_index = bound
        .iter()
        .position(|argument| argument == "--cmdline")
        .and_then(|index| bound.get(index + 1).map(|_| index + 1))
        .ok_or_else(|| {
            OperationFailure::with_detail(
                INTENT_MISMATCH,
                "argv: cloud-hypervisor-without-cmdline".to_owned(),
            )
        })?;
    if bound[cmdline_index]
        .split_ascii_whitespace()
        .any(|argument| argument.starts_with("d2b.guest_uid="))
    {
        return Err(OperationFailure::with_detail(
            INTENT_MISMATCH,
            "argv: prebound-guest-uid".to_owned(),
        ));
    }
    bound[cmdline_index].push_str(" d2b.guest_uid=");
    bound[cmdline_index].push_str(owner_uid.as_str());
    Ok(bound)
}

/// Extend the audio runner's environment with the PipeWire props the daemon
/// state names (the retired broker arm's `extend_audio_runner_pipewire_props`
/// enrichment).
fn extend_audio_runner_pipewire_props(
    vm_id: &str,
    role_id: &str,
    role: RunnerRole,
    env: &mut Vec<String>,
) -> Result<(), OperationFailure> {
    if !matches!(role, RunnerRole::Audio) || role_id != "audio" {
        return Ok(());
    }
    let state_path = PathBuf::from(format!("/var/lib/d2b/vms/{vm_id}/state/audio-state.json"));
    let bytes = std::fs::read(&state_path).map_err(|error| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!(
                "audio runner {vm_id}:{role_id} could not read {}: {error}",
                state_path.display()
            ),
        )
    })?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!(
                "audio runner {vm_id}:{role_id} could not parse {}: {error}",
                state_path.display()
            ),
        )
    })?;
    let mic = audio_state_value(&value, "mic", vm_id, role_id)?;
    let speaker = audio_state_value(&value, "speaker", vm_id, role_id)?;
    let input_target = audio_input_target_node(env, vm_id, role_id)?;
    let target_prop = if mic == "on" {
        input_target
            .as_deref()
            .map(|target| format!(" target.object = \"{target}\""))
            .unwrap_or_default()
    } else {
        String::new()
    };
    env.retain(|entry| !entry.starts_with("PIPEWIRE_PROPS="));
    env.push(format!(
        "PIPEWIRE_PROPS={{ application.name = \"d2b-{vm_id}\" node.name = \"d2b-{vm_id}\" node.description = \"d2b {vm_id}\" d2b.vm = \"{vm_id}\" d2b.mic = \"{mic}\" d2b.speaker = \"{speaker}\"{target_prop} }}"
    ));
    Ok(())
}

/// The audio input target node the launch env names, validated.
fn audio_input_target_node(
    env: &[String],
    vm_id: &str,
    role_id: &str,
) -> Result<Option<String>, OperationFailure> {
    let Some(raw) = env
        .iter()
        .find_map(|entry| entry.strip_prefix("D2B_AUDIO_INPUT_TARGET_NODE="))
    else {
        return Ok(None);
    };
    if raw.is_empty() || raw.contains('"') || raw.contains('\n') {
        return Err(OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("audio runner {vm_id}:{role_id} has invalid D2B_AUDIO_INPUT_TARGET_NODE"),
        ));
    }
    Ok(Some(raw.to_owned()))
}

/// One audio state value, validated to the closed `on`/`off` set.
fn audio_state_value<'a>(
    value: &'a serde_json::Value,
    key: &str,
    vm_id: &str,
    role_id: &str,
) -> Result<&'a str, OperationFailure> {
    let state = value.get(key).and_then(serde_json::Value::as_str).ok_or_else(|| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("audio runner {vm_id}:{role_id} state missing string key {key:?}"),
        )
    })?;
    match state {
        "on" | "off" => Ok(state),
        other => Err(OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("audio runner {vm_id}:{role_id} state key {key:?} has invalid value {other:?}"),
        )),
    }
}

/// Whether one process's cgroup path matches the expected subtree.
fn proc_cgroup_matches(pid: i32, expected_subtree: &str) -> bool {
    if expected_subtree.is_empty() {
        return false;
    }
    let Ok(content) = std::fs::read_to_string(format!("/proc/{pid}/cgroup")) else {
        return false;
    };
    let expected = expected_subtree.trim_start_matches('/');
    content.lines().any(|line| {
        let Some((_, path)) = line.split_once("::") else {
            return false;
        };
        let actual = path.trim_start_matches('/');
        actual == expected || actual.ends_with(&format!("/{expected}"))
    })
}

/// Whether one observed executable path matches the trusted binary.
fn executable_matches(actual: Option<&str>, expected: &Path) -> bool {
    let Some(actual) = actual else {
        return false;
    };
    let Ok(actual) = std::fs::canonicalize(actual) else {
        return false;
    };
    let Ok(expected) = std::fs::canonicalize(expected) else {
        return false;
    };
    actual == expected
}

// ---------------------------------------------------------------------------
// The Device-worker scope and swtpm identity (ported from the retired broker
// arm)
// ---------------------------------------------------------------------------

/// The pinned owning-Device scope of one Device-owned worker launch.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DeviceWorkerScope {
    zone_uid: ResourceUid,
    device_ref: ResourceRef,
    device_uid: ResourceUid,
    guest: String,
}

/// The trusted identity of one resource-backed `w1-swtpm` launch.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResourceBackedSwtpm {
    guest: String,
    state_root: PathBuf,
    state_volume: Option<String>,
}

/// The Device-worker launch scope of one spawn request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DeviceWorkerLaunch {
    scope: Option<DeviceWorkerScope>,
    binds_runtime_socket: bool,
}

/// The deterministic resource uid of one `(zone, type, name)` key, rendered
/// in the UUIDv4-shaped wire spelling (the same derivation the manager rows
/// use).
fn deterministic_resource_uid(zone: &str, resource_type: &str, name: &str) -> ResourceUid {
    let key = d2b_resource_runtime::identity::ResourceKey::new(zone, resource_type, name);
    let mut bytes = d2b_resource_runtime::manager::deterministic_uid(&key);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let text = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    );
    ResourceUid::parse(text).expect("the deterministic uid renders canonically")
}

/// The Zone resource bundle bytes for one Zone uid.
fn zone_bundle_for_uid<'a>(
    resolver: &'a BundleResolver,
    zone_uid: &ResourceUid,
) -> Option<(String, &'a [u8])> {
    let zone = resolver
        .zone_resource_bundle_zones()
        .ok()?
        .into_iter()
        .find(|zone| resolver.zone_uid(zone).as_ref() == Some(zone_uid))?;
    let bytes = resolver.zone_resource_bundle_bytes(zone.as_str())?;
    Some((zone.as_str().to_owned(), bytes))
}

/// The authored `metadata.ownerRef` of one row of a verified Zone resource
/// bundle, parsed into a canonical reference.
fn row_owner_ref(bundle_bytes: &[u8], resource_type: &str, name: &str) -> Option<ResourceRef> {
    let bundle: serde_json::Value = serde_json::from_slice(bundle_bytes).ok()?;
    for resource in bundle.get("resources")?.as_array()? {
        if resource.get("type").and_then(serde_json::Value::as_str) != Some(resource_type) {
            continue;
        }
        let metadata = resource.get("metadata")?;
        if metadata.get("name").and_then(serde_json::Value::as_str) != Some(name) {
            continue;
        }
        return metadata
            .get("ownerRef")
            .and_then(serde_json::Value::as_str)
            .and_then(|owner| ResourceRef::parse(owner).ok());
    }
    None
}

/// The Guest one Device row declares as its owner.
fn device_guest_owner(bundle_bytes: &[u8], device: &str) -> Option<String> {
    let bundle: serde_json::Value = serde_json::from_slice(bundle_bytes).ok()?;
    for resource in bundle.get("resources")?.as_array()? {
        if resource.get("type").and_then(serde_json::Value::as_str) != Some("Device") {
            continue;
        }
        let Some(metadata) = resource.get("metadata") else {
            continue;
        };
        if metadata.get("name").and_then(serde_json::Value::as_str) != Some(device) {
            continue;
        }
        let owner = metadata
            .get("ownerRef")
            .and_then(serde_json::Value::as_str)?;
        return owner
            .strip_prefix("Guest/")
            .map(str::to_owned)
            .filter(|guest| !guest.is_empty());
    }
    None
}

/// Whether one Device-owned worker role binds its socket under the broker
/// runtime root's per-Guest directory.
fn binds_runtime_socket(role: &ProcessRole) -> bool {
    matches!(
        role,
        ProcessRole::Swtpm | ProcessRole::Gpu | ProcessRole::GpuRenderNode
    )
}

/// Pin the Device scope of one Device-owned worker launch.
///
/// The launched row is resolved from the verified Zone resource bundle the
/// request's `zone_uid` names (`Process.metadata.ownerRef`), that owner must
/// be a `Device`, `owner_ref` must be exactly it, and `owner_uid` must be
/// that Device row's durable uid. Only then is the Device's declared Guest
/// read. Every refusal is fail-closed (the retired broker arm's
/// `resolve_launch_scope` fence).
fn resolve_launch_scope(
    resolver: &BundleResolver,
    resource_ref: &ResourceRef,
    zone_uid: &ResourceUid,
    owner_ref: Option<&ResourceRef>,
    owner_uid: Option<&ResourceUid>,
) -> Result<DeviceWorkerScope, OperationFailure> {
    let (zone, bundle_bytes) = zone_bundle_for_uid(resolver, zone_uid).ok_or_else(|| {
        OperationFailure::with_detail(INTENT_MISMATCH, "resource_ref: verified-bundle-row".to_owned())
    })?;
    let owning = row_owner_ref(
        bundle_bytes,
        resource_ref.resource_type().as_str(),
        resource_ref.name().as_str(),
    )
    .ok_or_else(|| {
        OperationFailure::with_detail(INTENT_MISMATCH, "resource_ref: bundle-row-owner".to_owned())
    })?;
    if owning.resource_type().as_str() != "Device" {
        return Err(OperationFailure::with_detail(
            INTENT_MISMATCH,
            format!(
                "resource_ref: bundle-row-owner-is-not-a-device ({})",
                owning.to_canonical_string()
            ),
        ));
    }
    let Some(owner_ref) = owner_ref else {
        return Err(OperationFailure::with_detail(
            INTENT_MISMATCH,
            "owner_ref: owning-device-of-launched-row".to_owned(),
        ));
    };
    if owner_ref != &owning {
        return Err(OperationFailure::with_detail(
            INTENT_MISMATCH,
            format!(
                "owner_ref: {} vs owning Device {}",
                owner_ref.to_canonical_string(),
                owning.to_canonical_string()
            ),
        ));
    }
    let owning_uid = deterministic_resource_uid(&zone, "Device", owning.name().as_str());
    match owner_uid {
        None => {
            return Err(OperationFailure::with_detail(
                INTENT_MISMATCH,
                format!("owner_uid: {}", owning_uid.to_canonical_string()),
            ));
        }
        Some(claimed) if claimed != &owning_uid => {
            return Err(OperationFailure::with_detail(
                INTENT_MISMATCH,
                format!(
                    "owner_uid: {} vs Device uid {}",
                    claimed.to_canonical_string(),
                    owning_uid.to_canonical_string()
                ),
            ));
        }
        Some(_) => {}
    }
    let guest = device_guest_owner(bundle_bytes, owning.name().as_str()).ok_or_else(|| {
        OperationFailure::with_detail(
            INTENT_MISMATCH,
            format!("resource_ref: device-guest-owner ({})", owning.to_canonical_string()),
        )
    })?;
    Ok(DeviceWorkerScope {
        zone_uid: zone_uid.clone(),
        device_ref: owning,
        device_uid: owning_uid,
        guest,
    })
}

/// What one launch arm resolved for a Device-owned worker row.
///
/// The default - no scope, no runtime socket - is what every launch whose
/// intent is not a Device-owned worker role resolves.
fn resolve_device_worker_launch(
    resolver: &BundleResolver,
    req: &SpawnRunnerRequest,
    intent: &ResolvedRunnerIntent,
) -> Result<DeviceWorkerLaunch, OperationFailure> {
    if !is_device_worker_role(&intent.role) {
        return Ok(DeviceWorkerLaunch::default());
    }
    let binds_runtime_socket = binds_runtime_socket(&intent.role);
    let (Some(resource_ref), Some(zone_uid)) = (req.resource_ref.as_ref(), req.zone_uid.as_ref())
    else {
        return Ok(DeviceWorkerLaunch {
            scope: None,
            binds_runtime_socket,
        });
    };
    let scope = resolve_launch_scope(
        resolver,
        resource_ref,
        zone_uid,
        req.owner_ref.as_ref(),
        req.owner_uid.as_ref(),
    )?;
    Ok(DeviceWorkerLaunch {
        scope: Some(scope),
        binds_runtime_socket,
    })
}

/// The directory one trusted storage row names, refusing any row whose
/// template is not an anchored absolute path.
fn storage_path(spec: &d2b_core::storage::StoragePathSpec) -> Option<PathBuf> {
    let path = PathBuf::from(spec.path_template.as_str());
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    Some(path)
}

/// The TPM Provider's own state Volume naming for one Device.
fn state_volume_name(device_uid: &ResourceUid) -> String {
    let short: String = device_uid
        .as_str()
        .bytes()
        .filter(|byte| byte.is_ascii_hexdigit())
        .take(32)
        .map(char::from)
        .collect();
    format!("device-{short}-tpm-state")
}

/// The trusted identity of one resource-backed `w1-swtpm` launch.
///
/// Every field is resolved from verified bundle artifacts - the Zone
/// resource bundle's `Device` row and the host storage contract - never from
/// the request's caller-supplied fields (the retired broker arm's
/// `resource_backed_swtpm_identity` derivation).
fn resource_backed_swtpm_identity(
    resolver: &BundleResolver,
    req: &SpawnRunnerRequest,
    device_worker: &DeviceWorkerLaunch,
) -> Option<ResourceBackedSwtpm> {
    if !matches!(req.role, RunnerRole::Swtpm | RunnerRole::SwtpmFlush) {
        return None;
    }
    let scope = device_worker.scope.as_ref()?;
    if scope.device_ref.resource_type().as_str() != "Device" {
        return None;
    }
    let (_zone, bundle_bytes) = zone_bundle_for_uid(resolver, &scope.zone_uid)?;
    let guest = device_guest_owner(bundle_bytes, scope.device_ref.name().as_str())?;
    let state_root = storage_root(resolver, &format!("path:swtpm-state:{guest}"))?;
    if storage_root(resolver, "path:tpm-state")? != state_root {
        return None;
    }
    Some(ResourceBackedSwtpm {
        guest,
        state_root,
        state_volume: Some(state_volume_name(&scope.device_uid)),
    })
}

/// The directory one trusted storage row names.
fn storage_root(resolver: &BundleResolver, id: &str) -> Option<PathBuf> {
    storage_path(resolver.find_storage_path_spec(id)?)
}

// ---------------------------------------------------------------------------
// The OpenPidfd handler
// ---------------------------------------------------------------------------

/// The handler of [`OPEN_PIDFD`].
struct OpenPidfdHandler;

#[async_trait]
impl OperationHandler for OpenPidfdHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: OpenPidfdRequest = typed_request(OPEN_PIDFD, &payload)?;
        if request.pid <= 0 || request.expected_start_time_ticks == 0 {
            return Err(OperationFailure::with_detail(
                KERNEL_REFUSED,
                "OpenPidfd: pid and expectedStartTimeTicks must be positive".to_owned(),
            ));
        }
        // A typed adoption names the trusted intent; the request's VM/role
        // identity must match it (the retired arm's intent fence). A legacy
        // adoption carries no intent reference and is validated by the
        // kernel's start-time check alone.
        if let Some(intent_ref) = request.bundle_runner_intent_ref.as_ref() {
            let kernel = ctx
                .kernel
                .ok_or_else(|| OperationFailure::new(KERNEL_SEAM_UNWIRED))?;
            let intent = resolve_intent(kernel, OPEN_PIDFD, intent_ref.as_str())?;
            if request.vm_id.as_str() != intent.vm_name
                || request.role_id.as_str() != wire_role_id_for_intent(&intent)
            {
                return Err(OperationFailure::with_detail(
                    INTENT_MISMATCH,
                    format!(
                        "OpenPidfd: {}:{} does not match the trusted intent {}:{}",
                        request.vm_id.as_str(),
                        request.role_id.as_str(),
                        intent.vm_name,
                        wire_role_id_for_intent(&intent)
                    ),
                ));
            }
        }
        let reply = invoke_kernel_nested(
            &ctx,
            "open-pidfd",
            serde_json::json!({
                "pid": request.pid,
                "expectedStartTimeTicks": request.expected_start_time_ticks,
            }),
            Vec::new(),
        )
        .await?;
        let result = kernel_result(&reply)?;
        let pid = result.get("pid").and_then(serde_json::Value::as_i64).ok_or_else(|| {
            OperationFailure::with_detail(KERNEL_REFUSED, "open-pidfd: result pid missing".to_owned())
        })? as i32;
        let verified = result
            .get("verifiedStartTimeTicks")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| {
                OperationFailure::with_detail(
                    KERNEL_REFUSED,
                    "open-pidfd: result verifiedStartTimeTicks missing".to_owned(),
                )
            })? as u64;
        let pidfd = take_reply_fd(reply, 0)?;
        let response = OpenPidfdResponse {
            vm_id: request.vm_id.clone(),
            role_id: request.role_id.clone(),
            pid,
            verified_start_time_ticks: verified,
            pidfd_index: 0,
            controller_bootstrap_fd_index: None,
        };
        Ok(OperationResult::with_fds(
            typed_result(OPEN_PIDFD, &response)?,
            vec![pidfd],
        ))
    }
}

// ---------------------------------------------------------------------------
// The OpenPeerPidfdFromAcceptedSocket handler
// ---------------------------------------------------------------------------

/// The handler of [`OPEN_PEER_PIDFD_FROM_ACCEPTED_SOCKET`].
struct OpenPeerPidfdHandler;

#[async_trait]
impl OperationHandler for OpenPeerPidfdHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let _request: OpenPeerPidfdFromAcceptedSocketRequest =
            typed_request(OPEN_PEER_PIDFD_FROM_ACCEPTED_SOCKET, &payload)?;
        let socket = ctx.fds.first().ok_or_else(|| {
            OperationFailure::with_detail(
                KERNEL_REFUSED,
                "OpenPeerPidfdFromAcceptedSocket: no accepted socket attached".to_owned(),
            )
        })?;
        // The attached descriptor belongs to the transport's frame; the
        // kernel leg needs its own duplicate to attach to the nested call.
        let socket = duplicate_fd(*socket, "OpenPeerPidfdFromAcceptedSocket")?;
        let reply = invoke_kernel_nested(
            &ctx,
            "open-peer-pidfd-from-accepted-socket",
            serde_json::json!({}),
            vec![socket],
        )
        .await?;
        let _result = kernel_result(&reply)?;
        let pidfd = take_reply_fd(reply, 0)?;
        let response = OpenPeerPidfdFromAcceptedSocketResponse { pidfd_index: 0 };
        Ok(OperationResult::with_fds(
            typed_result(OPEN_PEER_PIDFD_FROM_ACCEPTED_SOCKET, &response)?,
            vec![pidfd],
        ))
    }
}

// ---------------------------------------------------------------------------
// The ObserveRunner handler
// ---------------------------------------------------------------------------

/// The handler of [`OBSERVE_RUNNER`].
struct ObserveRunnerHandler;

#[async_trait]
impl OperationHandler for ObserveRunnerHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: ObserveRunnerRequest = typed_request(OBSERVE_RUNNER, &payload)?;
        let kernel = ctx
            .kernel
            .ok_or_else(|| OperationFailure::new(KERNEL_SEAM_UNWIRED))?;
        let intent = resolve_intent(kernel, OBSERVE_RUNNER, request.bundle_runner_intent_ref.as_str())?;
        let expected_role = runner_role_for_process_role(&intent.role).ok_or_else(|| {
            OperationFailure::with_detail(
                INTENT_MISMATCH,
                format!("ObserveRunner: role {:?} is not a wire runner role", intent.role),
            )
        })?;
        if request.vm_id.as_str() != intent.vm_name
            || request.role != expected_role
            || request.bundle_runner_intent_ref.as_str() != intent.intent_id
            || request.role_id.as_str() != wire_role_id_for_intent(&intent)
        {
            return Err(OperationFailure::with_detail(
                INTENT_MISMATCH,
                "ObserveRunner: runner observation intent mismatch".to_owned(),
            ));
        }
        // The daemon's pidfd table is the authoritative presence source: a
        // runner the daemon does not track is not present, exactly as the
        // retired arm answered a registry miss with an absent observation.
        let Some((pid, start_time_ticks)) = runner_lookup(kernel, request.vm_id.as_str(), request.role_id.as_str())
        else {
            let response = ObserveRunnerResponse {
                vm_id: request.vm_id.clone(),
                role_id: request.role_id.clone(),
                present: false,
                pid: 0,
                start_time_ticks: 0,
                cgroup_verified: false,
                executable_verified: false,
            };
            return Ok(OperationResult::new(typed_result(OBSERVE_RUNNER, &response)?));
        };
        let typed = request.resource_ref.is_some();
        let cgroup_placement = private_cgroup_placement(
            &intent.cgroup_placement,
            request.vm_id.as_str(),
            request.runtime_scope,
            typed,
        )?;
        let reply = invoke_kernel_nested(
            &ctx,
            "observe-process",
            serde_json::json!({ "pid": pid }),
            Vec::new(),
        )
        .await?;
        let result = kernel_result(&reply)?;
        let present = result
            .get("present")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let observed_ticks = result
            .get("startTimeTicks")
            .and_then(serde_json::Value::as_i64)
            .map(|value| value as u64);
        let executable = result
            .get("executable")
            .and_then(serde_json::Value::as_str);
        let response = ObserveRunnerResponse {
            vm_id: request.vm_id.clone(),
            role_id: request.role_id.clone(),
            present: present && observed_ticks == Some(start_time_ticks),
            pid,
            start_time_ticks,
            cgroup_verified: proc_cgroup_matches(pid, &cgroup_placement.subtree),
            executable_verified: executable_matches(executable, &intent.binary_path),
        };
        Ok(OperationResult::new(typed_result(OBSERVE_RUNNER, &response)?))
    }
}

// ---------------------------------------------------------------------------
// The PollChildReaped handler
// ---------------------------------------------------------------------------

/// The handler of [`POLL_CHILD_REAPED`].
///
/// The daemon's own per-entry reap probing (the composition point's
/// pidfd-table walk invoking the `poll-child-reaped` kernel per entry) is the
/// live drain path; this declared family operation answers the drain shape
/// with the entries the daemon tracks. The family handler holds no pidfds
/// (the runner lookup carries `(pid, start_time_ticks)` only), so it serves
/// the surface with an empty notification set: every reaped child the daemon
/// owns is drained by the per-entry walk, never through this operation.
struct PollChildReapedHandler;

#[async_trait]
impl OperationHandler for PollChildReapedHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let _ = ctx;
        let _request: serde_json::Value = typed_request(POLL_CHILD_REAPED, &payload)?;
        let response = PollChildReapedResponse {
            notifications: Vec::new(),
        };
        Ok(OperationResult::new(typed_result(
            POLL_CHILD_REAPED,
            &response,
        )?))
    }
}

// ---------------------------------------------------------------------------
// The PrepareRuntimeDir / PrepareStateDir handlers
// ---------------------------------------------------------------------------

/// The handler of [`PREPARE_RUNTIME_DIR`].
struct PrepareRuntimeDirHandler;

#[async_trait]
impl OperationHandler for PrepareRuntimeDirHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        prepare_directory_handler(&ctx, &payload, PREPARE_RUNTIME_DIR, true).await
    }
}

/// The handler of [`PREPARE_STATE_DIR`].
struct PrepareStateDirHandler;

#[async_trait]
impl OperationHandler for PrepareStateDirHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        prepare_directory_handler(&ctx, &payload, PREPARE_STATE_DIR, false).await
    }
}

/// The shared prepare-directory leg of the two prepare operations.
async fn prepare_directory_handler(
    ctx: &OperationCtx<'_>,
    payload: &ValidatedPayload,
    operation: &'static str,
    runtime_dir: bool,
) -> Result<OperationResult, OperationFailure> {
    let request: PrepareDirRequest = typed_request(operation, payload)?;
    let kernel = ctx
        .kernel
        .ok_or_else(|| OperationFailure::new(KERNEL_SEAM_UNWIRED))?;
    let intent = kernel
        .bundle
        .resolve_prepare_dir_intent(request.vm_id.as_str(), runtime_dir)
        .ok_or_else(|| {
            OperationFailure::with_detail(
                KERNEL_REFUSED,
                format!("{operation}: unknown subject {:?}", request.vm_id.as_str()),
            )
        })?;
    let reply = invoke_kernel_nested(
        ctx,
        "prepare-directory",
        serde_json::json!({
            "kind": if runtime_dir { "runtime" } else { "state" },
            "baseDir": intent.base_dir.display().to_string(),
            "vmIdOrScope": intent.vm_name,
            "mode": intent.mode,
            "ownerUid": intent.owner_uid,
            "ownerGid": intent.owner_gid,
            "createdPaths": [],
        }),
        Vec::new(),
    )
    .await?;
    let _result = kernel_result(&reply)?;
    // The retired arms acknowledged the prepare with an empty body; the
    // family surface keeps that shape.
    let result = CanonicalJsonObject::parse(b"{}").expect("the empty object is canonical JSON");
    Ok(OperationResult::new(result))
}

// ---------------------------------------------------------------------------
// The CgroupKill handler
// ---------------------------------------------------------------------------

/// The handler of [`CGROUP_KILL`].
struct CgroupKillHandler;

#[async_trait]
impl OperationHandler for CgroupKillHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: CgroupKillRequest = typed_request(CGROUP_KILL, &payload)?;
        let kernel = ctx
            .kernel
            .ok_or_else(|| OperationFailure::new(KERNEL_SEAM_UNWIRED))?;
        let cgroup_path = resolve_runner_cgroup_leaf(kernel, &request)?;
        let reply = invoke_kernel_nested(
            &ctx,
            "kill-cgroup",
            serde_json::json!({ "cgroupPath": cgroup_path.display().to_string() }),
            Vec::new(),
        )
        .await?;
        let _result = kernel_result(&reply)?;
        let result = CanonicalJsonObject::parse(b"{}").expect("the empty object is canonical JSON");
        Ok(OperationResult::new(result))
    }
}

/// Resolve the trusted runner cgroup leaf of one `CgroupKill` request.
///
/// Mirrors the retired broker arm's `resolve_runner_cgroup_leaf`: the leaf is
/// resolved from the trusted bundle's runner intents, refusing an ambiguous
/// or non-canonical placement.
fn resolve_runner_cgroup_leaf(
    kernel: &KernelCaller,
    req: &CgroupKillRequest,
) -> Result<PathBuf, OperationFailure> {
    let mut matching_intent = None;
    for intent_id in kernel.bundle.runner_intent_ids() {
        let Some(intent) = kernel.bundle.find_runner_intent(intent_id) else {
            continue;
        };
        if intent.vm_name == req.vm_id.as_str()
            && runner_role_matches(&intent.role_id, req.role_id.as_str())
            && runner_cgroup_shape(&intent.cgroup_placement.subtree, req.vm_id.as_str()).is_some()
        {
            if matching_intent.is_some() {
                return Err(OperationFailure::with_detail(
                    KERNEL_REFUSED,
                    "CgroupKill: ambiguous trusted runner cgroup intent".to_owned(),
                ));
            }
            matching_intent = Some(intent);
        }
    }
    let intent = matching_intent.ok_or_else(|| {
        OperationFailure::with_detail(
            RUNNER_UNKNOWN,
            format!(
                "CgroupKill: unknown subject {}:{}",
                req.vm_id.as_str(),
                req.role_id.as_str()
            ),
        )
    })?;
    let components = Path::new(&intent.cgroup_placement.subtree)
        .components()
        .collect::<Vec<_>>();
    let segment = |index: usize| {
        components.get(index).and_then(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
    };
    let Some(role_index) = runner_cgroup_shape(&intent.cgroup_placement.subtree, req.vm_id.as_str())
    else {
        return Err(OperationFailure::with_detail(
            KERNEL_REFUSED,
            "CgroupKill: runner-cgroup-leaf-invalid".to_owned(),
        ));
    };
    let role_matches_path = segment(role_index) == Some(intent.role_id.as_str())
        || (req.role_id.as_str() == "ch-runner"
            && intent.role_id == "cloud-hypervisor"
            && segment(role_index) == Some("cloud-hypervisor"));
    if !role_matches_path {
        return Err(OperationFailure::with_detail(
            KERNEL_REFUSED,
            "CgroupKill: runner-cgroup-leaf-invalid".to_owned(),
        ));
    }
    Ok(Path::new("/sys/fs/cgroup").join(&intent.cgroup_placement.subtree))
}

/// Whether one requested role names one trusted intent role.
fn runner_role_matches(intent_role: &str, requested_role: &str) -> bool {
    intent_role == requested_role
        || (requested_role == "ch-runner" && intent_role == "cloud-hypervisor")
}

/// The canonical runner cgroup shape of one trusted subtree.
fn runner_cgroup_shape(subtree: &str, vm_id: &str) -> Option<usize> {
    let components = Path::new(subtree).components().collect::<Vec<_>>();
    if components.len() == 4
        && matches!(components[0], std::path::Component::Normal(value) if value == "d2b.slice")
        && matches!(components[2], std::path::Component::Normal(value) if value == vm_id)
    {
        Some(3)
    } else if components.len() == 3
        && matches!(components[0], std::path::Component::Normal(value) if value == "d2b.slice")
        && matches!(components[1], std::path::Component::Normal(value) if value == vm_id)
        && matches!(components[2], std::path::Component::Normal(_))
    {
        Some(2)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// The SignalRunner handler
// ---------------------------------------------------------------------------

/// The handler of [`SIGNAL_RUNNER`].
struct SignalRunnerHandler;

#[async_trait]
impl OperationHandler for SignalRunnerHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: SignalRunnerRequest = typed_request(SIGNAL_RUNNER, &payload)?;
        let kernel = ctx
            .kernel
            .ok_or_else(|| OperationFailure::new(KERNEL_SEAM_UNWIRED))?;
        if !typed_control_identity_complete(
            request.resource_ref.as_ref(),
            request.resource_uid.as_ref(),
            request.zone_uid.as_ref(),
            request.generation,
            request.runtime_scope,
            request.provider_ref.as_ref(),
            request.provider_identity,
            request.template_identity,
        ) {
            return Err(OperationFailure::with_detail(
                RUNNER_UNKNOWN,
                format!(
                    "SignalRunner: incomplete typed identity for {}:{}",
                    request.vm_id.as_str(),
                    request.role_id.as_str()
                ),
            ));
        }
        // The daemon's pidfd table is the authoritative runner source. The
        // kernel signals through a pidfd, so the handler derives one from the
        // tracked `(pid, start_time_ticks)` via the open-pidfd kernel - the
        // retired arm's open-then-signal fallback - and signals through it.
        let Some((pid, start_time_ticks)) = runner_lookup(
            kernel,
            request.vm_id.as_str(),
            request.role_id.as_str(),
        ) else {
            return Err(OperationFailure::with_detail(
                RUNNER_UNKNOWN,
                format!(
                    "SignalRunner: no tracked runner {}:{}",
                    request.vm_id.as_str(),
                    request.role_id.as_str()
                ),
            ));
        };
        let open_reply = invoke_kernel_nested(
            &ctx,
            "open-pidfd",
            serde_json::json!({
                "pid": pid,
                "expectedStartTimeTicks": start_time_ticks,
            }),
            Vec::new(),
        )
        .await?;
        let _open_result = kernel_result(&open_reply)?;
        let pidfd = take_reply_fd(open_reply, 0)?;
        // The POSIX signal numbers the kernel's `signal-pidfd` payload names
        // (SIGTERM/SIGKILL/SIGQUIT, the same mapping the retired arm used).
        let signal = match request.signal {
            RunnerSignal::Term => 15,
            RunnerSignal::Kill => 9,
            RunnerSignal::Quit => 3,
        };
        let reply = invoke_kernel_nested(
            &ctx,
            "signal-pidfd",
            serde_json::json!({ "signal": signal }),
            vec![pidfd],
        )
        .await?;
        let result = kernel_result(&reply)?;
        let signaled = result
            .get("signaled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let response = SignalRunnerResponse {
            signaled,
            vm_id: request.vm_id.clone(),
            role_id: request.role_id.clone(),
        };
        Ok(OperationResult::new(typed_result(SIGNAL_RUNNER, &response)?))
    }
}

// ---------------------------------------------------------------------------
// The DeregisterRunnerPidfd handler
// ---------------------------------------------------------------------------

/// The handler of [`DEREGISTER_RUNNER_PIDFD`].
struct DeregisterRunnerPidfdHandler;

#[async_trait]
impl OperationHandler for DeregisterRunnerPidfdHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: DeregisterRunnerPidfdRequest = typed_request(DEREGISTER_RUNNER_PIDFD, &payload)?;
        let reply = invoke_kernel_nested(
            &ctx,
            "deregister-pidfd",
            serde_json::json!({}),
            Vec::new(),
        )
        .await?;
        let result = kernel_result(&reply)?;
        let removed = result
            .get("removed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let response = DeregisterRunnerPidfdResponse {
            vm_id: request.vm_id.clone(),
            role_id: request.role_id.clone(),
            removed,
        };
        Ok(OperationResult::new(typed_result(
            DEREGISTER_RUNNER_PIDFD,
            &response,
        )?))
    }
}

// ---------------------------------------------------------------------------
// The SpawnRunner handler
// ---------------------------------------------------------------------------

/// The handler of [`SPAWN_RUNNER`].
struct SpawnRunnerHandler;

#[async_trait]
impl OperationHandler for SpawnRunnerHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: SpawnRunnerRequest = typed_request(SPAWN_RUNNER, &payload)?;
        let kernel = ctx
            .kernel
            .ok_or_else(|| OperationFailure::new(KERNEL_SEAM_UNWIRED))?;
        let intent = resolve_intent(kernel, SPAWN_RUNNER, request.bundle_runner_intent_ref.as_str())?;
        let posture = LaunchPosture::resolve(request.role, &intent);
        posture.validate_request_fds(request.inherited_fd_count, ctx.fds.len())?;
        // Device-owned worker launches derive every runtime path from the
        // Device that owns the launched row; that Device is resolved and
        // pinned here, from the verified bundle, before any Device-derived
        // identity is trusted (the retired arm's posture).
        let device_worker = resolve_device_worker_launch(&kernel.bundle, &request, &intent)?;
        if request.resource_ref.is_some() {
            let Some(bundle_content_identity) = request.bundle_content_identity.as_deref() else {
                return Err(OperationFailure::with_detail(
                    INTENT_MISMATCH,
                    "bundle_content_identity: required".to_owned(),
                ));
            };
            let resolved = kernel.bundle.bundle.bundle_hash.as_deref().ok_or_else(|| {
                OperationFailure::with_detail(
                    INTENT_MISMATCH,
                    "bundle_content_identity: missing".to_owned(),
                )
            })?;
            if bundle_content_identity != resolved {
                return Err(OperationFailure::with_detail(
                    INTENT_MISMATCH,
                    format!(
                        "bundle_content_identity: {bundle_content_identity} vs {resolved}"
                    ),
                ));
            }
        }
        if request.generation.is_some_and(|generation| generation == 0) {
            return Err(OperationFailure::with_detail(
                INTENT_MISMATCH,
                "generation: 0 vs nonzero".to_owned(),
            ));
        }
        match (&request.activation_input, request.role) {
            (Some(input), RunnerRole::ActivationNixos) => {
                if input.target_generation == 0 {
                    return Err(OperationFailure::with_detail(
                        INTENT_MISMATCH,
                        "activation_input.target_generation: 0 vs nonzero".to_owned(),
                    ));
                }
                if request.generation.is_none() {
                    return Err(OperationFailure::with_detail(
                        INTENT_MISMATCH,
                        "generation: required-for-activation-input".to_owned(),
                    ));
                }
                let encoded = serde_json::to_vec(input).map_err(|_| {
                    OperationFailure::with_detail(
                        INTENT_MISMATCH,
                        "activation_input: unserializable".to_owned(),
                    )
                })?;
                if encoded.len()
                    > d2b_contracts_resource::v3::MAX_ACTIVATION_RUNNER_INPUT_BYTES
                {
                    return Err(OperationFailure::with_detail(
                        INTENT_MISMATCH,
                        format!(
                            "activation_input: {} exceeds the bounded stdin envelope",
                            encoded.len()
                        ),
                    ));
                }
            }
            (Some(_), _) => {
                return Err(OperationFailure::with_detail(
                    INTENT_MISMATCH,
                    "activation_input: activation-nixos-role-only".to_owned(),
                ));
            }
            (None, RunnerRole::ActivationNixos) => {
                return Err(OperationFailure::with_detail(
                    INTENT_MISMATCH,
                    "activation_input: required-for-activation-nixos".to_owned(),
                ));
            }
            (None, _) => {}
        }
        if request.resource_ref.is_some()
            && (request.execution_ref.is_none()
                || request.execution_domain.is_none()
                || request.resource_uid.is_none()
                || request
                    .provider_identity
                    .is_none_or(|identity| identity == [0; 32])
                || request
                    .template_identity
                    .is_none_or(|identity| identity == [0; 32])
                || request.generation.is_none())
        {
            return Err(OperationFailure::with_detail(
                INTENT_MISMATCH,
                "process_identity: resource/provider/template/generation-required".to_owned(),
            ));
        }
        validate_spawn_runner_request_matches_intent(
            &request,
            &intent,
            posture,
            device_worker.scope.as_ref(),
        )?;
        let typed_process = request.resource_ref.is_some();
        let cgroup_placement = private_cgroup_placement(
            &intent.cgroup_placement,
            request.vm_id.as_str(),
            request.runtime_scope,
            typed_process,
        )?;
        if let Some(plan) = &request.sandbox_plan {
            validate_sandbox_launch_plan(&request, &intent, plan)?;
        }
        // The OtelHostBridge runner must target the obs VM the manifest
        // declares; any other target would let a tampered bundle redirect
        // host OTLP egress (the retired arm's fail-closed fence).
        if matches!(request.role, RunnerRole::OtelHostBridge)
            && intent.vm_name != kernel.bundle.manifest.observability.vm_name
        {
            return Err(OperationFailure::with_detail(
                INTENT_MISMATCH,
                format!(
                    "OtelHostBridge: intent vm {} does not match the obs VM {}",
                    intent.vm_name, kernel.bundle.manifest.observability.vm_name
                ),
            ));
        }
        // The bundle-declared vm-start prerequisites prepare the runtime and
        // state directories through the same prepare-directory kernel the
        // dedicated family operations use (the retired arm's
        // `apply_vm_start_prerequisites`).
        for prerequisite in kernel
            .bundle
            .resolve_vm_start_prerequisites(request.vm_id.as_str(), request.role_id.as_str())
        {
            for action in &prerequisite.actions {
                let (kind, dir) = match action {
                    d2b_core::bundle_resolver::ResolvedVmStartAction::PrepareRuntimeDir(dir) => {
                        ("runtime", dir)
                    }
                    d2b_core::bundle_resolver::ResolvedVmStartAction::PrepareStateDir(dir) => {
                        ("state", dir)
                    }
                };
                let reply = invoke_kernel_nested(
                    &ctx,
                    "prepare-directory",
                    serde_json::json!({
                        "kind": kind,
                        "baseDir": dir.base_dir.display().to_string(),
                        "vmIdOrScope": prerequisite.vm_name,
                        "mode": dir.mode,
                        "ownerUid": dir.owner_uid,
                        "ownerGid": dir.owner_gid,
                        "createdPaths": [],
                    }),
                    Vec::new(),
                )
                .await?;
                let _result = kernel_result(&reply)?;
            }
        }
        // The bundle resolver is the sole authority for trusted runner argv.
        let mount_policy = intent.mount_policy.clone();
        let mut env = intent.env.clone();
        extend_audio_runner_pipewire_props(
            request.vm_id.as_str(),
            request.role_id.as_str(),
            request.role,
            &mut env,
        )?;
        let launch_argv = match request.launch_args.as_ref() {
            Some(launch_args) => {
                let mut argv = Vec::with_capacity(launch_args.as_slice().len() + 1);
                argv.push(intent.binary_path.to_string_lossy().into_owned());
                argv.extend(launch_args.as_slice().iter().cloned());
                argv
            }
            None => intent.argv.clone(),
        };
        let argv = bind_cloud_hypervisor_guest_uid(request.role, request.owner_uid.as_ref(), &launch_argv)?;
        // The launch identity is the trusted intent's principal for every
        // posture (the retired arm's `prepare_runner_launch_identity`).
        let runner_uid = intent.uid;
        let runner_gid = intent.gid;
        let runner_user_namespace = intent.user_namespace.map(|spec| {
            serde_json::json!({
                "hostUidForZero": spec.host_uid_for_zero,
                "hostGidForZero": spec.host_gid_for_zero,
            })
        });
        let swtpm_identity = resource_backed_swtpm_identity(&kernel.bundle, &request, &device_worker);
        let reply = invoke_kernel_nested(
            &ctx,
            "spawn-process",
            serde_json::json!({
                "binaryPath": intent.binary_path.display().to_string(),
                "argv": argv,
                "uid": runner_uid,
                "gid": runner_gid,
                "supplementaryGroups": intent.supplementary_groups,
                "env": env,
                "capabilities": intent.capabilities,
                "namespaces": intent.namespaces,
                "seccompPolicyRef": intent.seccomp_policy_ref,
                "mountPolicy": mount_policy,
                "cgroupPlacement": cgroup_placement,
                "rootCarveOut": intent.root_carve_out,
                "skipBinaryExistsCheck": false,
                "userNamespace": runner_user_namespace,
                "umask": intent.umask,
                "activationInput": request.activation_input,
                "swtpmIdentity": swtpm_identity.map(|identity| serde_json::json!({
                    "guest": identity.guest,
                    "stateRoot": identity.state_root.display().to_string(),
                    "stateVolume": identity.state_volume,
                })),
                "deviceWorker": {
                    "scope": device_worker.scope.as_ref().map(|scope| serde_json::json!({
                        "zoneUid": scope.zone_uid.as_str(),
                        "deviceRef": scope.device_ref.to_canonical_string(),
                        "deviceUid": scope.device_uid.as_str(),
                        "guest": scope.guest,
                    })),
                    "bindsRuntimeSocket": device_worker.binds_runtime_socket,
                },
                // The launch role, the serving-worker posture (resolved
                // from the trusted intent exactly once) and the runner
                // identity the kernel needs for the retired arm's
                // in-broker behaviors: the USBIP backend device-bind
                // extension, the serving-worker ACL grant, the stale
                // socket cleanups, the duplicate-runner guard and the
                // runner-id-keyed registration.
                "role": request.role,
                "servingWorker": posture.is_serving_worker(),
                "runnerIdentity": {
                    "vmId": request.vm_id.as_str(),
                    "roleId": request.role_id.as_str(),
                    "resourceRef": request.resource_ref.as_ref().map(|reference| reference.to_canonical_string()),
                    "resourceUid": request.resource_uid.as_ref().map(|uid| uid.as_str()),
                    "zoneUid": request.zone_uid.as_ref().map(|uid| uid.as_str()),
                    "generation": request.generation,
                    "runtimeScope": request.runtime_scope.map(|scope| scope.to_vec()),
                    "ownerRef": request.owner_ref.as_ref().map(|reference| reference.to_canonical_string()),
                    "providerRef": request.provider_ref.as_ref().map(|reference| reference.to_canonical_string()),
                    "providerIdentity": request.provider_identity.map(|identity| identity.to_vec()),
                    "templateIdentity": request.template_identity.map(|identity| identity.to_vec()),
                    "bundleRunnerIntentRef": request.bundle_runner_intent_ref.as_str(),
                    "guestExecution": request.guest_execution,
                },
            }),
            Vec::new(),
        )
        .await?;
        let result = kernel_result(&reply)?;
        let pid = result.get("pid").and_then(serde_json::Value::as_i64).ok_or_else(|| {
            OperationFailure::with_detail(KERNEL_REFUSED, "spawn-process: result pid missing".to_owned())
        })? as i32;
        let start_time_ticks = result
            .get("startTimeTicks")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| {
                OperationFailure::with_detail(
                    KERNEL_REFUSED,
                    "spawn-process: result startTimeTicks missing".to_owned(),
                )
            })? as u64;
        let pidfd = take_reply_fd(reply, 0)?;
        // The kernel returns the pidfd alone today: the retired arm's
        // controller-bootstrap duplicate and console descriptor were
        // broker-side response enrichments the kernel path does not mint, so
        // the response advertises no extra indices.
        let response = SpawnRunnerResponse {
            vm_id: request.vm_id.clone(),
            role_id: request.role_id.clone(),
            role: request.role,
            resource_ref: request.resource_ref.clone(),
            resource_uid: request.resource_uid.clone(),
            zone_uid: request.zone_uid.clone(),
            owner_ref: request.owner_ref.clone(),
            runtime_scope: request.runtime_scope,
            execution_ref: request.execution_ref.clone(),
            execution_domain: request.execution_domain,
            user_ref: request.user_ref.clone(),
            guest_execution: request.guest_execution.clone(),
            provider_identity: request.provider_identity,
            template_identity: request.template_identity,
            generation: request.generation,
            bundle_content_identity: request.bundle_content_identity.clone(),
            pid,
            start_time_ticks,
            pidfd_index: 0,
            controller_bootstrap_fd_index: None,
            console_fd_index: None,
        };
        Ok(OperationResult::with_fds(
            typed_result(SPAWN_RUNNER, &response)?,
            vec![pidfd],
        ))
    }
}