//! Broker-owned transient systemd lifecycle.
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
use std::thread;
use std::time::{Duration, Instant};

use d2b_contracts::broker_wire::{
    OpenSystemdUnitPidfdRequest, StartTransientUnitRequest, StopSystemdUnitRequest,
    SystemdStopClass, SystemdUnitDomain, SystemdUnitIdentity,
};
use d2b_core::bundle_resolver::{BundleResolver, ResolvedRunnerIntent};
use sha2::{Digest, Sha256};
use zbus::Address;
use zbus::blocking::{Connection, Proxy, connection};
use zbus::zvariant::{OwnedObjectPath, Value};

const SYSTEMD_DESTINATION: &str = "org.freedesktop.systemd1";
const SYSTEMD_MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const SYSTEMD_MANAGER_INTERFACE: &str = "org.freedesktop.systemd1.Manager";
const SYSTEMD_UNIT_INTERFACE: &str = "org.freedesktop.systemd1.Unit";
const SYSTEMD_METHOD_TIMEOUT: Duration = Duration::from_secs(5);
const IDENTITY_READY_TIMEOUT: Duration = Duration::from_secs(5);
const IDENTITY_RETRY_INTERVAL: Duration = Duration::from_millis(20);

/// Closed failures from the broker-owned systemd effect owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemdError {
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

impl std::fmt::Display for SystemdError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest(_) => "systemd-invalid-request",
            Self::BundleIntent => "systemd-bundle-intent",
            Self::UserManagerUnavailable => "systemd-user-manager-unavailable",
            Self::Query => "systemd-query-failed",
            Self::Start => "systemd-start-failed",
            Self::IdentityMismatch => "systemd-identity-mismatch",
            Self::Pidfd => "systemd-pidfd-failed",
            Self::Stop => "systemd-stop-failed",
            Self::Timeout => "systemd-identity-timeout",
        })
    }
}

impl std::error::Error for SystemdError {}

fn validate_request(
    resolver: &BundleResolver,
    request: &d2b_contracts::broker_wire::SystemdUnitRequest,
) -> Result<ResolvedRunnerIntent, SystemdError> {
    if request.generation == 0
        || request.provider_identity == [0; 32]
        || request.template_identity == [0; 32]
    {
        return Err(SystemdError::InvalidRequest("identity"));
    }
    let intent = resolver
        .find_runner_intent(request.bundle_runner_intent_ref.as_str())
        .ok_or(SystemdError::BundleIntent)?;
    if intent.vm_name != request.vm_id.as_str()
        || intent.role_id != request.role_id.as_str()
        || !role_matches(request.role, &intent.role)
        || intent.binary_path.as_os_str().is_empty()
        || !intent.binary_path.is_absolute()
        || intent.argv.is_empty()
    {
        return Err(SystemdError::BundleIntent);
    }
    let expected_execution = d2b_contracts::v3::ResourceRef::parse(&intent.execution_ref)
        .map_err(|_| SystemdError::IdentityMismatch)?;
    if request
        .execution_ref
        .as_ref()
        .is_some_and(|execution| execution != &expected_execution)
    {
        return Err(SystemdError::IdentityMismatch);
    }
    let expected_domain = match intent.execution_domain {
        d2b_core::processes::ProcessExecutionDomain::System => SystemdUnitDomain::System,
        d2b_core::processes::ProcessExecutionDomain::User => SystemdUnitDomain::User,
    };
    if request.domain != expected_domain {
        return Err(SystemdError::IdentityMismatch);
    }
    let expected_user = intent
        .user_ref
        .as_deref()
        .map(d2b_contracts::v3::ResourceRef::parse)
        .transpose()
        .map_err(|_| SystemdError::IdentityMismatch)?;
    if request.user_ref != expected_user {
        return Err(SystemdError::IdentityMismatch);
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
fn manager_connection(
    intent: &ResolvedRunnerIntent,
    domain: SystemdUnitDomain,
) -> Result<Connection, SystemdError> {
    match domain {
        SystemdUnitDomain::System => system_connection(),
        SystemdUnitDomain::User => {
            let runtime_dir = PathBuf::from("/run/user").join(intent.uid.to_string());
            let bus_path = user_bus_path(intent.uid);
            let runtime_metadata = std::fs::metadata(&runtime_dir)
                .map_err(|_| SystemdError::UserManagerUnavailable)?;
            let bus_metadata =
                std::fs::metadata(&bus_path).map_err(|_| SystemdError::UserManagerUnavailable)?;
            if !runtime_metadata.is_dir()
                || runtime_metadata.uid() != intent.uid
                || !bus_metadata.file_type().is_socket()
                || bus_metadata.uid() != intent.uid
            {
                return Err(SystemdError::UserManagerUnavailable);
            }
            let address_text = format!("unix:path={}", bus_path.display());
            let address = Address::try_from(address_text.as_str())
                .map_err(|_| SystemdError::UserManagerUnavailable)?;
            connection::Builder::address(address)
                .map_err(|_| SystemdError::UserManagerUnavailable)?
                .method_timeout(SYSTEMD_METHOD_TIMEOUT)
                .build()
                .map_err(|_| SystemdError::UserManagerUnavailable)
        }
    }
}

fn role_matches(
    role: d2b_contracts::broker_wire::RunnerRole,
    process_role: &d2b_core::processes::ProcessRole,
) -> bool {
    use d2b_contracts::broker_wire::RunnerRole;
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

fn unit_name(request: &d2b_contracts::broker_wire::SystemdUnitRequest) -> String {
    let mut digest = Sha256::new();
    digest.update(b"d2b-systemd-transient-unit-v1");
    digest.update(request.vm_id.as_str().as_bytes());
    digest.update([0]);
    digest.update(request.role_id.as_str().as_bytes());
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

fn system_connection() -> Result<Connection, SystemdError> {
    connection::Builder::system()
        .map_err(|_| SystemdError::Query)?
        .method_timeout(SYSTEMD_METHOD_TIMEOUT)
        .build()
        .map_err(|_| SystemdError::Query)
}

fn manager_proxy(connection: &Connection) -> Result<Proxy<'_>, SystemdError> {
    Proxy::new(
        connection,
        SYSTEMD_DESTINATION,
        SYSTEMD_MANAGER_PATH,
        SYSTEMD_MANAGER_INTERFACE,
    )
    .map_err(|_| SystemdError::Query)
}

fn is_no_such_unit(error: &zbus::Error) -> bool {
    matches!(
        error,
        zbus::Error::MethodError(name, _, _)
            if name.as_str() == "org.freedesktop.systemd1.NoSuchUnit"
    )
}

/// Verify reachability of the trusted per-user systemd manager without
/// exposing its connection or accepting a caller-supplied bus address.
pub fn check_user_manager(
    resolver: &BundleResolver,
    request: &d2b_contracts::broker_wire::CheckSystemdUserManagerRequest,
) -> Result<bool, SystemdError> {
    if request.domain != SystemdUnitDomain::User {
        return Err(SystemdError::InvalidRequest("user-manager-domain"));
    }
    let intent = validate_request(resolver, request)?;
    let connection = manager_connection(&intent, request.domain)?;
    let manager = manager_proxy(&connection)?;
    let result: Result<OwnedObjectPath, zbus::Error> =
        manager.call("GetUnit", &(unit_name(request)));
    match result {
        Ok(_) => Ok(true),
        Err(error) if is_no_such_unit(&error) => Ok(true),
        Err(_) => Err(SystemdError::UserManagerUnavailable),
    }
}

fn unit_proxy<'a>(manager: &Proxy<'a>, name: &str) -> Result<OwnedObjectPath, SystemdError> {
    manager.call("GetUnit", &(name)).map_err(|error| {
        if is_no_such_unit(&error) {
            SystemdError::BundleIntent
        } else {
            SystemdError::Query
        }
    })
}

fn cgroup_identity(
    control_group: &str,
    name: &str,
    domain: SystemdUnitDomain,
    uid: u32,
) -> Result<[u8; 32], SystemdError> {
    if control_group.is_empty()
        || !control_group.starts_with('/')
        || Path::new(control_group)
            .file_name()
            .and_then(|value| value.to_str())
            != Some(name)
    {
        return Err(SystemdError::IdentityMismatch);
    }
    let components = control_group
        .split('/')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    let placement_valid = match domain {
        SystemdUnitDomain::System => components.first() == Some(&"d2b.slice"),
        SystemdUnitDomain::User => {
            let user_manager = format!("user@{uid}.service");
            components
                .iter()
                .position(|component| *component == user_manager)
                .and_then(|index| components.get(index + 1))
                == Some(&"app.slice")
        }
    };
    if !placement_valid {
        return Err(SystemdError::IdentityMismatch);
    }
    let mut digest = Sha256::new();
    digest.update(b"d2b-systemd-cgroup-v1");
    digest.update(control_group.as_bytes());
    Ok(digest.finalize().into())
}

fn read_identity(
    request: &d2b_contracts::broker_wire::SystemdUnitRequest,
    intent: &ResolvedRunnerIntent,
    connection: &Connection,
    name: &str,
) -> Result<Option<SystemdUnitIdentity>, SystemdError> {
    let manager = manager_proxy(connection)?;
    let unit_path = match unit_proxy(&manager, name) {
        Ok(path) => path,
        Err(SystemdError::BundleIntent) => return Ok(None),
        Err(error) => return Err(error),
    };
    let unit = Proxy::new(
        connection,
        SYSTEMD_DESTINATION,
        unit_path.as_str(),
        SYSTEMD_UNIT_INTERFACE,
    )
    .map_err(|_| SystemdError::Query)?;
    let active_state: String = unit
        .get_property("ActiveState")
        .map_err(|_| SystemdError::Query)?;
    if !matches!(active_state.as_str(), "active" | "activating" | "reloading") {
        return Ok(None);
    }
    let invocation: Vec<u8> = unit
        .get_property("InvocationID")
        .map_err(|_| SystemdError::Query)?;
    let invocation_id: [u8; 16] = invocation
        .try_into()
        .map_err(|_| SystemdError::IdentityMismatch)?;
    let control_group: String = unit
        .get_property("ControlGroup")
        .map_err(|_| SystemdError::Query)?;
    let cgroup_identity = cgroup_identity(&control_group, name, request.domain, intent.uid)?;
    let main_pid: u32 = unit
        .get_property("MainPID")
        .map_err(|_| SystemdError::Query)?;
    let main_pid = NonZeroU32::new(main_pid).ok_or(SystemdError::IdentityMismatch)?;
    let start_time_ticks = crate::sys::pidfd_sys::read_proc_stat_start_time(main_pid.get() as i32)
        .map_err(|_| SystemdError::Pidfd)?;
    Ok(Some(SystemdUnitIdentity {
        invocation_id,
        cgroup_identity,
        main_pid: main_pid.get(),
        start_time_ticks,
        provider_identity: request.provider_identity,
        template_identity: request.template_identity,
        generation: request.generation,
    }))
}

fn wait_identity(
    request: &d2b_contracts::broker_wire::SystemdUnitRequest,
    intent: &ResolvedRunnerIntent,
    connection: &Connection,
    name: &str,
) -> Result<SystemdUnitIdentity, SystemdError> {
    let deadline = Instant::now() + IDENTITY_READY_TIMEOUT;
    loop {
        if let Some(identity) = read_identity(request, intent, connection, name)? {
            return Ok(identity);
        }
        if Instant::now() >= deadline {
            return Err(SystemdError::Timeout);
        }
        thread::sleep(IDENTITY_RETRY_INTERVAL);
    }
}

fn expected_matches(
    actual: &SystemdUnitIdentity,
    expected: &SystemdUnitIdentity,
) -> Result<(), SystemdError> {
    if actual == expected {
        Ok(())
    } else {
        Err(SystemdError::IdentityMismatch)
    }
}

/// Start a trusted transient system service and open its verified main pidfd.
pub fn start(
    resolver: &BundleResolver,
    request: &StartTransientUnitRequest,
) -> Result<(SystemdUnitIdentity, OwnedFd), SystemdError> {
    let intent = validate_request(resolver, request)?;
    let name = unit_name(request);
    let connection = manager_connection(&intent, request.domain)?;
    let manager = manager_proxy(&connection)?;
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
                SystemdUnitDomain::System => "d2b.slice",
                SystemdUnitDomain::User => "app.slice",
            }),
        ),
        ("KillMode", Value::from("control-group")),
        ("CollectMode", Value::from("inactive-or-failed")),
        ("NoNewPrivileges", Value::from(true)),
    ];
    if request.domain == SystemdUnitDomain::System {
        properties.push(("User", Value::from(intent.uid.to_string())));
        properties.push(("Group", Value::from(intent.gid.to_string())));
    }
    let auxiliary: Vec<(&str, Vec<(&str, Value<'_>)>)> = Vec::new();
    let _: OwnedObjectPath = manager
        .call(
            "StartTransientUnit",
            &(name.as_str(), "replace", properties, auxiliary),
        )
        .map_err(|_| SystemdError::Start)?;
    let identity = wait_identity(request, &intent, &connection, &name)?;
    let pidfd =
        crate::live_handlers::live_open_pidfd(identity.main_pid as i32, identity.start_time_ticks)
            .map_err(|_| SystemdError::Pidfd)?
            .pidfd;
    Ok((identity, pidfd))
}

/// Observe a trusted transient unit without opening a pidfd.
pub fn observe(
    resolver: &BundleResolver,
    request: &d2b_contracts::broker_wire::ObserveSystemdUnitRequest,
) -> Result<Option<SystemdUnitIdentity>, SystemdError> {
    let intent = validate_request(resolver, request)?;
    let connection = manager_connection(&intent, request.domain)?;
    read_identity(request, &intent, &connection, &unit_name(request))
}

/// Re-query a trusted unit, verify its identity, and open a fresh pidfd.
pub fn reopen(
    resolver: &BundleResolver,
    request: &OpenSystemdUnitPidfdRequest,
) -> Result<(SystemdUnitIdentity, OwnedFd), SystemdError> {
    let intent = validate_request(resolver, &request.unit)?;
    let connection = manager_connection(&intent, request.unit.domain)?;
    let actual = wait_identity(
        &request.unit,
        &intent,
        &connection,
        &unit_name(&request.unit),
    )?;
    expected_matches(&actual, &request.expected)?;
    let pidfd =
        crate::live_handlers::live_open_pidfd(actual.main_pid as i32, actual.start_time_ticks)
            .map_err(|_| SystemdError::Pidfd)?
            .pidfd;
    Ok((actual, pidfd))
}

/// Stop a trusted transient unit and verify that it becomes inactive.
pub fn stop(
    resolver: &BundleResolver,
    request: &StopSystemdUnitRequest,
) -> Result<(), SystemdError> {
    let intent = validate_request(resolver, &request.unit)?;
    let name = unit_name(&request.unit);
    let connection = manager_connection(&intent, request.unit.domain)?;
    let actual = read_identity(&request.unit, &intent, &connection, &name)?
        .ok_or(SystemdError::IdentityMismatch)?;
    expected_matches(&actual, &request.expected)?;
    let manager = manager_proxy(&connection)?;
    if request.class == SystemdStopClass::Terminate {
        manager
            .call_method("KillUnit", &(name.as_str(), "all", 9i32))
            .map_err(|_| SystemdError::Stop)?;
    }
    manager
        .call_method("StopUnit", &(name.as_str(), "replace"))
        .map_err(|_| SystemdError::Stop)?;
    let deadline = Instant::now() + IDENTITY_READY_TIMEOUT;
    loop {
        match read_identity(&request.unit, &intent, &connection, &name) {
            Ok(None) => return Ok(()),
            Ok(Some(_)) if Instant::now() >= deadline => return Err(SystemdError::Timeout),
            Ok(Some(_)) => thread::sleep(IDENTITY_RETRY_INTERVAL),
            Err(SystemdError::BundleIntent) => return Ok(()),
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::broker_wire::{RunnerRole, SystemdUnitRequest};
    use d2b_contracts::types::{BundleOpId, RoleId, VmId};

    fn request() -> SystemdUnitRequest {
        SystemdUnitRequest {
            execution_ref: None,
            user_ref: None,
            vm_id: VmId::new("vm"),
            role_id: RoleId::new("role"),
            role: RunnerRole::Audio,
            bundle_runner_intent_ref: BundleOpId::new("intent"),
            provider_identity: [1; 32],
            template_identity: [2; 32],
            generation: 3,
            domain: SystemdUnitDomain::System,
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
    fn cgroup_identity_rejects_foreign_unit_leaves() {
        assert!(
            cgroup_identity(
                "/d2b.slice/d2b-process-good.service",
                "d2b-process-good.service",
                SystemdUnitDomain::System,
                1000,
            )
            .is_ok()
        );
        assert!(matches!(
            cgroup_identity(
                "/d2b.slice/foreign.service",
                "d2b-process-good.service",
                SystemdUnitDomain::System,
                1000
            ),
            Err(SystemdError::IdentityMismatch)
        ));
    }

    #[test]
    fn cgroup_identity_binds_user_manager_and_slice() {
        assert!(
            cgroup_identity(
                "/user.slice/user-1000.slice/user@1000.service/app.slice/d2b-process-good.service",
                "d2b-process-good.service",
                SystemdUnitDomain::User,
                1000,
            )
            .is_ok()
        );
        assert!(matches!(
            cgroup_identity(
                "/user.slice/user-1001.slice/user@1001.service/app.slice/d2b-process-good.service",
                "d2b-process-good.service",
                SystemdUnitDomain::User,
                1000,
            ),
            Err(SystemdError::IdentityMismatch)
        ));
    }
}
