use crate::environment::{EnvironmentError, ManagerEnvironment};
use d2b_contracts::unsafe_local_wire::{HelperScopeKind, HelperScopeState, ScopeIdentity};
use std::fmt;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, Proxy, connection};
use zbus::zvariant::{OwnedObjectPath, Value};

const SYSTEMD_DESTINATION: &str = "org.freedesktop.systemd1";
const SYSTEMD_MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const SYSTEMD_MANAGER_INTERFACE: &str = "org.freedesktop.systemd1.Manager";
const SYSTEMD_SCOPE_INTERFACE: &str = "org.freedesktop.systemd1.Scope";
const SYSTEMD_UNIT_INTERFACE: &str = "org.freedesktop.systemd1.Unit";
const SYSTEMD_METHOD_TIMEOUT: Duration = Duration::from_secs(5);
const SCOPE_IDENTITY_READY_TIMEOUT: Duration = Duration::from_secs(2);
const SCOPE_IDENTITY_RETRY_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeError {
    UserManagerUnavailable,
    EnvironmentInvalid,
    Timeout,
    CreateFailed,
    IdentityMismatch,
    NotFound,
    QueryFailed,
    StopFailed,
}

impl From<EnvironmentError> for ScopeError {
    fn from(_: EnvironmentError) -> Self {
        Self::EnvironmentInvalid
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedScope {
    pub unit_name: String,
    pub invocation_id: String,
    pub control_group: String,
    pub kind: HelperScopeKind,
}

impl fmt::Debug for VerifiedScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifiedScope")
            .field("unit_name", &"<redacted>")
            .field("invocation_id", &"<redacted>")
            .field("control_group", &"<redacted>")
            .field("kind", &self.kind)
            .finish()
    }
}

impl VerifiedScope {
    pub fn wire_identity(&self) -> ScopeIdentity {
        ScopeIdentity {
            invocation_id: self.invocation_id.clone(),
            kind: self.kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeInspection {
    pub state: HelperScopeState,
    pub identity_matches: bool,
}

pub trait UserScopeManager: Send + Sync + 'static {
    fn manager_environment(&self) -> Result<ManagerEnvironment, ScopeError>;
    fn start_scope(
        &self,
        supervisor_pid: u32,
        kind: HelperScopeKind,
    ) -> Result<VerifiedScope, ScopeError>;
    fn inspect_scope(&self, scope: &VerifiedScope) -> Result<ScopeInspection, ScopeError>;
    fn terminate_scope(&self, scope: &VerifiedScope, signal: i32) -> Result<(), ScopeError>;
    fn stop_scope(&self, scope: &VerifiedScope) -> Result<(), ScopeError>;
}

#[derive(Debug, Default)]
pub struct SystemdUserScopeManager {
    connection: Mutex<Option<Connection>>,
}

impl SystemdUserScopeManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn with_connection<T>(
        &self,
        operation: impl FnOnce(&Connection) -> Result<T, ScopeError>,
    ) -> Result<T, ScopeError> {
        let mut cached = self
            .connection
            .lock()
            .map_err(|_| ScopeError::UserManagerUnavailable)?;
        if cached.is_none() {
            let connection = connection::Builder::session()
                .map_err(|_| ScopeError::UserManagerUnavailable)?
                .method_timeout(SYSTEMD_METHOD_TIMEOUT)
                .build()
                .map_err(map_user_manager_error)?;
            *cached = Some(connection);
        }
        operation(cached.as_ref().ok_or(ScopeError::UserManagerUnavailable)?)
    }

    fn manager_proxy(connection: &Connection) -> Result<Proxy<'_>, ScopeError> {
        Proxy::new(
            connection,
            SYSTEMD_DESTINATION,
            SYSTEMD_MANAGER_PATH,
            SYSTEMD_MANAGER_INTERFACE,
        )
        .map_err(|_| ScopeError::UserManagerUnavailable)
    }

    fn query_scope(
        connection: &Connection,
        unit_name: &str,
        kind: HelperScopeKind,
    ) -> Result<(VerifiedScope, HelperScopeState), ScopeError> {
        let manager = Self::manager_proxy(connection)?;
        let unit_path: OwnedObjectPath = manager
            .call("GetUnit", &(unit_name))
            .map_err(map_query_error)?;
        let unit = Proxy::new(
            connection,
            SYSTEMD_DESTINATION,
            unit_path.as_str(),
            SYSTEMD_UNIT_INTERFACE,
        )
        .map_err(|_| ScopeError::QueryFailed)?;
        let invocation: Vec<u8> = unit.get_property("InvocationID").map_err(map_query_error)?;
        if invocation.len() != 16 {
            return Err(ScopeError::IdentityMismatch);
        }
        let scope_unit = Proxy::new(
            connection,
            SYSTEMD_DESTINATION,
            unit_path.as_str(),
            SYSTEMD_SCOPE_INTERFACE,
        )
        .map_err(|_| ScopeError::QueryFailed)?;
        let control_group: String = scope_unit
            .get_property("ControlGroup")
            .map_err(map_query_error)?;
        if !control_group_matches_unit(&control_group, unit_name) {
            return Err(ScopeError::IdentityMismatch);
        }
        let active_state: String = unit.get_property("ActiveState").map_err(map_query_error)?;
        let state = match active_state.as_str() {
            "active" | "activating" | "reloading" => HelperScopeState::Active,
            "deactivating" => HelperScopeState::Stopping,
            "inactive" | "failed" => HelperScopeState::Exited,
            _ => HelperScopeState::Degraded,
        };
        Ok((
            VerifiedScope {
                unit_name: unit_name.to_owned(),
                invocation_id: hex(&invocation),
                control_group,
                kind,
            },
            state,
        ))
    }
}

impl UserScopeManager for SystemdUserScopeManager {
    fn manager_environment(&self) -> Result<ManagerEnvironment, ScopeError> {
        self.with_connection(|connection| {
            let manager = Self::manager_proxy(connection)?;
            let environment: Vec<String> = manager
                .get_property("Environment")
                .map_err(map_user_manager_error)?;
            ManagerEnvironment::parse(environment).map_err(Into::into)
        })
    }

    fn start_scope(
        &self,
        supervisor_pid: u32,
        kind: HelperScopeKind,
    ) -> Result<VerifiedScope, ScopeError> {
        self.with_connection(|connection| {
            let manager = Self::manager_proxy(connection)?;
            let unit_name = scope_unit_name(kind)?;
            let properties = vec![
                ("PIDs", Value::from(vec![supervisor_pid])),
                (
                    "Description",
                    Value::from("d2b unsafe-local supervised process"),
                ),
                ("Slice", Value::from("app.slice")),
                ("CollectMode", Value::from("inactive-or-failed")),
                ("KillMode", Value::from("control-group")),
            ];
            let auxiliary: Vec<(&str, Vec<(&str, Value<'_>)>)> = Vec::new();
            let _: OwnedObjectPath = manager
                .call(
                    "StartTransientUnit",
                    &(unit_name.as_str(), "fail", properties, auxiliary),
                )
                .map_err(map_create_error)?;

            await_scope_identity(
                || Self::query_scope(connection, &unit_name, kind),
                SCOPE_IDENTITY_READY_TIMEOUT,
                SCOPE_IDENTITY_RETRY_INTERVAL,
            )
        })
    }

    fn inspect_scope(&self, scope: &VerifiedScope) -> Result<ScopeInspection, ScopeError> {
        self.with_connection(|connection| {
            match Self::query_scope(connection, &scope.unit_name, scope.kind) {
                Ok((observed, state)) => Ok(ScopeInspection {
                    state,
                    identity_matches: observed.invocation_id == scope.invocation_id
                        && observed.control_group == scope.control_group
                        && observed.unit_name == scope.unit_name,
                }),
                Err(ScopeError::QueryFailed) => Ok(ScopeInspection {
                    state: HelperScopeState::Degraded,
                    identity_matches: false,
                }),
                Err(ScopeError::NotFound) => Ok(ScopeInspection {
                    state: HelperScopeState::Exited,
                    identity_matches: true,
                }),
                Err(error) => Err(error),
            }
        })
    }

    fn terminate_scope(&self, scope: &VerifiedScope, signal: i32) -> Result<(), ScopeError> {
        let inspection = self.inspect_scope(scope)?;
        if !inspection.identity_matches {
            return Err(ScopeError::IdentityMismatch);
        }
        if inspection.state == HelperScopeState::Exited {
            return Ok(());
        }
        self.with_connection(|connection| {
            let manager = Self::manager_proxy(connection)?;
            manager
                .call_method("KillUnit", &(scope.unit_name.as_str(), "all", signal))
                .map(|_| ())
                .map_err(map_stop_error)
                .or_else(|error| (error == ScopeError::NotFound).then_some(()).ok_or(error))?;
            Ok(())
        })
    }

    fn stop_scope(&self, scope: &VerifiedScope) -> Result<(), ScopeError> {
        let inspection = self.inspect_scope(scope)?;
        if !inspection.identity_matches {
            return Err(ScopeError::IdentityMismatch);
        }
        self.with_connection(|connection| {
            let manager = Self::manager_proxy(connection)?;
            let result: Result<OwnedObjectPath, zbus::Error> =
                manager.call("StopUnit", &(scope.unit_name.as_str(), "replace"));
            result
                .map(|_| ())
                .map_err(map_stop_error)
                .or_else(|error| (error == ScopeError::NotFound).then_some(()).ok_or(error))?;
            Ok(())
        })
    }
}

fn map_user_manager_error(error: zbus::Error) -> ScopeError {
    if is_timeout(&error) {
        ScopeError::Timeout
    } else {
        ScopeError::UserManagerUnavailable
    }
}

fn map_create_error(error: zbus::Error) -> ScopeError {
    if is_timeout(&error) {
        ScopeError::Timeout
    } else {
        ScopeError::CreateFailed
    }
}

fn map_query_error(error: zbus::Error) -> ScopeError {
    if is_timeout(&error) {
        ScopeError::Timeout
    } else if is_no_such_unit(&error) {
        ScopeError::NotFound
    } else {
        ScopeError::QueryFailed
    }
}

fn map_stop_error(error: zbus::Error) -> ScopeError {
    if is_timeout(&error) {
        ScopeError::Timeout
    } else if is_no_such_unit(&error) {
        ScopeError::NotFound
    } else {
        ScopeError::StopFailed
    }
}

fn is_no_such_unit(error: &zbus::Error) -> bool {
    matches!(
        error,
        zbus::Error::MethodError(name, _, _)
            if name.as_str() == "org.freedesktop.systemd1.NoSuchUnit"
    )
}

fn is_timeout(error: &zbus::Error) -> bool {
    matches!(
        error,
        zbus::Error::InputOutput(io_error)
            if io_error.kind() == std::io::ErrorKind::TimedOut
    )
}

fn await_scope_identity<F>(
    mut query: F,
    timeout: Duration,
    retry_interval: Duration,
) -> Result<VerifiedScope, ScopeError>
where
    F: FnMut() -> Result<(VerifiedScope, HelperScopeState), ScopeError>,
{
    let deadline = Instant::now() + timeout;
    loop {
        match query() {
            Ok((scope, HelperScopeState::Starting | HelperScopeState::Active)) => return Ok(scope),
            Ok(_) => return Err(ScopeError::IdentityMismatch),
            Err(ScopeError::NotFound | ScopeError::QueryFailed | ScopeError::IdentityMismatch)
                if Instant::now() < deadline =>
            {
                std::thread::sleep(retry_interval);
            }
            Err(error) => return Err(error),
        }
    }
}

fn scope_unit_name(kind: HelperScopeKind) -> Result<String, ScopeError> {
    let prefix = match kind {
        HelperScopeKind::LauncherApp => "app",
        HelperScopeKind::WaylandProxy => "proxy",
        HelperScopeKind::PersistentShell => "shell",
    };
    let mut random = [0u8; 16];
    getrandom::getrandom(&mut random).map_err(|_| ScopeError::CreateFailed)?;
    Ok(format!("d2b-unsafe-local-{prefix}-{}.scope", hex(&random)))
}

fn control_group_matches_unit(control_group: &str, unit_name: &str) -> bool {
    !control_group.is_empty()
        && control_group.starts_with('/')
        && Path::new(control_group)
            .file_name()
            .and_then(|name| name.to_str())
            == Some(unit_name)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgroup_identity_requires_exact_scope_leaf() {
        let unit = "d2b-unsafe-local-app-0123456789abcdef.scope";
        assert!(control_group_matches_unit(
            &format!("/user.slice/user-1000.slice/user@1000.service/app.slice/{unit}"),
            unit
        ));
        assert!(!control_group_matches_unit(
            "/user.slice/user-1000.slice/user@1000.service/app.slice/foreign.scope",
            unit
        ));
        assert!(!control_group_matches_unit("", unit));
    }

    #[test]
    fn verified_scope_debug_redacts_all_manager_identity() {
        let canary = "scope-identity-canary";
        let scope = VerifiedScope {
            unit_name: canary.to_owned(),
            invocation_id: canary.to_owned(),
            control_group: format!("/{canary}"),
            kind: HelperScopeKind::LauncherApp,
        };
        assert!(!format!("{scope:?}").contains(canary));
    }

    #[test]
    fn scope_identity_waits_for_transient_unit_properties() {
        let expected = VerifiedScope {
            unit_name: "d2b-unsafe-local-app-test.scope".to_owned(),
            invocation_id: "00112233445566778899aabbccddeeff".to_owned(),
            control_group:
                "/user.slice/user-1000.slice/user@1000.service/app.slice/d2b-unsafe-local-app-test.scope"
                    .to_owned(),
            kind: HelperScopeKind::LauncherApp,
        };
        let mut attempts = 0;

        let observed = await_scope_identity(
            || {
                attempts += 1;
                if attempts == 1 {
                    Err(ScopeError::QueryFailed)
                } else {
                    Ok((expected.clone(), HelperScopeState::Active))
                }
            },
            Duration::from_millis(50),
            Duration::ZERO,
        )
        .unwrap();

        assert_eq!(observed, expected);
        assert_eq!(attempts, 2);
    }

    #[test]
    fn dbus_method_timeouts_remain_typed() {
        let error = zbus::Error::InputOutput(std::sync::Arc::new(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "timeout",
        )));

        assert_eq!(map_user_manager_error(error), ScopeError::Timeout);
    }
}
