use d2b_unsafe_local_helper::shell_runtime::{
    AuthenticatedSystemdUserRuntime, AuthenticatedTerminalAttachment, CancelOutcome,
    EstablishedShellSession, ScopeInspection, ScopeOwnership, ScopeProcessState, ShellMethod,
    ShellOwner, ShellRequest, ShellRuntimeService, ShellServiceError, ShellState, ShellStateStore,
    TERMINAL_ATTACHMENT_INDEX, VerifiedTransientScope,
};
use nix::unistd::getuid;
use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

const GENERATION: u64 = 17;
const NOW: u64 = 10_000;

struct Session {
    authenticated: bool,
    uid: u32,
    workload: &'static str,
}

impl EstablishedShellSession for Session {
    fn service_package(&self) -> &str {
        "d2b.shell.v2"
    }

    fn endpoint_purpose(&self) -> &str {
        "shell-supervisor"
    }

    fn endpoint_role(&self) -> &str {
        "shell-supervisor"
    }

    fn is_authenticated(&self) -> bool {
        self.authenticated
    }

    fn uses_pre_authorized_transport(&self) -> bool {
        true
    }

    fn authenticated_uid(&self) -> u32 {
        self.uid
    }

    fn process_uid(&self) -> u32 {
        getuid().as_raw()
    }

    fn session_generation(&self) -> u64 {
        GENERATION
    }

    fn realm_id(&self) -> &str {
        "local"
    }

    fn workload_id(&self) -> &str {
        self.workload
    }
}

#[derive(Clone)]
struct FakeRuntime {
    scopes: Arc<Mutex<BTreeMap<String, (VerifiedTransientScope, ScopeInspection)>>>,
    killed: Arc<Mutex<Vec<String>>>,
}

impl FakeRuntime {
    fn new() -> Self {
        Self {
            scopes: Arc::new(Mutex::new(BTreeMap::new())),
            killed: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn scope(resource_id: &str, owner: &ShellOwner) -> VerifiedTransientScope {
        VerifiedTransientScope::new(
            resource_id.to_owned(),
            format!("d2b-shell-{resource_id}.scope"),
            format!("invocation-{resource_id}"),
            format!("/user.slice/user-1000.slice/{resource_id}"),
            owner.uid(),
            owner.session_generation(),
        )
        .unwrap()
    }

    fn exact_running() -> ScopeInspection {
        ScopeInspection {
            ownership: ScopeOwnership::Exact,
            process_state: ScopeProcessState::Running,
        }
    }
}

impl AuthenticatedSystemdUserRuntime for FakeRuntime {
    fn create_shell_scope(
        &mut self,
        owner: &ShellOwner,
        resource_id: &str,
        _operation_id: &str,
    ) -> Result<VerifiedTransientScope, ShellServiceError> {
        let scope = Self::scope(resource_id, owner);
        self.scopes.lock().unwrap().insert(
            resource_id.to_owned(),
            (scope.clone(), Self::exact_running()),
        );
        Ok(scope)
    }

    fn inspect_shell_scope(
        &mut self,
        _owner: &ShellOwner,
        scope: &VerifiedTransientScope,
    ) -> Result<ScopeInspection, ShellServiceError> {
        self.scopes
            .lock()
            .unwrap()
            .get(scope.resource_id())
            .map(|(_, inspection)| *inspection)
            .ok_or(ShellServiceError::NotFound)
    }

    fn adopt_shell_scope(
        &mut self,
        _owner: &ShellOwner,
        scope: &VerifiedTransientScope,
        _operation_id: &str,
    ) -> Result<ScopeInspection, ShellServiceError> {
        self.scopes
            .lock()
            .unwrap()
            .get(scope.resource_id())
            .map(|(_, inspection)| *inspection)
            .ok_or(ShellServiceError::NotFound)
    }

    fn kill_shell_scope(
        &mut self,
        _owner: &ShellOwner,
        scope: &VerifiedTransientScope,
        _operation_id: &str,
    ) -> Result<ScopeInspection, ShellServiceError> {
        let mut scopes = self.scopes.lock().unwrap();
        let (expected, inspection) = scopes
            .get_mut(scope.resource_id())
            .ok_or(ShellServiceError::NotFound)?;
        if expected != scope || inspection.ownership != ScopeOwnership::Exact {
            return Err(ShellServiceError::ScopeOwnershipMismatch);
        }
        inspection.process_state = ScopeProcessState::Exited;
        self.killed
            .lock()
            .unwrap()
            .push(scope.resource_id().to_owned());
        Ok(*inspection)
    }

    fn cancel(&mut self, _owner: &ShellOwner, _request_id: [u8; 16]) -> CancelOutcome {
        CancelOutcome::UnknownRequest
    }
}

fn owner() -> ShellOwner {
    ShellOwner::admit(&Session {
        authenticated: true,
        uid: getuid().as_raw(),
        workload: "terminal",
    })
    .unwrap()
}

fn request(method: ShellMethod, resource_id: &str) -> ShellRequest {
    ShellRequest {
        method,
        request_id: [3; 16],
        idempotency_key: method.mutating().then_some([4; 32]),
        issued_at_unix_ms: NOW - 1,
        expires_at_unix_ms: NOW + 1_000,
        session_generation: GENERATION,
        realm_id: "local".into(),
        workload_id: "terminal".into(),
        resource_id: resource_id.into(),
        operation_id: if method.mutating() {
            "operation".into()
        } else {
            String::new()
        },
        stream_id: String::new(),
        attachment_indexes: Vec::new(),
        output_ring_bytes: 0,
    }
}

fn create(service: &mut ShellRuntimeService<FakeRuntime>, resource_id: &str, bytes: usize) {
    let mut create = request(ShellMethod::Create, resource_id);
    create.output_ring_bytes = bytes;
    assert_eq!(
        service.dispatch(&create, vec![], NOW).unwrap().state,
        ShellState::Running
    );
}

#[test]
fn admission_binds_exact_authenticated_requesting_uid() {
    let uid = getuid().as_raw();
    assert_eq!(
        ShellOwner::admit(&Session {
            authenticated: false,
            uid,
            workload: "terminal",
        })
        .unwrap_err(),
        ShellServiceError::Unauthenticated
    );
    assert_eq!(
        ShellOwner::admit(&Session {
            authenticated: true,
            uid: uid.saturating_add(1),
            workload: "terminal",
        })
        .unwrap_err(),
        ShellServiceError::OwnerMismatch
    );
}

#[test]
fn disconnect_detaches_but_shared_supervisor_survives_reconnect() {
    let state = ShellStateStore::default();
    let runtime = FakeRuntime::new();
    let mut first = ShellRuntimeService::new(owner(), runtime.clone(), state.clone());
    create(&mut first, "primary", 256 * 1024);

    let mut attach = request(ShellMethod::Attach, "primary");
    attach.stream_id = "terminal".into();
    attach.attachment_indexes = vec![TERMINAL_ATTACHMENT_INDEX];
    let (terminal, _peer) = UnixStream::pair().unwrap();
    let descriptor = AuthenticatedTerminalAttachment::new(
        terminal.into(),
        getuid().as_raw(),
        GENERATION,
        attach.request_id,
    );
    assert_eq!(
        first
            .dispatch(&attach, vec![descriptor], NOW)
            .unwrap()
            .state,
        ShellState::Attached
    );
    first.disconnect().unwrap();

    let mut reconnected = ShellRuntimeService::new(owner(), runtime, state);
    assert_eq!(
        reconnected
            .dispatch(&request(ShellMethod::Inspect, "primary"), vec![], NOW)
            .unwrap()
            .state,
        ShellState::Running
    );
}

#[test]
fn exact_kill_cannot_touch_unrelated_same_uid_scope() {
    let runtime = FakeRuntime::new();
    let killed = Arc::clone(&runtime.killed);
    let unrelated_owner = owner();
    runtime.scopes.lock().unwrap().insert(
        "unrelated".into(),
        (
            FakeRuntime::scope("unrelated", &unrelated_owner),
            FakeRuntime::exact_running(),
        ),
    );
    let mut service = ShellRuntimeService::new(owner(), runtime, ShellStateStore::default());
    create(&mut service, "primary", 256 * 1024);

    assert_eq!(
        service
            .dispatch(&request(ShellMethod::Kill, "primary"), vec![], NOW)
            .unwrap()
            .state,
        ShellState::Exited
    );
    assert_eq!(&*killed.lock().unwrap(), &["primary"]);
}

#[test]
fn ambiguous_adoption_is_preserved_degraded_and_never_killed() {
    let runtime = FakeRuntime::new();
    let killed = Arc::clone(&runtime.killed);
    let scope = FakeRuntime::scope("adopted", &owner());
    runtime.scopes.lock().unwrap().insert(
        "adopted".into(),
        (
            scope.clone(),
            ScopeInspection {
                ownership: ScopeOwnership::Ambiguous,
                process_state: ScopeProcessState::Running,
            },
        ),
    );
    let mut service = ShellRuntimeService::new(owner(), runtime, ShellStateStore::default());
    assert_eq!(
        service.adopt(scope, "adopt-operation", 256 * 1024).unwrap(),
        ShellState::Degraded
    );
    assert_eq!(
        service
            .dispatch(&request(ShellMethod::Kill, "adopted"), vec![], NOW)
            .unwrap_err(),
        ShellServiceError::ScopeOwnershipMismatch
    );
    assert!(killed.lock().unwrap().is_empty());
}

#[test]
fn output_budget_and_terminal_descriptor_count_fail_closed() {
    let runtime = FakeRuntime::new();
    let scopes = Arc::clone(&runtime.scopes);
    let mut service =
        ShellRuntimeService::new(owner(), runtime, ShellStateStore::new(512 * 1024).unwrap());
    create(&mut service, "first", 512 * 1024);
    let mut second = request(ShellMethod::Create, "second");
    second.output_ring_bytes = 1;
    assert_eq!(
        service.dispatch(&second, vec![], NOW).unwrap_err(),
        ShellServiceError::ReservationExhausted
    );
    assert!(!scopes.lock().unwrap().contains_key("second"));

    let mut attach = request(ShellMethod::Attach, "first");
    attach.stream_id = "terminal".into();
    attach.attachment_indexes = vec![TERMINAL_ATTACHMENT_INDEX];
    assert_eq!(
        service.dispatch(&attach, vec![], NOW).unwrap_err(),
        ShellServiceError::AttachmentMismatch
    );
}

#[test]
fn shared_store_is_bound_to_one_authenticated_workload_owner() {
    let state = ShellStateStore::default();
    let runtime = FakeRuntime::new();
    let mut primary = ShellRuntimeService::new(owner(), runtime.clone(), state.clone());
    create(&mut primary, "primary", 256 * 1024);

    let other_owner = ShellOwner::admit(&Session {
        authenticated: true,
        uid: getuid().as_raw(),
        workload: "other",
    })
    .unwrap();
    let mut other = ShellRuntimeService::new(other_owner, runtime, state);
    let mut list = request(ShellMethod::List, "");
    list.workload_id = "other".into();
    assert_eq!(
        other.dispatch(&list, vec![], NOW).unwrap_err(),
        ShellServiceError::OwnerMismatch
    );
}

#[test]
fn debug_and_output_projection_never_expose_terminal_bytes_or_ids() {
    let mut service =
        ShellRuntimeService::new(owner(), FakeRuntime::new(), ShellStateStore::default());
    create(&mut service, "private-shell", 256 * 1024);
    service
        .append_output("private-shell", b"private-terminal-canary")
        .unwrap();
    let output = service
        .read_output("private-shell", 0, 1024, false, std::time::Duration::ZERO)
        .unwrap();
    assert!(!format!("{output:?}").contains("private-terminal-canary"));
    assert!(!format!("{service:?}").contains("private-shell"));
}
