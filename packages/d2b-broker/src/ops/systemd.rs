//! Broker-owned transient service-manager unit lifecycle.
//!
//! The daemon supplies only a trusted bundle runner reference and opaque
//! identity digests. This module resolves the executable, arguments, user,
//! environment, and unit name from the broker's bundle copy, selects either
//! the system manager or an exact same-UID user manager, performs all manager
//! calls, and returns only a closed identity tuple plus an optional pidfd.

use std::num::NonZeroU32;
use std::os::fd::OwnedFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use d2b_contracts_broker::broker_wire::{
    OpenUnitPidfdRequest, SandboxLaunchPlan, StartTransientUnitRequest,
    StopUnitRequest, UnitStopClass, UnitDomain, UnitIdentity,
};
use d2b_core::bundle_resolver::{BundleResolver, ResolvedRunnerIntent};
use sha2::{Digest, Sha256};
use zbus::connection;
use zbus::zvariant::{OwnedObjectPath, Value};
use zbus::{Address, Connection, Proxy};

const MANAGER_DESTINATION: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER_INTERFACE: &str = "org.freedesktop.systemd1.Manager";
const UNIT_INTERFACE: &str = "org.freedesktop.systemd1.Unit";
const METHOD_TIMEOUT: Duration = Duration::from_secs(5);
const IDENTITY_READY_TIMEOUT: Duration = Duration::from_secs(5);
const IDENTITY_RETRY_INTERVAL: Duration = Duration::from_millis(20);

/// Closed failures from the broker-owned unit effect owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitError {
    /// The request was structurally invalid or referred to the wrong role.
    InvalidRequest(&'static str),
    /// The trusted bundle runner intent was absent or inconsistent.
    BundleIntent,
    /// The per-user manager is unavailable or failed same-UID verification.
    UserManagerUnavailable,
    /// The system manager or unit query failed.
    Query,
    /// The transient unit could not be started.
    Start,
    /// The unit identity did not match the trusted request.
    IdentityMismatch,
    /// The exact main process pidfd could not be opened.
    Pidfd,
    /// The transient unit could not be stopped and verified inactive.
    Stop,
    /// The bounded identity wait expired.
    Timeout,
}

impl std::fmt::Display for UnitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest(_) => "unit-invalid-request",
            Self::BundleIntent => "unit-bundle-intent",
            Self::UserManagerUnavailable => "user-manager-unavailable",
            Self::Query => "unit-query-failed",
            Self::Start => "unit-start-failed",
            Self::IdentityMismatch => "unit-identity-mismatch",
            Self::Pidfd => "unit-pidfd-failed",
            Self::Stop => "unit-stop-failed",
            Self::Timeout => "unit-identity-timeout",
        })
    }
}

impl std::error::Error for UnitError {}

fn validate_request(
    resolver: &BundleResolver,
    request: &d2b_contracts_broker::broker_wire::UnitRequest,
) -> Result<ResolvedRunnerIntent, UnitError> {
    if request.generation == 0
        || request.provider_identity == [0; 32]
        || request.template_identity == [0; 32]
        || request.bundle_content_identity.is_empty()
    {
        return Err(UnitError::InvalidRequest("identity"));
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
    let intent = resolver
        .find_runner_intent(request.bundle_runner_intent_ref.as_str())
        .ok_or(UnitError::BundleIntent)?;
    if resolver.bundle.bundle_hash.as_deref() != Some(request.bundle_content_identity.as_str()) {
        return Err(UnitError::IdentityMismatch);
    }
    if intent.vm_name != request.vm_id.as_str()
        || intent.role_id != request.role_id.as_str()
        || !role_matches(request.role, &intent.role)
        || intent.binary_path.as_os_str().is_empty()
        || !intent.binary_path.is_absolute()
        || intent.argv.is_empty()
    {
        return Err(UnitError::BundleIntent);
    }
    let expected_execution = d2b_contracts_resource::v3::ResourceRef::parse(&intent.execution_ref)
        .map_err(|_| UnitError::IdentityMismatch)?;
    if request
        .execution_ref
        .as_ref()
        .is_some_and(|execution| execution != &expected_execution)
    {
        return Err(UnitError::IdentityMismatch);
    }
    let expected_domain = match intent.execution_domain {
        d2b_core::processes::ProcessExecutionDomain::System => UnitDomain::System,
        d2b_core::processes::ProcessExecutionDomain::User => UnitDomain::User,
    };
    if request.domain != expected_domain {
        return Err(UnitError::IdentityMismatch);
    }
    if let Some(plan) = &request.sandbox_plan
        && !sandbox_supported(plan)
    {
        return Err(UnitError::InvalidRequest(
            "sandbox-plan-unit-unsupported",
        ));
    }
    let expected_user = intent
        .user_ref
        .as_deref()
        .map(d2b_contracts_resource::v3::ResourceRef::parse)
        .transpose()
        .map_err(|_| UnitError::IdentityMismatch)?;
    if request.user_ref != expected_user {
        return Err(UnitError::IdentityMismatch);
    }
    Ok(intent.clone())
}

fn user_bus_path(uid: u32) -> PathBuf {
    PathBuf::from("/run/user").join(uid.to_string()).join("bus")
}

/// Connect to the manager selected by the trusted runner intent.
///
/// The user bus address is derived only from the broker-resolved UID.  Before
/// connecting, both the runtime directory and bus socket must be owned by
/// that UID.  This prevents a caller from selecting an arbitrary session bus
/// while still allowing the broker to keep manager connections out of the
/// daemon and Provider processes.
async fn manager_connection(
    intent: &ResolvedRunnerIntent,
    domain: UnitDomain,
) -> Result<Connection, UnitError> {
    match domain {
        UnitDomain::System => system_connection().await,
        UnitDomain::User => {
            let runtime_dir = PathBuf::from("/run/user").join(intent.uid.to_string());
            let bus_path = user_bus_path(intent.uid);
            let runtime_metadata = tokio::fs::metadata(&runtime_dir)
                .await
                .map_err(|_| UnitError::UserManagerUnavailable)?;
            let bus_metadata = tokio::fs::metadata(&bus_path)
                .await
                .map_err(|_| UnitError::UserManagerUnavailable)?;
            if !runtime_metadata.is_dir()
                || runtime_metadata.uid() != intent.uid
                || !bus_metadata.file_type().is_socket()
                || bus_metadata.uid() != intent.uid
            {
                return Err(UnitError::UserManagerUnavailable);
            }
            let address_text = format!("unix:path={}", bus_path.display());
            let address = Address::try_from(address_text.as_str())
                .map_err(|_| UnitError::UserManagerUnavailable)?;
            connection::Builder::address(address)
                .map_err(|_| UnitError::UserManagerUnavailable)?
                .method_timeout(METHOD_TIMEOUT)
                .build()
                .await
                .map_err(|_| UnitError::UserManagerUnavailable)
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

async fn system_connection() -> Result<Connection, UnitError> {
    connection::Builder::system()
        .map_err(|_| UnitError::Query)?
        .method_timeout(METHOD_TIMEOUT)
        .build()
        .await
        .map_err(|_| UnitError::Query)
}

async fn manager_proxy(connection: &Connection) -> Result<Proxy<'_>, UnitError> {
    Proxy::new(
        connection,
        MANAGER_DESTINATION,
        MANAGER_PATH,
        MANAGER_INTERFACE,
    )
    .await
    .map_err(|_| UnitError::Query)
}

fn is_no_such_unit(error: &zbus::Error) -> bool {
    matches!(
        error,
        zbus::Error::MethodError(name, _, _)
            if name.as_str() == "org.freedesktop.systemd1.NoSuchUnit"
    )
}

/// Verify reachability of the trusted per-user service manager without
/// exposing its connection or accepting a caller-supplied bus address.
pub async fn check_user_manager(
    resolver: &BundleResolver,
    request: &d2b_contracts_broker::broker_wire::CheckUserManagerRequest,
) -> Result<bool, UnitError> {
    if request.domain != UnitDomain::User {
        return Err(UnitError::InvalidRequest("user-manager-domain"));
    }
    let intent = validate_request(resolver, request)?;
    let connection = manager_connection(&intent, request.domain).await?;
    let manager = manager_proxy(&connection).await?;
    let result: Result<OwnedObjectPath, zbus::Error> =
        manager.call("GetUnit", &(unit_name(request))).await;
    match result {
        Ok(_) => Ok(true),
        Err(error) if is_no_such_unit(&error) => Ok(true),
        Err(_) => Err(UnitError::UserManagerUnavailable),
    }
}

async fn unit_proxy<'a>(manager: &Proxy<'a>, name: &str) -> Result<OwnedObjectPath, UnitError> {
    manager.call("GetUnit", &(name)).await.map_err(|error| {
        if is_no_such_unit(&error) {
            UnitError::BundleIntent
        } else {
            UnitError::Query
        }
    })
}

fn cgroup_identity(
    control_group: &str,
    name: &str,
    domain: UnitDomain,
    uid: u32,
) -> Result<[u8; 32], UnitError> {
    if control_group.is_empty()
        || !control_group.starts_with('/')
        || Path::new(control_group)
            .file_name()
            .and_then(|value| value.to_str())
            != Some(name)
    {
        return Err(UnitError::IdentityMismatch);
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
        return Err(UnitError::IdentityMismatch);
    }
    let mut digest = Sha256::new();
    digest.update(b"d2b-systemd-cgroup-v1");
    digest.update(control_group.as_bytes());
    Ok(digest.finalize().into())
}

async fn read_identity(
    request: &d2b_contracts_broker::broker_wire::UnitRequest,
    intent: &ResolvedRunnerIntent,
    connection: &Connection,
    name: &str,
) -> Result<Option<UnitIdentity>, UnitError> {
    let manager = manager_proxy(connection).await?;
    let unit_path = match unit_proxy(&manager, name).await {
        Ok(path) => path,
        Err(UnitError::BundleIntent) => return Ok(None),
        Err(error) => return Err(error),
    };
    let unit = Proxy::new(
        connection,
        MANAGER_DESTINATION,
        unit_path.as_str(),
        UNIT_INTERFACE,
    )
    .await
    .map_err(|_| UnitError::Query)?;
    let active_state: String = unit
        .get_property("ActiveState")
        .await
        .map_err(|_| UnitError::Query)?;
    if !matches!(active_state.as_str(), "active" | "activating" | "reloading") {
        return Ok(None);
    }
    let invocation: Vec<u8> = unit
        .get_property("InvocationID")
        .await
        .map_err(|_| UnitError::Query)?;
    let invocation_id: [u8; 16] = invocation
        .try_into()
        .map_err(|_| UnitError::IdentityMismatch)?;
    let control_group: String = unit
        .get_property("ControlGroup")
        .await
        .map_err(|_| UnitError::Query)?;
    let cgroup_identity = cgroup_identity(&control_group, name, request.domain, intent.uid)?;
    let main_pid: u32 = unit
        .get_property("MainPID")
        .await
        .map_err(|_| UnitError::Query)?;
    let main_pid = NonZeroU32::new(main_pid).ok_or(UnitError::IdentityMismatch)?;
    let start_time_ticks = crate::sys::pidfd_sys::read_proc_stat_start_time(main_pid.get() as i32)
        .map_err(|_| UnitError::Pidfd)?;
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
) -> Result<UnitIdentity, UnitError> {
    let deadline = Instant::now() + IDENTITY_READY_TIMEOUT;
    loop {
        if let Some(identity) = read_identity(request, intent, connection, name).await? {
            return Ok(identity);
        }
        if Instant::now() >= deadline {
            return Err(UnitError::Timeout);
        }
        tokio::time::sleep(IDENTITY_RETRY_INTERVAL).await;
    }
}

fn expected_matches(
    actual: &UnitIdentity,
    expected: &UnitIdentity,
) -> Result<(), UnitError> {
    if actual == expected {
        Ok(())
    } else {
        Err(UnitError::IdentityMismatch)
    }
}

/// Start a trusted transient system service and open its verified main pidfd.
pub async fn start(
    resolver: &BundleResolver,
    request: &StartTransientUnitRequest,
) -> Result<(UnitIdentity, OwnedFd), UnitError> {
    let intent = validate_request(resolver, request)?;
    let name = unit_name(request);
    let connection = manager_connection(&intent, request.domain).await?;
    let manager = manager_proxy(&connection).await?;
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
        .map_err(|_| UnitError::Start)?;
    let identity = wait_identity(request, &intent, &connection, &name).await?;
    let pidfd =
        crate::live_handlers::live_open_pidfd(identity.main_pid as i32, identity.start_time_ticks)
            .map_err(|_| UnitError::Pidfd)?
            .pidfd;
    Ok((identity, pidfd))
}

/// Observe a trusted transient unit without opening a pidfd.
pub async fn observe(
    resolver: &BundleResolver,
    request: &d2b_contracts_broker::broker_wire::ObserveUnitRequest,
) -> Result<Option<UnitIdentity>, UnitError> {
    let intent = validate_request(resolver, request)?;
    let connection = manager_connection(&intent, request.domain).await?;
    read_identity(request, &intent, &connection, &unit_name(request)).await
}

/// Re-query a trusted unit, verify its identity, and open a fresh pidfd.
pub async fn reopen(
    resolver: &BundleResolver,
    request: &OpenUnitPidfdRequest,
) -> Result<(UnitIdentity, OwnedFd), UnitError> {
    let intent = validate_request(resolver, &request.unit)?;
    let connection = manager_connection(&intent, request.unit.domain).await?;
    let actual = wait_identity(
        &request.unit,
        &intent,
        &connection,
        &unit_name(&request.unit),
    )
    .await?;
    expected_matches(&actual, &request.expected)?;
    let pidfd =
        crate::live_handlers::live_open_pidfd(actual.main_pid as i32, actual.start_time_ticks)
            .map_err(|_| UnitError::Pidfd)?
            .pidfd;
    Ok((actual, pidfd))
}

/// Stop a trusted transient unit and verify that it becomes inactive.
pub async fn stop(
    resolver: &BundleResolver,
    request: &StopUnitRequest,
) -> Result<(), UnitError> {
    let intent = validate_request(resolver, &request.unit)?;
    let name = unit_name(&request.unit);
    let connection = manager_connection(&intent, request.unit.domain).await?;
    let manager = manager_proxy(&connection).await?;
    let Some(actual) = read_identity(&request.unit, &intent, &connection, &name).await? else {
        return Ok(());
    };
    expected_matches(&actual, &request.expected)?;
    if request.class == UnitStopClass::Terminate {
        manager
            .call_method("KillUnit", &(name.as_str(), "all", 9i32))
            .await
            .map_err(|_| UnitError::Stop)?;
    }

    manager
        .call_method("StopUnit", &(name.as_str(), "replace"))
        .await
        .map_err(|_| UnitError::Stop)?;
    let deadline = Instant::now() + IDENTITY_READY_TIMEOUT;
    loop {
        match read_identity(&request.unit, &intent, &connection, &name).await {
            Ok(None) => return Ok(()),
            Ok(Some(_)) if Instant::now() >= deadline => return Err(UnitError::Timeout),
            Ok(Some(_)) => tokio::time::sleep(IDENTITY_RETRY_INTERVAL).await,
            Err(UnitError::BundleIntent) => return Ok(()),
            Err(error) => return Err(error),
        }
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
            Err(UnitError::IdentityMismatch)
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
            Err(UnitError::IdentityMismatch)
        ));
    }
}
