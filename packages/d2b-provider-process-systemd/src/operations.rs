//! The family's declared broker operations (U15).
//!
//! The five committed process-systemd rows (`StartSystemdUnit`,
//! `CheckSystemdUserManager`, `ObserveSystemdUnit`, `OpenSystemdUnitPidfd`,
//! `StopSystemdUnit`) stop being served by a broker module: this crate
//! declares them in its `operations.json` and serves them through the
//! provider-operation table of [`crate::systemd_descriptor`] over the
//! broker's forward seam. Each handler validates the typed request the
//! envelope forwarded (the same wire shape the retired broker arms
//! consumed) against the Zone's trusted bundle through the per-zone kernel
//! seam, performs the service-manager calls itself (the daemon process
//! owns the same D-Bus authority the retired broker arms presented), and
//! opens the exact-main pidfd through the broker-generic `open-pidfd`
//! kernel as a nested envelope call - the same kernel row the process
//! family's `OpenPidfd` handler invokes, so the pidfd evidence chain and
//! the in-broker correlation record are the ones the retired arms'
//! `live_open_pidfd` produced (KTD6).
//!
//! The closed refusal vocabulary is unchanged from the retired module: each
//! handler refuses with the same unit code the broker's `UnitError`
//! spelled, the nested kernel leg preserves the kernel's own closed code,
//! and a Zone whose seam was never wired refuses with
//! [`KERNEL_SEAM_UNWIRED`].

use std::num::NonZeroU32;
use std::os::fd::OwnedFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use d2b_contracts_broker::broker_wire::{
    OpenUnitPidfdRequest, SandboxLaunchPlan, StartTransientUnitRequest, StopUnitRequest,
    UnitStopClass, UnitDomain, UnitIdentity,
};
use d2b_contracts_broker::kernel_client::{
    KernelInvocation, KernelInvokeError, envelope_invoke_kernel,
};
use d2b_contracts_resource::v3::{CanonicalJsonObject, canonical_json_bytes};
use d2b_core::bundle_resolver::{BundleResolver, ResolvedRunnerIntent};
use d2b_core::kernel_seat;
use d2b_resource_types::{
    OperationCtx, OperationFailure, OperationHandler, OperationResult, ValidatedPayload,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use zbus::connection;
use zbus::zvariant::{OwnedObjectPath, Value};
use zbus::{Address, Connection, Proxy};

/// The family's `StartSystemdUnit` operation.
pub const START_SYSTEMD_UNIT: &str = "StartSystemdUnit";

/// The family's `CheckSystemdUserManager` operation.
pub const CHECK_SYSTEMD_USER_MANAGER: &str = "CheckSystemdUserManager";

/// The family's `ObserveSystemdUnit` operation.
pub const OBSERVE_SYSTEMD_UNIT: &str = "ObserveSystemdUnit";

/// The family's `OpenSystemdUnitPidfd` operation.
pub const OPEN_SYSTEMD_UNIT_PIDFD: &str = "OpenSystemdUnitPidfd";

/// The family's `StopSystemdUnit` operation.
pub const STOP_SYSTEMD_UNIT: &str = "StopSystemdUnit";

/// The refusal of a family operation whose kernel seam was never wired.
///
/// The composition point wires one [`KernelCaller`] per Zone alongside the
/// provider publication; a Zone whose seam was never wired serves forwarded
/// family operations without a kernel leg and every handler that needs one
/// refuses with this code.
pub const KERNEL_SEAM_UNWIRED: &str = "kernel-seam-unwired";

/// The refusal of a request that was structurally invalid or named the wrong
/// role (the retired module's `UnitError::InvalidRequest` spelling).
pub const UNIT_INVALID_REQUEST: &str = "unit-invalid-request";

/// The refusal of a request whose trusted bundle runner intent was absent or
/// inconsistent (the retired module's `UnitError::BundleIntent`).
pub const UNIT_BUNDLE_INTENT: &str = "unit-bundle-intent";

/// The refusal of a per-user manager that is unavailable or failed same-UID
/// verification (the retired module's `UnitError::UserManagerUnavailable`).
pub const USER_MANAGER_UNAVAILABLE: &str = "user-manager-unavailable";

/// The refusal of a system manager or unit query failure (the retired
/// module's `UnitError::Query`).
pub const UNIT_QUERY_FAILED: &str = "unit-query-failed";

/// The refusal of a transient unit that could not be started (the retired
/// module's `UnitError::Start`).
pub const UNIT_START_FAILED: &str = "unit-start-failed";

/// The refusal of a unit identity that did not match the trusted request
/// (the retired module's `UnitError::IdentityMismatch`).
pub const UNIT_IDENTITY_MISMATCH: &str = "unit-identity-mismatch";

/// The refusal of an exact-main pidfd that could not be opened (the retired
/// module's `UnitError::Pidfd`).
pub const UNIT_PIDFD_FAILED: &str = "unit-pidfd-failed";

/// The refusal of a unit that could not be stopped and verified inactive
/// (the retired module's `UnitError::Stop`).
pub const UNIT_STOP_FAILED: &str = "unit-stop-failed";

/// The refusal of the bounded identity wait expiring (the retired module's
/// `UnitError::Timeout`).
pub const UNIT_IDENTITY_TIMEOUT: &str = "unit-identity-timeout";

/// The private service-manager destinations the family addresses.
const MANAGER_DESTINATION: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER_INTERFACE: &str = "org.freedesktop.systemd1.Manager";
const UNIT_INTERFACE: &str = "org.freedesktop.systemd1.Unit";
// ControlGroup and MainPID are defined on the unit-kind interfaces
// (Service for the family's transient service units), not on Unit; the
// identity read must address them through this interface (issue #587).
const SERVICE_INTERFACE: &str = "org.freedesktop.systemd1.Service";

/// The systemd object interface a unit-identity property is defined on.
///
/// `ControlGroup` and `MainPID` are defined on `org.freedesktop.systemd1.Service`
/// (the family's transient service units), not on `Unit`; reading them through
/// the Unit proxy fails every read with `UnknownProperty` and the identity can
/// never bind (issue #587). `ActiveState` and `InvocationID` are defined on
/// `org.freedesktop.systemd1.Unit`. `read_identity` routes every property read
/// through this selection.
fn identity_property_interface(property: &str) -> &'static str {
    match property {
        "ControlGroup" | "MainPID" => SERVICE_INTERFACE,
        _ => UNIT_INTERFACE,
    }
}
const METHOD_TIMEOUT: Duration = Duration::from_secs(5);
const IDENTITY_READY_TIMEOUT: Duration = Duration::from_secs(5);
const IDENTITY_RETRY_INTERVAL: Duration = Duration::from_millis(20);

/// The broker kernel IO budget one nested invocation may take.
const KERNEL_IO_TIMEOUT: Duration = Duration::from_secs(10);

/// The family's declared operations, assembled once.
///
/// The handler table is the declaration itself: the descriptor this crate
/// publishes carries the five operation handlers, and the daemon's registry
/// serves them from there. There is no second registration step.
pub fn process_systemd_family_operations() -> &'static [d2b_resource_types::OperationDef] {
    &PROCESS_SYSTEMD_FAMILY_OPERATIONS[..]
}

// The committed rows spell the family operations in the catalog's PascalCase
// wire names (`StartSystemdUnit`, ...), while a `ResourceRef` name is a
// lowercase label. The envelope matches the forwarded wire name
// case-insensitively (the U10/U12 seam), so the table declares each operation
// under its parseable lowercase reference; the public constants above keep
// the catalog's wire spellings.
static PROCESS_SYSTEMD_FAMILY_OPERATIONS: std::sync::LazyLock<
    [d2b_resource_types::OperationDef; 5],
> = std::sync::LazyLock::new(|| {
    [
        d2b_resource_types::OperationDef {
            operation_ref: d2b_contracts_resource::v3::ResourceRef::parse("Operation/start-systemd-unit")
            .expect("the family's operation reference is canonical"),
            handler: &START_SYSTEMD_UNIT_HANDLER,
        },
        d2b_resource_types::OperationDef {
            operation_ref: d2b_contracts_resource::v3::ResourceRef::parse("Operation/check-systemd-user-manager")
            .expect("the family's operation reference is canonical"),
            handler: &CHECK_SYSTEMD_USER_MANAGER_HANDLER,
        },
        d2b_resource_types::OperationDef {
            operation_ref: d2b_contracts_resource::v3::ResourceRef::parse("Operation/observe-systemd-unit")
            .expect("the family's operation reference is canonical"),
            handler: &OBSERVE_SYSTEMD_UNIT_HANDLER,
        },
        d2b_resource_types::OperationDef {
            operation_ref: d2b_contracts_resource::v3::ResourceRef::parse("Operation/open-systemd-unit-pidfd")
            .expect("the family's operation reference is canonical"),
            handler: &OPEN_SYSTEMD_UNIT_PIDFD_HANDLER,
        },
        d2b_resource_types::OperationDef {
            operation_ref: d2b_contracts_resource::v3::ResourceRef::parse("Operation/stop-systemd-unit")
            .expect("the family's operation reference is canonical"),
            handler: &STOP_SYSTEMD_UNIT_HANDLER,
        },
    ]
});

static START_SYSTEMD_UNIT_HANDLER: StartSystemdUnitHandler = StartSystemdUnitHandler;
static CHECK_SYSTEMD_USER_MANAGER_HANDLER: CheckSystemdUserManagerHandler =
    CheckSystemdUserManagerHandler;
static OBSERVE_SYSTEMD_UNIT_HANDLER: ObserveSystemdUnitHandler = ObserveSystemdUnitHandler;
static OPEN_SYSTEMD_UNIT_PIDFD_HANDLER: OpenSystemdUnitPidfdHandler = OpenSystemdUnitPidfdHandler;
static STOP_SYSTEMD_UNIT_HANDLER: StopSystemdUnitHandler = StopSystemdUnitHandler;

// ---------------------------------------------------------------------------
// Typed request fences (ported from the retired broker module)
// ---------------------------------------------------------------------------

/// Resolve the trusted runner intent for one unit request and verify every
/// caller-supplied identity field against it, exactly as the retired broker
/// module's `validate_request` did.
///
/// The daemon suffers no caller assertion here: the unit name, user, domain,
/// and sandbox posture are all resolved from the Zone's verified bundle, and
/// a request that names a different identity by field refuses fail-closed.
// The read is one short /proc read on a genuinely synchronous validation
// path (the retired broker arm read the same file async; this crate's
// request fence is sync, and the read has no async form at this site).
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn validate_request(
    bundle: &BundleResolver,
    request: &d2b_contracts_broker::broker_wire::UnitRequest,
) -> Result<ResolvedRunnerIntent, &'static str> {
    if request.generation == 0
        || request.provider_identity == [0; 32]
        || request.template_identity == [0; 32]
        || request.bundle_content_identity.is_empty()
    {
        return Err(UNIT_INVALID_REQUEST);
    }

    fn sandbox_supported(plan: &SandboxLaunchPlan) -> bool {
        plan.namespace_classes.is_empty()
            && plan.capability_classes.is_empty()
            && plan.seccomp_class.as_str() == "strict"
            && plan.no_new_privileges
            && !plan.start_root
            && matches!(
                plan.environment_class,
                d2b_contracts_resource::v3::process::EnvironmentClass::Minimal
            )
            && plan.read_only_root
            && plan.user_namespace.is_none()
    }
    let intent = bundle
        .find_runner_intent(request.bundle_runner_intent_ref.as_str())
        .ok_or(UNIT_BUNDLE_INTENT)?;
    if bundle.bundle.bundle_hash.as_deref() != Some(request.bundle_content_identity.as_str()) {
        return Err(UNIT_IDENTITY_MISMATCH);
    }
    if intent.vm_name != request.vm_id.as_str()
        || intent.role_id != request.role_id.as_str()
        || !role_matches(request.role, &intent.role)
        || intent.binary_path.as_os_str().is_empty()
        || !intent.binary_path.is_absolute()
        || intent.argv.is_empty()
    {
        return Err(UNIT_BUNDLE_INTENT);
    }
    let expected_execution = d2b_contracts_resource::v3::ResourceRef::parse(&intent.execution_ref)
        .map_err(|_| UNIT_IDENTITY_MISMATCH)?;
    if request
        .execution_ref
        .as_ref()
        .is_some_and(|execution| execution != &expected_execution)
    {
        return Err(UNIT_IDENTITY_MISMATCH);
    }
    let expected_domain = match intent.execution_domain {
        d2b_core::processes::ProcessExecutionDomain::System => UnitDomain::System,
        d2b_core::processes::ProcessExecutionDomain::User => UnitDomain::User,
    };
    if request.domain != expected_domain {
        return Err(UNIT_IDENTITY_MISMATCH);
    }
    if let Some(plan) = &request.sandbox_plan
        && !sandbox_supported(plan)
    {
        return Err(UNIT_INVALID_REQUEST);
    }
    let expected_user = intent
        .user_ref
        .as_deref()
        .map(d2b_contracts_resource::v3::ResourceRef::parse)
        .transpose()
        .map_err(|_| UNIT_IDENTITY_MISMATCH)?;
    if request.user_ref != expected_user {
        return Err(UNIT_IDENTITY_MISMATCH);
    }
    // The Guest execution binding gate (restored from the retired broker
    // arm's `validate_guest_process_binding`, which ran under the Guest
    // profile). It is two checks, and both belong here:
    //
    // 1. A request whose execution reference targets a Guest must carry a
    //    Guest execution binding - the profile gate required the binding,
    //    and the row's payload schema admits it as an opaque object, so
    //    only this fence can refuse a Guest-targeting request that omits
    //    it.
    // 2. A carried binding must be well-formed and its boot-identity
    //    digest must be the domain-tagged SHA-256 of this kernel's
    //    `/proc/sys/kernel/random/boot_id` (the deleted validator's
    //    `d2b-kernel-boot-id-v1` tag) - the stale-boot replay guard. A
    //    binding minted on a previous boot, or a zero/unpopulated
    //    binding, refuses fail-closed with the handler's invalid-request
    //    code.
    //
    // Host-mode requests carry no binding and target no Guest, so the
    // admission is identical to the old Host profile, which never checked.
    if let Some(binding) = &request.guest_execution {
        if !binding.is_valid() {
            return Err(UNIT_INVALID_REQUEST);
        }
        let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|_| UNIT_INVALID_REQUEST)?;
        let mut digest = Sha256::new();
        digest.update(b"d2b-kernel-boot-id-v1\0");
        digest.update(boot_id.trim().as_bytes());
        let expected: [u8; 32] = digest.finalize().into();
        if binding.boot_identity_digest != expected {
            return Err(UNIT_INVALID_REQUEST);
        }
    } else if request
        .execution_ref
        .as_ref()
        .is_some_and(|execution| execution.resource_type().as_str() == "Guest")
    {
        // A Guest-targeting request without a binding is the profile
        // gate's refusal: the binding is mandatory for a Guest execution
        // reference, not optional.
        return Err(UNIT_INVALID_REQUEST);
    }
    Ok(intent.clone())
}

fn user_bus_path(uid: u32) -> PathBuf {
    PathBuf::from("/run/user").join(uid.to_string()).join("bus")
}

/// Connect to the manager selected by the trusted runner intent.
///
/// The user bus address is derived only from the trusted intent's UID.
/// Before connecting, both the runtime directory and bus socket must be
/// owned by that UID. This prevents a caller from selecting an arbitrary
/// session bus (the retired module's `manager_connection`).
async fn manager_connection(
    intent: &ResolvedRunnerIntent,
    domain: UnitDomain,
) -> Result<Connection, &'static str> {
    match domain {
        UnitDomain::System => system_connection().await,
        UnitDomain::User => {
            let runtime_dir = PathBuf::from("/run/user").join(intent.uid.to_string());
            let bus_path = user_bus_path(intent.uid);
            let runtime_metadata = tokio::fs::metadata(&runtime_dir)
                .await
                .map_err(|_| USER_MANAGER_UNAVAILABLE)?;
            let bus_metadata = tokio::fs::metadata(&bus_path)
                .await
                .map_err(|_| USER_MANAGER_UNAVAILABLE)?;
            if !runtime_metadata.is_dir()
                || runtime_metadata.uid() != intent.uid
                || !bus_metadata.file_type().is_socket()
                || bus_metadata.uid() != intent.uid
            {
                return Err(USER_MANAGER_UNAVAILABLE);
            }
            let address_text = format!("unix:path={}", bus_path.display());
            let address =
                Address::try_from(address_text.as_str()).map_err(|_| USER_MANAGER_UNAVAILABLE)?;
            connection::Builder::address(address)
                .map_err(|_| USER_MANAGER_UNAVAILABLE)?
                .method_timeout(METHOD_TIMEOUT)
                .build()
                .await
                .map_err(|_| USER_MANAGER_UNAVAILABLE)
        }
    }
}

fn role_matches(
    role: d2b_contracts_broker::broker_wire::RunnerRole,
    process_role: &d2b_core::processes::ProcessRole,
) -> bool {
    use d2b_contracts_broker::broker_wire::RunnerRole;
    use d2b_core::processes::ProcessRole;
    matches!(
        (role, process_role),
        (
            RunnerRole::CloudHypervisor,
            ProcessRole::CloudHypervisorRunner
        ) | (RunnerRole::QemuMedia, ProcessRole::QemuMediaRunner)
            | (RunnerRole::Virtiofsd, ProcessRole::Virtiofsd)
            | (RunnerRole::Swtpm, ProcessRole::Swtpm)
            | (RunnerRole::SwtpmFlush, ProcessRole::SwtpmPreStartFlush)
            | (
                RunnerRole::Gpu,
                ProcessRole::Gpu | ProcessRole::GpuRenderNode
            )
            | (RunnerRole::Audio, ProcessRole::Audio)
            | (RunnerRole::Video, ProcessRole::Video)
            | (RunnerRole::VsockRelay, ProcessRole::VsockRelay)
            | (RunnerRole::Usbip, ProcessRole::Usbip)
            | (RunnerRole::OtelHostBridge, ProcessRole::OtelHostBridge)
            | (RunnerRole::WaylandProxy, ProcessRole::WaylandProxy)
    )
}

fn unit_name(request: &d2b_contracts_broker::broker_wire::UnitRequest) -> String {
    let mut digest = Sha256::new();
    digest.update(b"d2b-systemd-transient-unit-v1");
    digest.update(request.vm_id.as_str().as_bytes());
    digest.update([0]);
    digest.update(request.role_id.as_str().as_bytes());
    digest.update([0]);
    if let Some(resource_ref) = &request.resource_ref {
        digest.update(resource_ref.to_canonical_string().as_bytes());
    }
    digest.update([0]);
    if let Some(resource_uid) = &request.resource_uid {
        digest.update(resource_uid.as_str().as_bytes());
    }
    digest.update([0]);
    digest.update(request.bundle_runner_intent_ref.as_str().as_bytes());
    digest.update(request.provider_identity);
    digest.update(request.template_identity);
    digest.update(request.generation.to_le_bytes());
    let digest: [u8; 32] = digest.finalize().into();
    let mut suffix = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        suffix.push_str(&format!("{byte:02x}"));
    }
    format!("d2b-process-{suffix}.service")
}

async fn system_connection() -> Result<Connection, &'static str> {
    connection::Builder::system()
        .map_err(|_| UNIT_QUERY_FAILED)?
        .method_timeout(METHOD_TIMEOUT)
        .build()
        .await
        .map_err(|_| UNIT_QUERY_FAILED)
}

async fn manager_proxy(connection: &Connection) -> Result<Proxy<'_>, &'static str> {
    Proxy::new(
        connection,
        MANAGER_DESTINATION,
        MANAGER_PATH,
        MANAGER_INTERFACE,
    )
    .await
    .map_err(|_| UNIT_QUERY_FAILED)
}

fn is_no_such_unit(error: &zbus::Error) -> bool {
    matches!(
        error,
        zbus::Error::MethodError(name, _, _)
            if name.as_str() == "org.freedesktop.systemd1.NoSuchUnit"
    )
}

async fn unit_proxy<'a>(manager: &Proxy<'a>, name: &str) -> Result<OwnedObjectPath, &'static str> {
    manager.call("GetUnit", &(name)).await.map_err(|error| {
        if is_no_such_unit(&error) {
            UNIT_BUNDLE_INTENT
        } else {
            UNIT_QUERY_FAILED
        }
    })
}

/// Reads one unit-identity property through the proxy the selection seam
/// names for it (issue #587: Service-owned properties must be addressed
/// through `org.freedesktop.systemd1.Service`).
async fn unit_identity_property(
    connection: &Connection,
    unit_path: &str,
    property: &'static str,
) -> Result<zbus::zvariant::OwnedValue, &'static str> {
    let proxy = Proxy::new(
        connection,
        MANAGER_DESTINATION,
        unit_path,
        identity_property_interface(property),
    )
    .await
    .map_err(|_| UNIT_QUERY_FAILED)?;
    proxy
        .get_property::<zbus::zvariant::OwnedValue>(property)
        .await
        .map_err(|_| UNIT_QUERY_FAILED)
}

fn cgroup_identity(
    control_group: &str,
    name: &str,
    domain: UnitDomain,
    uid: u32,
) -> Result<[u8; 32], &'static str> {
    if control_group.is_empty()
        || !control_group.starts_with('/')
        || Path::new(control_group)
            .file_name()
            .and_then(|value| value.to_str())
            != Some(name)
    {
        return Err(UNIT_IDENTITY_MISMATCH);
    }
    let components = control_group
        .split('/')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    let placement_valid = match domain {
        UnitDomain::System => components.first() == Some(&"d2b.slice"),
        UnitDomain::User => {
            let user_manager = format!("user@{uid}.service");
            components
                .iter()
                .position(|component| *component == user_manager)
                .and_then(|index| components.get(index + 1))
                == Some(&"app.slice")
        }
    };
    if !placement_valid {
        return Err(UNIT_IDENTITY_MISMATCH);
    }
    let mut digest = Sha256::new();
    digest.update(b"d2b-systemd-cgroup-v1");
    digest.update(control_group.as_bytes());
    Ok(digest.finalize().into())
}

/// The process start-time ticks of one pid, read from `/proc/<pid>/stat`.
///
/// Field 22 (`starttime`) is the value the retired broker arms' pidfd race
/// fence compared; the read is async (`tokio::fs`) so it never blocks a
/// daemon worker.
async fn read_proc_stat_start_time(pid: i32) -> Result<u64, &'static str> {
    let content = tokio::fs::read_to_string(format!("/proc/{pid}/stat"))
        .await
        .map_err(|error| {
            tracing::debug!(
                family = "process-systemd",
                pid = pid,
                read_error = ?error,
                "proc stat read failed during unit identity observation"
            );
            UNIT_PIDFD_FAILED
        })?;
    // The comm field may contain spaces and parentheses; the start-time
    // field follows the last `)`.
    let Some(closing) = content.rfind(')') else {
        return Err(UNIT_PIDFD_FAILED);
    };
    let fields = content[closing + 1..].split_whitespace().collect::<Vec<_>>();
    // Field 22 of the full stat line is index 19 of the suffix after comm.
    fields
        .get(19)
        .and_then(|field| field.parse::<u64>().ok())
        .ok_or(UNIT_PIDFD_FAILED)
}

async fn read_identity(
    request: &d2b_contracts_broker::broker_wire::UnitRequest,
    intent: &ResolvedRunnerIntent,
    connection: &Connection,
    name: &str,
) -> Result<Option<UnitIdentity>, &'static str> {
    let manager = manager_proxy(connection).await?;
    let unit_path = match unit_proxy(&manager, name).await {
        Ok(path) => path,
        Err(UNIT_BUNDLE_INTENT) => return Ok(None),
        Err(error) => return Err(error),
    };
    let active_state: String = unit_identity_property(connection, unit_path.as_str(), "ActiveState")
        .await?
        .try_into()
        .map_err(|_| UNIT_QUERY_FAILED)?;
    if !matches!(active_state.as_str(), "active" | "activating" | "reloading") {
        return Ok(None);
    }
    let invocation: Vec<u8> = unit_identity_property(connection, unit_path.as_str(), "InvocationID")
        .await?
        .try_into()
        .map_err(|_| UNIT_IDENTITY_MISMATCH)?;
    let invocation_id: [u8; 16] = invocation.try_into().map_err(|_| UNIT_IDENTITY_MISMATCH)?;
    let control_group: String = unit_identity_property(connection, unit_path.as_str(), "ControlGroup")
        .await?
        .try_into()
        .map_err(|_| UNIT_QUERY_FAILED)?;
    let cgroup_identity = cgroup_identity(&control_group, name, request.domain, intent.uid)?;
    let main_pid: u32 = unit_identity_property(connection, unit_path.as_str(), "MainPID")
        .await?
        .try_into()
        .map_err(|_| UNIT_IDENTITY_MISMATCH)?;
    let main_pid = NonZeroU32::new(main_pid).ok_or(UNIT_IDENTITY_MISMATCH)?;
    let start_time_ticks = read_proc_stat_start_time(main_pid.get() as i32).await?;
    Ok(Some(UnitIdentity {
        invocation_id,
        cgroup_identity,
        main_pid: main_pid.get(),
        start_time_ticks,
        provider_identity: request.provider_identity,
        template_identity: request.template_identity,
        generation: request.generation,
        bundle_content_identity: request.bundle_content_identity.clone(),
        guest_execution: request.guest_execution.clone(),
    }))
}

async fn wait_identity(
    request: &d2b_contracts_broker::broker_wire::UnitRequest,
    intent: &ResolvedRunnerIntent,
    connection: &Connection,
    name: &str,
) -> Result<UnitIdentity, &'static str> {
    let deadline = Instant::now() + IDENTITY_READY_TIMEOUT;
    loop {
        if let Some(identity) = read_identity(request, intent, connection, name).await? {
            return Ok(identity);
        }
        if Instant::now() >= deadline {
            return Err(UNIT_IDENTITY_TIMEOUT);
        }
        tokio::time::sleep(IDENTITY_RETRY_INTERVAL).await;
    }
}

fn expected_matches(actual: &UnitIdentity, expected: &UnitIdentity) -> Result<(), &'static str> {
    if actual == expected {
        Ok(())
    } else {
        Err(UNIT_IDENTITY_MISMATCH)
    }
}

// ---------------------------------------------------------------------------
// The nested kernel leg (ported from the process family's seam, U10/KTD6)
// ---------------------------------------------------------------------------

/// Invoke the broker-generic `open-pidfd` kernel as the nested core of the
/// two pidfd-minting unit operations.
///
/// The kernel call re-presents the evidence chain the forwarded invocation
/// runs on - the root invocation id plus the ordered identities - with the
/// handler's own caller identity appended, so the graft rule authorizes the
/// kernel call against the chain's initiating principal and the in-broker
/// leg records the correlation leg keyed by the same root invocation id
/// (KTD6). The kernel's reply carries the envelope response plus the pidfd
/// it minted, in frame order.
async fn invoke_open_pidfd_kernel(
    ctx: &OperationCtx<'_>,
    pid: i32,
    expected_start_time_ticks: u64,
) -> Result<OwnedFd, OperationFailure> {
    let kernel = ctx
        .kernel
        .ok_or_else(|| OperationFailure::new(KERNEL_SEAM_UNWIRED))?;
    let socket_path = kernel.socket_path.clone();
    let caller_role = kernel.caller_role.clone();
    let zone = ctx.zone.as_str().to_owned();
    let invocation_id = ctx.invocation_id.to_owned();
    let mut chain_identities = ctx.chain_identities.to_vec();
    chain_identities.push(ctx.caller.to_canonical_string());
    let payload = serde_json::json!({
        "pid": pid,
        "expectedStartTimeTicks": expected_start_time_ticks,
    });
    let reply = kernel_seat::run(move || {
        envelope_invoke_kernel(
            &socket_path,
            KERNEL_IO_TIMEOUT,
            caller_role,
            KernelInvocation {
                operation: "open-pidfd",
                zone: &zone,
                payload,
                fds: &[],
                chain_root_invocation_id: Some(&invocation_id),
                chain_identities: Some(&chain_identities),
            },
        )
    })
    .await
    .map_err(|refusal| match refusal {
        kernel_seat::KernelRefusal::Busy => OperationFailure::with_detail(
            UNIT_PIDFD_FAILED,
            "open-pidfd: kernel invocation worker busy".to_string(),
        ),
        kernel_seat::KernelRefusal::Unavailable => OperationFailure::with_detail(
            UNIT_PIDFD_FAILED,
            "open-pidfd: kernel invocation worker unavailable".to_string(),
        ),
    })?
    .map_err(|error| {
        // The kernel's own closed code is preserved: "errored" for a kernel
        // error, "handler-refused" for a kernel refusal, so the caller's
        // classification of the leg keeps working exactly as it did for the
        // retired arms.
        let (code, detail) = match error {
            KernelInvokeError::Refused { code, detail } => (code, detail),
            other => ("errored".to_owned(), Some(other.to_string())),
        };
        OperationFailure::with_detail(
            UNIT_PIDFD_FAILED,
            format!(
                "open-pidfd: {code}{}",
                detail
                    .map(|detail| format!(" ({detail})"))
                    .unwrap_or_default()
            ),
        )
    })?;
    let mut fds = reply.fds.into_iter();
    fds.next().ok_or_else(|| {
        OperationFailure::with_detail(
            UNIT_PIDFD_FAILED,
            format!("{}: open-pidfd reply carried no pidfd", reply.response.operation),
        )
    })
}

// ---------------------------------------------------------------------------
// Typed wire (de)serialization helpers
// ---------------------------------------------------------------------------

/// Deserialize one typed request from the forwarded payload.
fn typed_request<T: serde::de::DeserializeOwned>(
    operation: &str,
    payload: &ValidatedPayload,
) -> Result<T, OperationFailure> {
    let value = serde_json::to_value(payload.object()).map_err(|error| {
        OperationFailure::with_detail(
            UNIT_INVALID_REQUEST,
            format!("{operation}: payload conversion failed: {error}"),
        )
    })?;
    serde_json::from_value(value).map_err(|error| {
        OperationFailure::with_detail(
            UNIT_INVALID_REQUEST,
            format!("{operation}: payload is not the typed {} request: {error}", operation),
        )
    })
}

/// Serialize one typed response into the canonical result payload.
fn typed_result<T: Serialize>(
    operation: &str,
    response: &T,
) -> Result<CanonicalJsonObject, OperationFailure> {
    let value =
        serde_json::to_value(response).map_err(|error| {
            OperationFailure::with_detail(
                UNIT_QUERY_FAILED,
                format!("{operation}: response conversion failed: {error}"),
            )
        })?;
    let bytes = canonical_json_bytes(&value).map_err(|error| {
        OperationFailure::with_detail(
            UNIT_QUERY_FAILED,
            format!("{operation}: response is not canonical JSON: {error}"),
        )
    })?;
    CanonicalJsonObject::parse(&bytes).map_err(|error| {
        OperationFailure::with_detail(
            UNIT_QUERY_FAILED,
            format!("{operation}: response is not a canonical object: {error}"),
        )
    })
}

/// Refuse a shared helper with a closed unit code.
fn refused(code: &'static str) -> OperationFailure {
    OperationFailure::new(code)
}

// ---------------------------------------------------------------------------
// The StartSystemdUnit handler
// ---------------------------------------------------------------------------

/// The handler of [`START_SYSTEMD_UNIT`].
struct StartSystemdUnitHandler;

#[async_trait::async_trait]
impl OperationHandler for StartSystemdUnitHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: StartTransientUnitRequest = typed_request(START_SYSTEMD_UNIT, &payload)?;
        let bundle = ctx
            .kernel
            .ok_or_else(|| refused(KERNEL_SEAM_UNWIRED))?
            .bundle
            .as_ref();
        let intent = validate_request(bundle, &request).map_err(refused)?;
        let name = unit_name(&request);
        let connection = manager_connection(&intent, request.domain)
            .await
            .map_err(refused)?;
        let manager = manager_proxy(&connection).await.map_err(refused)?;
        let exec_start = vec![(
            intent.binary_path.to_string_lossy().into_owned(),
            intent.argv.clone(),
            false,
        )];
        let mut properties = vec![
            ("Type", Value::from("exec")),
            ("ExecStart", Value::from(exec_start)),
            ("Environment", Value::from(intent.env.clone())),
            (
                "Slice",
                Value::from(match request.domain {
                    UnitDomain::System => "d2b.slice",
                    UnitDomain::User => "app.slice",
                }),
            ),
            ("KillMode", Value::from("control-group")),
            ("CollectMode", Value::from("inactive-or-failed")),
            ("NoNewPrivileges", Value::from(true)),
            ("ProtectSystem", Value::from("strict")),
        ];
        if let Some(plan) = &request.sandbox_plan {
            if let Some(umask) = plan.umask.as_deref() {
                properties.push(("UMask", Value::from(umask)));
            }
            properties.push(("OOMScoreAdjust", Value::from(plan.oom_score_adj)));
        }
        if request.domain == UnitDomain::System {
            properties.push(("User", Value::from(intent.uid.to_string())));
            properties.push(("Group", Value::from(intent.gid.to_string())));
        }
        let auxiliary: Vec<(&str, Vec<(&str, Value<'_>)>)> = Vec::new();
        let _: OwnedObjectPath = manager
            .call(
                "StartTransientUnit",
                &(name.as_str(), "replace", properties, auxiliary),
            )
            .await
            .map_err(|_| refused(UNIT_START_FAILED))?;
        let identity = wait_identity(&request, &intent, &connection, &name)
            .await
            .map_err(refused)?;
        let pidfd = invoke_open_pidfd_kernel(&ctx, identity.main_pid as i32, identity.start_time_ticks)
            .await?;
        let result = typed_result(
            START_SYSTEMD_UNIT,
            &d2b_contracts_broker::broker_wire::StartTransientUnitResponse {
                vm_id: request.vm_id,
                role_id: request.role_id,
                identity,
                pidfd_index: 0,
            },
        )?;
        Ok(OperationResult::with_fds(result, vec![pidfd]))
    }
}

// ---------------------------------------------------------------------------
// The CheckSystemdUserManager handler
// ---------------------------------------------------------------------------

/// The handler of [`CHECK_SYSTEMD_USER_MANAGER`].
struct CheckSystemdUserManagerHandler;

#[async_trait::async_trait]
impl OperationHandler for CheckSystemdUserManagerHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: d2b_contracts_broker::broker_wire::CheckUserManagerRequest =
            typed_request(CHECK_SYSTEMD_USER_MANAGER, &payload)?;
        if request.domain != UnitDomain::User {
            return Err(refused(UNIT_INVALID_REQUEST));
        }
        let bundle = ctx
            .kernel
            .ok_or_else(|| refused(KERNEL_SEAM_UNWIRED))?
            .bundle
            .as_ref();
        let intent = validate_request(bundle, &request).map_err(refused)?;
        let connection = manager_connection(&intent, request.domain)
            .await
            .map_err(refused)?;
        let manager = manager_proxy(&connection).await.map_err(refused)?;
        let result: Result<OwnedObjectPath, zbus::Error> =
            manager.call("GetUnit", &(unit_name(&request))).await;
        let available = match result {
            Ok(_) => true,
            Err(error) if is_no_such_unit(&error) => true,
            Err(_) => return Err(refused(USER_MANAGER_UNAVAILABLE)),
        };
        let result = typed_result(
            CHECK_SYSTEMD_USER_MANAGER,
            &d2b_contracts_broker::broker_wire::CheckUserManagerResponse {
                vm_id: request.vm_id,
                role_id: request.role_id,
                available,
            },
        )?;
        Ok(OperationResult::new(result))
    }
}

// ---------------------------------------------------------------------------
// The ObserveSystemdUnit handler
// ---------------------------------------------------------------------------

/// The handler of [`OBSERVE_SYSTEMD_UNIT`].
struct ObserveSystemdUnitHandler;

#[async_trait::async_trait]
impl OperationHandler for ObserveSystemdUnitHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: d2b_contracts_broker::broker_wire::ObserveUnitRequest =
            typed_request(OBSERVE_SYSTEMD_UNIT, &payload)?;
        let bundle = ctx
            .kernel
            .ok_or_else(|| refused(KERNEL_SEAM_UNWIRED))?
            .bundle
            .as_ref();
        let intent = validate_request(bundle, &request).map_err(refused)?;
        let connection = manager_connection(&intent, request.domain)
            .await
            .map_err(refused)?;
        let identity = read_identity(&request, &intent, &connection, &unit_name(&request))
            .await
            .map_err(refused)?;
        let result = typed_result(
            OBSERVE_SYSTEMD_UNIT,
            &d2b_contracts_broker::broker_wire::ObserveUnitResponse {
                vm_id: request.vm_id,
                role_id: request.role_id,
                present: identity.is_some(),
                identity,
            },
        )?;
        Ok(OperationResult::new(result))
    }
}

// ---------------------------------------------------------------------------
// The OpenSystemdUnitPidfd handler
// ---------------------------------------------------------------------------

/// The handler of [`OPEN_SYSTEMD_UNIT_PIDFD`].
struct OpenSystemdUnitPidfdHandler;

#[async_trait::async_trait]
impl OperationHandler for OpenSystemdUnitPidfdHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: OpenUnitPidfdRequest = typed_request(OPEN_SYSTEMD_UNIT_PIDFD, &payload)?;
        let bundle = ctx
            .kernel
            .ok_or_else(|| refused(KERNEL_SEAM_UNWIRED))?
            .bundle
            .as_ref();
        let intent = validate_request(bundle, &request.unit).map_err(refused)?;
        let connection = manager_connection(&intent, request.unit.domain)
            .await
            .map_err(refused)?;
        let actual = wait_identity(&request.unit, &intent, &connection, &unit_name(&request.unit))
            .await
            .map_err(refused)?;
        expected_matches(&actual, &request.expected).map_err(refused)?;
        let pidfd =
            invoke_open_pidfd_kernel(&ctx, actual.main_pid as i32, actual.start_time_ticks).await?;
        let result = typed_result(
            OPEN_SYSTEMD_UNIT_PIDFD,
            &d2b_contracts_broker::broker_wire::OpenUnitPidfdResponse {
                vm_id: request.unit.vm_id,
                role_id: request.unit.role_id,
                identity: actual,
                pidfd_index: 0,
            },
        )?;
        Ok(OperationResult::with_fds(result, vec![pidfd]))
    }
}

// ---------------------------------------------------------------------------
// The StopSystemdUnit handler
// ---------------------------------------------------------------------------

/// The handler of [`STOP_SYSTEMD_UNIT`].
struct StopSystemdUnitHandler;

#[async_trait::async_trait]
impl OperationHandler for StopSystemdUnitHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: StopUnitRequest = typed_request(STOP_SYSTEMD_UNIT, &payload)?;
        let bundle = ctx
            .kernel
            .ok_or_else(|| refused(KERNEL_SEAM_UNWIRED))?
            .bundle
            .as_ref();
        let intent = validate_request(bundle, &request.unit).map_err(refused)?;
        let name = unit_name(&request.unit);
        let connection = manager_connection(&intent, request.unit.domain)
            .await
            .map_err(refused)?;
        let manager = manager_proxy(&connection).await.map_err(refused)?;
        let Some(actual) = read_identity(&request.unit, &intent, &connection, &name)
            .await
            .map_err(refused)?
        else {
            let result = typed_result(
                STOP_SYSTEMD_UNIT,
                &d2b_contracts_broker::broker_wire::StopUnitResponse {
                    vm_id: request.unit.vm_id,
                    role_id: request.unit.role_id,
                    stopped: true,
                },
            )?;
            return Ok(OperationResult::new(result));
        };
        expected_matches(&actual, &request.expected).map_err(refused)?;
        if request.class == UnitStopClass::Terminate {
            manager
                .call_method("KillUnit", &(name.as_str(), "all", 9i32))
                .await
                .map_err(|_| refused(UNIT_STOP_FAILED))?;
        }
        manager
            .call_method("StopUnit", &(name.as_str(), "replace"))
            .await
            .map_err(|_| refused(UNIT_STOP_FAILED))?;
        let deadline = Instant::now() + IDENTITY_READY_TIMEOUT;
        loop {
            match read_identity(&request.unit, &intent, &connection, &name).await {
                Ok(None) => break,
                Ok(Some(_)) if Instant::now() >= deadline => return Err(refused(UNIT_IDENTITY_TIMEOUT)),
                Ok(Some(_)) => tokio::time::sleep(IDENTITY_RETRY_INTERVAL).await,
                Err(UNIT_BUNDLE_INTENT) => break,
                Err(error) => return Err(refused(error)),
            }
        }
        let result = typed_result(
            STOP_SYSTEMD_UNIT,
            &d2b_contracts_broker::broker_wire::StopUnitResponse {
                vm_id: request.unit.vm_id,
                role_id: request.unit.role_id,
                stopped: true,
            },
        )?;
        Ok(OperationResult::new(result))
    }
}

// ---------------------------------------------------------------------------
// Shared test fixtures (unit tests in this crate, including the hosted-
// service tests in effects_service.rs)
// ---------------------------------------------------------------------------

#[cfg(test)]
use d2b_contracts_broker::broker_wire::UnitRequest;

/// The trusted bundle whose runner targets a Host execution reference:
/// every unit-request fixture's intent resolves here, with the bundle
/// hash the request's content identity matches.
#[cfg(test)]
pub(crate) fn fixture_resolver() -> BundleResolver {
    fixture_resolver_with_execution("Host/vm", 1000)
}

/// The trusted bundle whose runner targets a Guest execution reference:
/// the request-facing variant the guest-binding gate's profile half
/// exercises.
#[cfg(test)]
pub(crate) fn fixture_guest_resolver() -> BundleResolver {
    fixture_resolver_with_execution("Guest/vm", 1000)
}

#[cfg(test)]
fn fixture_resolver_with_execution(execution_ref: &str, uid: u32) -> BundleResolver {
    use d2b_core::bundle::{Bundle, BundleGeneration};
    use d2b_core::manifest_v04::ManifestV04;
    use d2b_core::processes::{
        NodeId, ProcessExecutionDomain, ProcessNode, ProcessRole, ProcessesJson, RoleProfile,
        VmProcessDag, VmProcessInvariants,
    };
    use d2b_core::sandbox_profile::{CgroupPlacement, MountPolicy, NamespaceSet};
    use std::collections::BTreeMap;

    let host = serde_json::from_value(serde_json::json!({
        "schemaVersion": "v2",
        "site": { "allowUnsafeEastWest": false },
        "environments": [],
        "nftables": {
            "family": "inet",
            "table": "d2b",
            "chains": [],
            "tableHashAfterApply": null,
            "ownershipId": "test"
        },
        "networkManager": {
            "filePath": "/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf",
            "matchCriteria": [],
            "reloadBehavior": "atomic-reload",
            "ownership": {
                "owner": "root",
                "group": "root",
                "mode": "0644",
                "driftPolicy": "replace"
            }
        },
        "hostsFile": {
            "startMarker": "# d2b-managed begin",
            "endMarker": "# d2b-managed end",
            "rule": "replace-managed-block"
        },
        "kernelModules": [],
        "fdOwnership": [],
        "cloudHypervisorCapabilities": [],
        "ifNameMappings": [],
        "qemuMedia": null,
        "ch": null,
        "firewallCoexistencePolicy": null
    }))
    .expect("host fixture parses");
    let manifest = ManifestV04::from_slice(
        serde_json::to_vec(&serde_json::json!({
            "_manifest": { "manifestVersion": 6 },
            "_observability": {
                "enabled": false,
                "signozUrl": "http://127.0.0.1:8080",
                "signozOtlpGrpcPort": 4317,
                "signozOtlpHttpPort": 4318,
                "obsVsockCid": 0,
                "obsVsockHostSocket": "",
                "vmName": ""
            }
        }))
        .expect("manifest json serializes")
        .as_slice(),
    )
    .expect("manifest fixture parses");
    // The unit tests never reach the manager leg (the gate tests refuse
    // before it), so the fixed runner uid is fine for the execution
    // fixture wrappers; the live-uid twin substitutes the test process's
    // own uid so the manager leg drives the test user's own bus.
    BundleResolver::from_artifacts_with_zone_resource_bundles(
        Bundle {
            bundle_version: 1,
            schema_version: "v3".to_owned(),
            privileges_path: "privileges.json".to_owned(),
            storage_path: None,
            realm_workloads_launcher_v2_path: None,
            generation: BundleGeneration {
                generator: "test".to_owned(),
                source_revision: None,
                generated_at: None,
            },
            bundle_hash: Some("sha256:bundle".to_owned()),
            artifact_hashes: None,
        },
        host,
        ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: vec![VmProcessDag {
                workload_identity: None,
                vm: "vm".to_owned(),
                nodes: vec![ProcessNode {
                    id: NodeId("role".to_owned()),
                    execution_ref: Some(execution_ref.to_owned()),
                    execution_domain: Some(ProcessExecutionDomain::User),
                    user_ref: Some(format!("User/user-{uid}")),
                    role: ProcessRole::Audio,
                    unit: None,
                    binary_path: Some("/run/current-system/sw/bin/sleep".to_owned()),
                    argv: vec!["sleep".to_owned(), "60".to_owned()],
                    env: Vec::new(),
                    plan_ops: Vec::new(),
                    network_interfaces: Vec::new(),
                    profile: RoleProfile {
                        profile_id: "profile-role".to_owned(),
                        uid,
                        gid: 100_u32,
                        adr_carve_out: None,
                        caps: Vec::new(),
                        namespaces: NamespaceSet {
                            mount: false,
                            pid: false,
                            net: false,
                            ipc: false,
                            uts: false,
                            user: false,
                        },
                        seccomp_policy_ref: None,
                        mount_policy: MountPolicy {
                            read_only_paths: Vec::new(),
                            writable_paths: Vec::new(),
                            nix_store_read_only: true,
                            hide_device_nodes_by_default: true,
                            device_binds: Vec::new(),
                            bind_mounts: Vec::new(),
                        },
                        cgroup_placement: CgroupPlacement {
                            subtree: "d2b.slice/vm/role".to_owned(),
                            controllers: Vec::new(),
                            delegated: false,
                        },
                        user_namespace: None,
                        umask: None,
                    },
                    readiness: Vec::new(),
                }],
                edges: Vec::new(),
                invariants: VmProcessInvariants {
                    swtpm_pre_start_flush: false,
                    per_vm_audit_pipeline: false,
                    usbip_gating: true,
                    tpm_ownership_migration_without_running_vm_mutation: true,
                },
            }],
        },
        manifest,
        BTreeMap::new(),
    )
}

/// The typed unit request that passes every bundle check in
/// [`validate_request`]: every field matches the fixture resolver's
/// trusted runner intent.
#[cfg(test)]
pub(crate) fn fixture_unit_request() -> UnitRequest {
    use d2b_contracts::types::{BundleOpId, RoleId, VmId};
    use d2b_contracts_broker::broker_wire::RunnerRole;
    use d2b_contracts_resource::v3::ResourceRef;
    UnitRequest {
        vm_id: VmId::new("vm"),
        role_id: RoleId::new("role"),
        resource_ref: None,
        resource_uid: None,
        role: RunnerRole::Audio,
        bundle_runner_intent_ref: BundleOpId::new("runner:vm:vm:role:role"),
        bundle_content_identity: "sha256:bundle".to_owned(),
        provider_identity: [1; 32],
        template_identity: [2; 32],
        generation: 3,
        domain: UnitDomain::User,
        execution_ref: Some(
            ResourceRef::parse("Host/vm").expect("the execution reference is canonical"),
        ),
        user_ref: Some(
            ResourceRef::parse("User/user-1000").expect("the user reference is canonical"),
        ),
        guest_execution: None,
        sandbox_plan: None,
        tracing_span_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::types::{BundleOpId, RoleId, VmId};
    use d2b_contracts_broker::broker_wire::{RunnerRole, UnitRequest};
    use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};

    fn request() -> UnitRequest {
        UnitRequest {
            execution_ref: None,
            user_ref: None,
            vm_id: VmId::new("vm"),
            role_id: RoleId::new("role"),
            resource_ref: None,
            resource_uid: None,
            role: RunnerRole::Audio,
            bundle_runner_intent_ref: BundleOpId::new("intent"),
            bundle_content_identity: "bundle".to_owned(),
            provider_identity: [1; 32],
            template_identity: [2; 32],
            generation: 3,
            domain: UnitDomain::System,
            guest_execution: None,
            sandbox_plan: None,
            tracing_span_id: None,
        }
    }

    #[test]
    fn unit_names_are_deterministic_and_path_safe() {
        let name = unit_name(&request());
        assert!(name.starts_with("d2b-process-"));
        assert!(name.ends_with(".service"));
        assert!(
            name.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.')
        );
        assert_eq!(name, unit_name(&request()));
    }

    #[test]
    fn generic_resource_identity_changes_unit_name() {
        let mut first = request();
        first.resource_ref = Some(ResourceRef::parse("Process/worker").unwrap());
        first.resource_uid =
            Some(ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap());

        let mut second = first.clone();
        second.resource_uid =
            Some(ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").unwrap());
        assert_ne!(unit_name(&first), unit_name(&second));

        let mut third = first.clone();
        third.resource_ref = Some(ResourceRef::parse("Process/other-worker").unwrap());
        assert_ne!(unit_name(&first), unit_name(&third));
    }

    #[test]
    fn cgroup_identity_rejects_foreign_unit_leaves() {
        assert!(
            cgroup_identity(
                "/d2b.slice/d2b-process-good.service",
                "d2b-process-good.service",
                UnitDomain::System,
                1000,
            )
            .is_ok()
        );
        assert!(matches!(
            cgroup_identity(
                "/d2b.slice/foreign.service",
                "d2b-process-good.service",
                UnitDomain::System,
                1000
            ),
            Err(UNIT_IDENTITY_MISMATCH)
        ));
    }

    #[test]
    fn cgroup_identity_binds_user_manager_and_slice() {
        assert!(
            cgroup_identity(
                "/user.slice/user-1000.slice/user@1000.service/app.slice/d2b-process-good.service",
                "d2b-process-good.service",
                UnitDomain::User,
                1000,
            )
            .is_ok()
        );
        assert!(matches!(
            cgroup_identity(
                "/user.slice/user-1001.slice/user@1001.service/app.slice/d2b-process-good.service",
                "d2b-process-good.service",
                UnitDomain::User,
                1000,
            ),
            Err(UNIT_IDENTITY_MISMATCH)
        ));
    }

    /// The Guest execution binding gate: a request that carries a binding
    /// with a zero or wrong boot-identity digest refuses with the
    /// invalid-request code, and a binding carrying this kernel's
    /// domain-tagged boot-identity digest is admitted - the gate compares
    /// against the digest of `/proc/sys/kernel/random/boot_id`, never a
    /// constant.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn a_guest_execution_binding_with_a_wrong_boot_identity_digest_is_refused() {
        use d2b_contracts_broker::broker_wire::GuestExecutionBinding;
        let bundle = fixture_resolver();
        let mut request = fixture_unit_request();
        request.guest_execution = Some(GuestExecutionBinding {
            target_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000")
                .expect("the guest uid is canonical"),
            boot_identity_digest: [0; 32],
            session_generation: 1,
            assignment_epoch: 2,
            provider_generation: 3,
            controller_generation: 4,
        });
        assert_eq!(
            validate_request(&bundle, &request),
            Err(UNIT_INVALID_REQUEST),
            "a zero boot-identity digest is no binding"
        );

        let mut wrong = request.clone();
        wrong.guest_execution.as_mut().unwrap().boot_identity_digest = [7; 32];
        assert_eq!(
            validate_request(&bundle, &wrong),
            Err(UNIT_INVALID_REQUEST),
            "a digest that is not this kernel's boot identity is refused"
        );

        // The matching digest is admitted: the commitment is this
        // kernel's domain-tagged boot identity, so the gate proves the
        // comparison rather than rejecting every binding.
        let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .expect("the kernel boot id is readable");
        let mut digest = Sha256::new();
        digest.update(b"d2b-kernel-boot-id-v1\0");
        digest.update(boot_id.trim().as_bytes());
        request.guest_execution.as_mut().unwrap().boot_identity_digest = digest.finalize().into();
        assert!(
            validate_request(&bundle, &request).is_ok(),
            "the matching kernel boot-identity digest is admitted"
        );
    }

    /// The Guest execution binding gate's profile half: a request whose
    /// execution reference targets a Guest but carries no binding is
    /// refused with the invalid-request code - the binding is mandatory
    /// for a Guest target, never optional.
    #[test]
    fn a_guest_targeting_request_without_a_binding_is_refused() {
        let bundle = fixture_guest_resolver();
        let mut request = fixture_unit_request();
        request.execution_ref =
            Some(ResourceRef::parse("Guest/vm").expect("the execution reference is canonical"));
        assert_eq!(
            validate_request(&bundle, &request),
            Err(UNIT_INVALID_REQUEST),
            "a Guest execution reference requires a Guest execution binding"
        );
    }

    #[test]
    fn the_family_registers_exactly_the_five_committed_operations() {
        let operations = process_systemd_family_operations();
        let refs: Vec<String> = operations
            .iter()
            .map(|operation| operation.operation_ref.to_canonical_string())
            .collect();
        assert_eq!(
            refs,
            vec![
                "Operation/start-systemd-unit".to_owned(),
                "Operation/check-systemd-user-manager".to_owned(),
                "Operation/observe-systemd-unit".to_owned(),
                "Operation/open-systemd-unit-pidfd".to_owned(),
                "Operation/stop-systemd-unit".to_owned(),
            ]
        );
    }

    /// Hermetic pin for the identity read's interface selection (issue
    /// #587): Service-owned properties (ControlGroup, MainPID) must be
    /// addressed through `org.freedesktop.systemd1.Service`; reading them
    /// through the Unit proxy fails every read with UnknownProperty and the
    /// identity can never bind. Unit-owned identity properties
    /// (ActiveState, InvocationID) stay on the Unit interface.
    #[test]
    fn identity_reads_select_the_service_interface_for_service_owned_properties() {
        assert_eq!(
            identity_property_interface("ControlGroup"),
            SERVICE_INTERFACE,
            "ControlGroup is defined on org.freedesktop.systemd1.Service; the Unit \
             proxy can never bind it"
        );
        assert_eq!(
            identity_property_interface("MainPID"),
            SERVICE_INTERFACE,
            "MainPID is defined on org.freedesktop.systemd1.Service; the Unit \
             proxy can never bind it"
        );
        assert_eq!(identity_property_interface("ActiveState"), UNIT_INTERFACE);
        assert_eq!(identity_property_interface("InvocationID"), UNIT_INTERFACE);
    }
}
