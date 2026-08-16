//! Zone-bound Provider lifecycle acceptance through the daemon boundary.
//!
//! The effect fixture is filesystem-backed rather than a call recorder.  It
//! gives the daemon a durable process state to observe, mutate, and adopt
//! after reconstruction, while the Provider registry and lifecycle admission
//! remain the production implementations under test.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

use d2b_contracts::{
    broker_wire::BrokerCallerRole,
    v3::{ResourceName, ResourceRef, identity::ZoneId},
};
use d2bd::provider_effects::{
    EffectDispatch, GuestLifecycleOperation, GuestLifecycleRequest, GuestLifecycleState,
    ProviderEffectError, ProviderLifecycleDispatch, ProviderLifecycleEffectPort,
};
use d2bd::provider_registry::{ProviderBinding, ProviderRuntime, ProviderRuntimeDispatch};

struct FilesystemLifecycle {
    root: PathBuf,
    apply_calls: AtomicUsize,
}

impl FilesystemLifecycle {
    fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            apply_calls: AtomicUsize::new(0),
        }
    }

    fn state_path(&self, request: &GuestLifecycleRequest) -> PathBuf {
        self.root
            .join(format!("{}.state", request.guest().name().as_str()))
    }

    fn write_state(&self, request: &GuestLifecycleRequest, state: &str) {
        let path = self.state_path(request);
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .expect("open filesystem-backed lifecycle state");
        file.write_all(state.as_bytes())
            .expect("write filesystem-backed lifecycle state");
        file.sync_all().expect("sync lifecycle state");
    }
}

impl ProviderLifecycleEffectPort for FilesystemLifecycle {
    type Output = GuestLifecycleState;

    fn actual_state(
        &self,
        request: &GuestLifecycleRequest,
    ) -> Result<GuestLifecycleState, ProviderEffectError> {
        match fs::read_to_string(self.state_path(request)) {
            Ok(contents) => match contents.as_str() {
                "started" => Ok(GuestLifecycleState::Started),
                "stopped" => Ok(GuestLifecycleState::Stopped),
                _ => Err(ProviderEffectError::StateUnavailable),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(GuestLifecycleState::Stopped)
            }
            Err(_) => Err(ProviderEffectError::StateUnavailable),
        }
    }

    fn apply(&self, request: &GuestLifecycleRequest) -> Result<Self::Output, ProviderEffectError> {
        self.apply_calls.fetch_add(1, Ordering::SeqCst);
        let state = match request.operation() {
            GuestLifecycleOperation::Start => ("started", GuestLifecycleState::Started),
            GuestLifecycleOperation::Stop => ("stopped", GuestLifecycleState::Stopped),
        };
        self.write_state(request, state.0);
        Ok(state.1)
    }
}

fn zone() -> ZoneId {
    ZoneId::parse("work").expect("valid Zone")
}

fn provider_binding(name: &str) -> ProviderBinding {
    ProviderBinding::new(
        zone(),
        ResourceRef::parse(&format!("Provider/{name}")).expect("valid Provider ref"),
        ResourceName::parse(name).expect("valid Provider name"),
        "sha256:0000000000000000000000000000000000000000000000000000000000000001",
    )
    .expect("valid Provider binding")
}

fn request(operation: GuestLifecycleOperation, key: &str) -> GuestLifecycleRequest {
    GuestLifecycleRequest::new(
        zone(),
        ResourceRef::parse("Guest/workstation").expect("valid Guest ref"),
        operation,
        key,
    )
    .expect("valid lifecycle request")
}

#[test]
fn activation_refusal_and_removal_cross_the_provider_boundary() {
    let directory = tempfile::tempdir().expect("temporary lifecycle state");
    let effect = FilesystemLifecycle::new(directory.path());
    let runtime = ProviderRuntime::from_bindings(
        zone(),
        1,
        [provider_binding("runtime")],
        [(
            "workstation".to_owned(),
            ResourceRef::parse("Provider/runtime").expect("Provider route"),
        )],
    )
    .expect("compose Provider runtime");
    let admin = BrokerCallerRole::AdminUid { uid: 1000 };

    assert_eq!(
        runtime
            .dispatch_lifecycle(
                &admin,
                "workstation",
                GuestLifecycleOperation::Start,
                "activate-workstation",
                &effect,
            )
            .expect("activate Guest"),
        ProviderRuntimeDispatch::Active(EffectDispatch::Dispatched(GuestLifecycleState::Started))
    );
    assert_eq!(
        effect.actual_state(&request(GuestLifecycleOperation::Start, "state")),
        Ok(GuestLifecycleState::Started)
    );

    assert_eq!(
        runtime.dispatch_lifecycle(
            &BrokerCallerRole::NotAuthorized,
            "workstation",
            GuestLifecycleOperation::Stop,
            "refused-stop",
            &effect,
        ),
        Err(ProviderEffectError::CallerRoleDenied)
    );
    assert_eq!(
        effect.actual_state(&request(GuestLifecycleOperation::Start, "state")),
        Ok(GuestLifecycleState::Started),
        "authorization refusal must not mutate the process state"
    );

    assert_eq!(
        runtime
            .dispatch_lifecycle(
                &admin,
                "workstation",
                GuestLifecycleOperation::Stop,
                "remove-workstation",
                &effect,
            )
            .expect("remove Guest"),
        ProviderRuntimeDispatch::Active(EffectDispatch::Dispatched(GuestLifecycleState::Stopped))
    );
    assert_eq!(
        effect.actual_state(&request(GuestLifecycleOperation::Stop, "state")),
        Ok(GuestLifecycleState::Stopped)
    );
}

#[test]
fn pending_activation_is_adopted_after_daemon_restart() {
    let directory = tempfile::tempdir().expect("temporary lifecycle state");
    let state_path = directory.path().join("provider-lifecycle.json");
    let effect = FilesystemLifecycle::new(directory.path());
    let admin = BrokerCallerRole::AdminUid { uid: 1000 };
    let activation = request(GuestLifecycleOperation::Start, "restart-adoption");

    let first = ProviderLifecycleDispatch::new_persistent(zone(), &state_path)
        .expect("create first durable dispatcher");
    assert_eq!(
        first.admit(&admin, &activation).expect("admit activation"),
        d2bd::provider_effects::LifecycleDispatch::Dispatch
    );
    effect
        .apply(&activation)
        .expect("external process activation");
    drop(first);

    let restarted = ProviderLifecycleDispatch::new_persistent(zone(), &state_path)
        .expect("recreate durable dispatcher");
    assert_eq!(
        restarted
            .dispatch(&admin, &activation, &effect)
            .expect("adopt already-running process"),
        EffectDispatch::Duplicate
    );
    assert_eq!(
        effect.apply_calls.load(Ordering::SeqCst),
        1,
        "restart adoption must not launch a second process"
    );
}
