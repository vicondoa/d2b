//! Typed `d2b.activation.v2` service on the authenticated guest session.

use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::RequestId,
    v2_services::{
        StrictWireMessage,
        activation_ttrpc::ActivationService,
        common::{
            self, CancelOutcome, DesiredState, ErrorKind, ObservationState, Outcome, RetryClass,
        },
    },
};
use protobuf::{EnumOrUnknown, Message, MessageField};
use sha2::{Digest, Sha256};

use crate::{
    activation::{
        ACTIVATION_MAX_TIMEOUT_MS, ActivationCancelOutcome, ActivationError, ActivationRuntime,
        ActivationSnapshot, ActivationState,
    },
    guest_service::GuestServiceAccess,
    request_tracker::{GuestRequestAdmission, GuestRequestTicket, RequestAdmissionError},
};

const ACTIVATION_SERVICE_NAME: &str = "d2b.activation.v2.ActivationService";
const READINESS_OPERATION_ID: &str = "readiness";
const MONITOR_INTERVAL: Duration = Duration::from_millis(250);
const MONITOR_GRACE: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivationMethod {
    Activate,
    Inspect,
}

impl ActivationMethod {
    const fn name(self) -> &'static str {
        match self {
            Self::Activate => "Activate",
            Self::Inspect => "Inspect",
        }
    }

    const fn mutating(self) -> bool {
        matches!(self, Self::Activate)
    }
}

enum Admission {
    New(GuestRequestTicket),
    Replay(common::ServiceResponse),
}

pub struct ActivationServiceV2 {
    runtime: Arc<ActivationRuntime>,
    access: GuestServiceAccess,
}

impl std::fmt::Debug for ActivationServiceV2 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActivationServiceV2")
            .field("service", &ACTIVATION_SERVICE_NAME)
            .field("generation", &"<redacted>")
            .field("runtime", &self.runtime)
            .finish()
    }
}

impl ActivationServiceV2 {
    pub(crate) fn new(runtime: Arc<ActivationRuntime>, access: GuestServiceAccess) -> Arc<Self> {
        Arc::new(Self { runtime, access })
    }

    async fn call(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        method: ActivationMethod,
        request: common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        request
            .validate_wire(method.mutating())
            .map_err(|_| invalid_request())?;
        self.validate_request_shape(method, &request)?;
        if self.access.authorization.load(Ordering::Acquire) != 1 {
            return Err(rpc_error(
                ttrpc::Code::PERMISSION_DENIED,
                "activation-session-not-authorized",
            ));
        }
        let metadata = request.metadata.as_ref().ok_or_else(invalid_request)?;
        let scope = request.scope.as_ref().ok_or_else(invalid_request)?;
        if metadata.session_generation != self.access.session.generation()
            || scope.workload_id != self.runtime.workload_id()
            || scope.realm_id.is_empty()
            || !scope.provider_id.is_empty()
            || !scope.role_id.is_empty()
        {
            return Err(rpc_error(
                ttrpc::Code::PERMISSION_DENIED,
                "activation-scope-denied",
            ));
        }
        let ticket = match self.admit(context, method, &request).await? {
            Admission::Replay(response) => return Ok(response),
            Admission::New(ticket) => ticket,
        };

        let result = match method {
            ActivationMethod::Activate => {
                self.runtime
                    .activate(
                        &metadata.request_id,
                        &request.resource_id,
                        &request.operation_id,
                        &request.request_digest,
                    )
                    .await
            }
            ActivationMethod::Inspect if request.operation_id == READINESS_OPERATION_ID => {
                if self.runtime.ready() {
                    Ok(readiness_snapshot(&request.request_digest))
                } else {
                    Err(ActivationError::Unavailable)
                }
            }
            ActivationMethod::Inspect => {
                self.runtime
                    .inspect(
                        &request.resource_id,
                        &request.operation_id,
                        (!request.request_digest.is_empty())
                            .then_some(request.request_digest.as_slice()),
                    )
                    .await
            }
        };
        let response = match result {
            Ok(snapshot) => success_response(&request, snapshot),
            Err(error) => error_response(&request, error),
        };
        response.validate_wire(false).map_err(|_| service_error())?;
        let encoded = response.write_to_bytes().map_err(|_| service_error())?;
        let keep_active = method == ActivationMethod::Activate
            && response.outcome.enum_value().ok() == Some(Outcome::OUTCOME_ACCEPTED);
        self.access
            .requests
            .complete_response(&ticket, encoded, keep_active)
            .await;
        if keep_active {
            self.spawn_monitor(
                ticket,
                request.resource_id.clone(),
                request.operation_id.clone(),
            );
        }
        Ok(response)
    }

    async fn admit(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        method: ActivationMethod,
        request: &common::ServiceRequest,
    ) -> ttrpc::Result<Admission> {
        let metadata = request.metadata.as_ref().ok_or_else(invalid_request)?;
        let encoded = request.write_to_bytes().map_err(|_| invalid_request())?;
        let mut digest = Sha256::new();
        digest.update(b"d2b-activation-request-replay-v2\0");
        digest.update(method.name().as_bytes());
        digest.update(encoded);
        let peer_timeout = u64::try_from(context.timeout_nano)
            .ok()
            .filter(|timeout| *timeout != 0);
        match self
            .access
            .requests
            .admit(
                metadata,
                method.name(),
                &digest.finalize(),
                method.mutating(),
                self.access.clock.now_unix_ms(),
                peer_timeout,
            )
            .await
            .map_err(map_admission_error)?
        {
            GuestRequestAdmission::New(ticket) => Ok(Admission::New(ticket)),
            GuestRequestAdmission::Replay(bytes) => {
                common::ServiceResponse::parse_from_bytes(&bytes)
                    .map(Admission::Replay)
                    .map_err(|_| service_error())
            }
        }
    }

    fn validate_request_shape(
        &self,
        method: ActivationMethod,
        request: &common::ServiceRequest,
    ) -> ttrpc::Result<()> {
        if request.resource_id != self.runtime.configured_intent_id()
            || request.resource_id.is_empty()
            || request.operation_id.is_empty()
            || !request.stream_id.is_empty()
            || !request.attachment_indexes.is_empty()
            || request.page_size != 0
            || !request.page_cursor.is_empty()
        {
            return Err(invalid_request());
        }
        let desired_state = request
            .desired_state
            .enum_value()
            .map_err(|_| invalid_request())?;
        match method {
            ActivationMethod::Activate
                if request.request_digest.len() == 32
                    && request.request_digest.iter().any(|byte| *byte != 0)
                    && desired_state == DesiredState::DESIRED_STATE_RUNNING =>
            {
                Ok(())
            }
            ActivationMethod::Inspect
                if desired_state == DesiredState::DESIRED_STATE_UNSPECIFIED =>
            {
                Ok(())
            }
            _ => Err(invalid_request()),
        }
    }

    fn spawn_monitor(&self, ticket: GuestRequestTicket, intent_id: String, operation_id: String) {
        let runtime = Arc::clone(&self.runtime);
        let requests = Arc::clone(&self.access.requests);
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now()
                + Duration::from_millis(ACTIVATION_MAX_TIMEOUT_MS)
                + MONITOR_GRACE;
            loop {
                if tokio::time::Instant::now() >= deadline {
                    requests.finish(&ticket).await;
                    return;
                }
                match runtime.inspect(&intent_id, &operation_id, None).await {
                    Ok(snapshot) if snapshot.state.is_terminal() => {
                        requests.finish(&ticket).await;
                        return;
                    }
                    Ok(_) | Err(ActivationError::Unavailable) => {
                        tokio::time::sleep(MONITOR_INTERVAL).await;
                    }
                    Err(_) => {
                        requests.finish(&ticket).await;
                        return;
                    }
                }
            }
        });
    }
}

#[async_trait]
impl ActivationService for ActivationServiceV2 {
    async fn activate(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        self.call(context, ActivationMethod::Activate, request)
            .await
    }

    async fn inspect(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        self.call(context, ActivationMethod::Inspect, request).await
    }

    async fn cancel(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: common::CancelRequest,
    ) -> ttrpc::Result<common::CancelResponse> {
        request
            .validate_wire(false)
            .map_err(|_| invalid_request())?;
        if self.access.authorization.load(Ordering::Acquire) != 1 {
            return Err(rpc_error(
                ttrpc::Code::PERMISSION_DENIED,
                "activation-session-not-authorized",
            ));
        }
        if request.session_generation != self.access.session.generation()
            || RequestId::new(request.request_id.clone()).is_err()
        {
            return Ok(cancel_response(
                CancelOutcome::CANCEL_OUTCOME_GENERATION_MISMATCH,
            ));
        }
        let tracked = self.access.requests.cancel(&request).await;
        let runtime = self.runtime.cancel_by_request(&request.request_id).await;
        let outcome = match runtime {
            Ok(ActivationCancelOutcome::Signalled) => {
                CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED
            }
            Ok(ActivationCancelOutcome::AlreadyTerminal) => {
                CancelOutcome::CANCEL_OUTCOME_ALREADY_TERMINAL
            }
            Ok(ActivationCancelOutcome::NotFound) | Err(ActivationError::NotFound) => tracked
                .outcome
                .enum_value()
                .unwrap_or(CancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST),
            Err(_) => CancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST,
        };
        let response = cancel_response(outcome);
        response.validate_wire(false).map_err(|_| service_error())?;
        Ok(response)
    }
}

fn success_response(
    request: &common::ServiceRequest,
    snapshot: ActivationSnapshot,
) -> common::ServiceResponse {
    let outcome = match snapshot.state {
        ActivationState::Running => Outcome::OUTCOME_ACCEPTED,
        ActivationState::Succeeded => Outcome::OUTCOME_SUCCEEDED,
        ActivationState::Cancelled => Outcome::OUTCOME_CANCELLED,
        ActivationState::Lost => Outcome::OUTCOME_DEGRADED,
        ActivationState::Failed | ActivationState::TimedOut => Outcome::OUTCOME_FAILED,
    };
    let digest = snapshot.result_digest(&request.resource_id, &request.operation_id);
    let error = match snapshot.state {
        ActivationState::Failed => Some(error_envelope(
            ErrorKind::ERROR_KIND_INTERNAL,
            RetryClass::RETRY_CLASS_AFTER_INTERACTION,
        )),
        ActivationState::TimedOut => Some(error_envelope(
            ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED,
            RetryClass::RETRY_CLASS_AFTER_INTERACTION,
        )),
        ActivationState::Cancelled => Some(error_envelope(
            ErrorKind::ERROR_KIND_CANCELLED,
            RetryClass::RETRY_CLASS_NEVER,
        )),
        ActivationState::Lost => Some(error_envelope(
            ErrorKind::ERROR_KIND_UNAVAILABLE,
            RetryClass::RETRY_CLASS_AFTER_OBSERVATION,
        )),
        ActivationState::Running | ActivationState::Succeeded => None,
    };
    common::ServiceResponse {
        outcome: EnumOrUnknown::new(outcome),
        operation_id: request.operation_id.clone(),
        resource_handle: request.operation_id.clone(),
        result_digest: digest.to_vec(),
        observations: vec![common::Observation {
            resource_id: request.resource_id.clone(),
            state: EnumOrUnknown::new(match snapshot.state {
                ActivationState::Running => ObservationState::OBSERVATION_STATE_RUNNING,
                ActivationState::Succeeded => ObservationState::OBSERVATION_STATE_READY,
                ActivationState::Cancelled => ObservationState::OBSERVATION_STATE_STOPPED,
                ActivationState::Lost => ObservationState::OBSERVATION_STATE_DEGRADED,
                ActivationState::Failed | ActivationState::TimedOut => {
                    ObservationState::OBSERVATION_STATE_FAILED
                }
            }),
            generation: request
                .metadata
                .as_ref()
                .map(|metadata| metadata.session_generation)
                .unwrap_or(1),
            digest: digest.to_vec(),
            ..Default::default()
        }],
        error: error.map(MessageField::some).unwrap_or_default(),
        ..Default::default()
    }
}

fn error_response(
    request: &common::ServiceRequest,
    error: ActivationError,
) -> common::ServiceResponse {
    let (kind, retry) = match error {
        ActivationError::InvalidRequest => (
            ErrorKind::ERROR_KIND_INVALID_REQUEST,
            RetryClass::RETRY_CLASS_NEVER,
        ),
        ActivationError::Unavailable | ActivationError::SpawnFailed => (
            ErrorKind::ERROR_KIND_UNAVAILABLE,
            RetryClass::RETRY_CLASS_AFTER_OBSERVATION,
        ),
        ActivationError::NotFound => (
            ErrorKind::ERROR_KIND_NOT_FOUND,
            RetryClass::RETRY_CLASS_AFTER_OBSERVATION,
        ),
        ActivationError::Conflict => (
            ErrorKind::ERROR_KIND_CONFLICT,
            RetryClass::RETRY_CLASS_NEVER,
        ),
        ActivationError::TimedOut => (
            ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED,
            RetryClass::RETRY_CLASS_AFTER_INTERACTION,
        ),
    };
    common::ServiceResponse {
        outcome: EnumOrUnknown::new(Outcome::OUTCOME_FAILED),
        operation_id: request.operation_id.clone(),
        error: MessageField::some(error_envelope(kind, retry)),
        ..Default::default()
    }
}

fn error_envelope(kind: ErrorKind, retry: RetryClass) -> common::ErrorEnvelope {
    common::ErrorEnvelope {
        kind: EnumOrUnknown::new(kind),
        retry: EnumOrUnknown::new(retry),
        ..Default::default()
    }
}

fn readiness_snapshot(seed: &[u8]) -> ActivationSnapshot {
    let mut digest = Sha256::new();
    digest.update(b"d2b-guest-activation-readiness-v2\0");
    digest.update(seed);
    ActivationSnapshot {
        state: ActivationState::Succeeded,
        exit_code: Some(0),
        signal: None,
        status_code: None,
        intent_digest: digest.finalize().into(),
    }
}

fn cancel_response(outcome: CancelOutcome) -> common::CancelResponse {
    common::CancelResponse {
        outcome: EnumOrUnknown::new(outcome),
        ..Default::default()
    }
}

fn map_admission_error(error: RequestAdmissionError) -> ttrpc::Error {
    match error {
        RequestAdmissionError::Deadline => rpc_error(
            ttrpc::Code::DEADLINE_EXCEEDED,
            "activation-deadline-invalid",
        ),
        RequestAdmissionError::Duplicate | RequestAdmissionError::ReplayConflict => {
            rpc_error(ttrpc::Code::ALREADY_EXISTS, "activation-replay-conflict")
        }
        RequestAdmissionError::Cancelled => {
            rpc_error(ttrpc::Code::CANCELLED, "activation-request-cancelled")
        }
        RequestAdmissionError::Session => rpc_error(
            ttrpc::Code::FAILED_PRECONDITION,
            "activation-session-invalid",
        ),
    }
}

fn invalid_request() -> ttrpc::Error {
    rpc_error(ttrpc::Code::INVALID_ARGUMENT, "activation-request-invalid")
}

fn service_error() -> ttrpc::Error {
    rpc_error(ttrpc::Code::INTERNAL, "activation-service-failed-closed")
}

fn rpc_error(code: ttrpc::Code, message: &'static str) -> ttrpc::Error {
    ttrpc::Error::RpcStatus(ttrpc::get_status(code, message.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Instant,
    };

    use d2b_contracts::v2_component_session::{CloseReason, Remediation, SessionErrorCode};
    use d2b_session::{
        Cancellation, ComponentSessionDriver, OwnedAttachment, RequestRegistry, SessionError,
        SessionEvent, StreamEvent, StreamId,
    };

    use super::*;
    use crate::{
        activation::{
            ACTIVATION_PAYLOAD_FILE, ActivationMode, ActivationRuntimeConfig,
            ActivationUnitManager, encode_activation_payload_for_test,
        },
        request_tracker::GuestRequestTracker,
    };

    const GENERATION: u64 = 9;
    const NOW_MS: u64 = 10_000;
    const WORKLOAD_ID: &str = "bbbbbbbbbbbbbbbbbbba";
    const OPERATION_ID: &str = "activation-0123456789abcdef0123456789abcdef";

    struct FixedClock(u64);

    impl crate::guest_service::GuestWallClock for FixedClock {
        fn now_unix_ms(&self) -> u64 {
            self.0
        }
    }

    struct FakeDriver {
        requests: Mutex<RequestRegistry>,
    }

    impl FakeDriver {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::new(RequestRegistry::new(GENERATION).unwrap()),
            })
        }
    }

    fn session_error() -> SessionError {
        SessionError::new(SessionErrorCode::InternalInvariant)
    }

    #[async_trait]
    impl ComponentSessionDriver for FakeDriver {
        fn generation(&self) -> u64 {
            GENERATION
        }

        async fn start_ttrpc(&self, _: RequestId, _: Vec<u8>) -> d2b_session::Result<()> {
            Err(session_error())
        }

        async fn complete_ttrpc(&self, _: RequestId) -> d2b_session::Result<bool> {
            Err(session_error())
        }

        async fn cancel(&self, _: u64, _: RequestId) -> d2b_session::Result<()> {
            Err(session_error())
        }

        async fn send_ttrpc(&self, _: Vec<u8>) -> d2b_session::Result<()> {
            Err(session_error())
        }

        async fn receive_ttrpc(&self) -> d2b_session::Result<Vec<u8>> {
            Err(session_error())
        }

        async fn register_inbound_call(
            &self,
            request_id: RequestId,
        ) -> d2b_session::Result<Cancellation> {
            self.requests.lock().unwrap().register(request_id)
        }

        async fn complete_inbound_call(&self, request_id: RequestId) -> d2b_session::Result<bool> {
            Ok(self.requests.lock().unwrap().complete(&request_id))
        }

        async fn remove_inbound_call(&self, request_id: RequestId) -> d2b_session::Result<bool> {
            Ok(self.requests.lock().unwrap().remove(&request_id))
        }

        async fn send_attachments(&self, _: Vec<OwnedAttachment>) -> d2b_session::Result<()> {
            Err(session_error())
        }

        async fn receive_attachments(&self) -> d2b_session::Result<Vec<OwnedAttachment>> {
            Err(session_error())
        }

        async fn open_named_stream(&self, _: StreamId, _: u32, _: u32) -> d2b_session::Result<()> {
            Err(session_error())
        }

        async fn send_named_stream(&self, _: StreamId, _: Vec<u8>) -> d2b_session::Result<()> {
            Err(session_error())
        }

        async fn receive_named_stream(&self) -> d2b_session::Result<StreamEvent> {
            Err(session_error())
        }

        async fn grant_named_stream_credit(&self, _: StreamId, _: u32) -> d2b_session::Result<()> {
            Err(session_error())
        }

        async fn close_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
            Err(session_error())
        }

        async fn reset_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
            Err(session_error())
        }

        async fn drive_keepalive(&self, _: Instant) -> d2b_session::Result<()> {
            Err(session_error())
        }

        async fn receive_control(&self) -> d2b_session::Result<SessionEvent> {
            Err(session_error())
        }

        async fn close(&self, _: CloseReason, _: Remediation) -> d2b_session::Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeUnits {
        starts: AtomicUsize,
        cancels: AtomicUsize,
        query: Mutex<Option<ActivationSnapshot>>,
    }

    #[async_trait]
    impl ActivationUnitManager for FakeUnits {
        async fn start_unit(
            &self,
            _: &str,
            _: &Path,
            _: ActivationMode,
            _: u64,
        ) -> Result<(), ActivationError> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn query_unit(
            &self,
            _: &str,
            _: [u8; 32],
        ) -> Result<Option<ActivationSnapshot>, ActivationError> {
            Ok(self.query.lock().unwrap().clone())
        }

        async fn cancel_unit(&self, _: &str) -> Result<(), ActivationError> {
            self.cancels.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn cleanup_terminal_unit(
            &self,
            _: &str,
            _: &ActivationSnapshot,
        ) -> Result<(), ActivationError> {
            Ok(())
        }
    }

    struct TestTree {
        root: PathBuf,
        status: PathBuf,
        switch: PathBuf,
    }

    impl TestTree {
        fn new(tag: &str) -> Self {
            let root = std::env::var_os("CARGO_TARGET_TMPDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap().join("target/test-tmp"))
                .join(format!("activation-service-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let status = root.join("status");
            fs::create_dir_all(&status).unwrap();
            fs::set_permissions(&status, fs::Permissions::from_mode(0o700)).unwrap();
            let store = root.join("nix/store");
            let switch = store
                .join("0123456789abcdfghijklmnpqrsvwxyz-nixos-system")
                .join("bin/switch-to-configuration");
            fs::create_dir_all(switch.parent().unwrap()).unwrap();
            executable(&switch);
            Self {
                root,
                status,
                switch,
            }
        }

        fn runtime(&self, units: Arc<dyn ActivationUnitManager>) -> Arc<ActivationRuntime> {
            let systemd_run = executable(&self.root.join("systemd-run"));
            let systemctl = executable(&self.root.join("systemctl"));
            ActivationRuntime::with_manager(
                ActivationRuntimeConfig {
                    workload_id: WORKLOAD_ID.to_owned(),
                    systemd_run_path: systemd_run,
                    systemctl_path: systemctl,
                    status_dir: self.status.clone(),
                    switch_store_root: self.root.join("nix/store"),
                    max_timeout_ms: 30_000,
                },
                units,
            )
        }

        fn install_payload(
            &self,
            runtime: &ActivationRuntime,
            operation_id: &str,
            timeout_ms: u64,
        ) -> [u8; 32] {
            let bytes = encode_activation_payload_for_test(
                runtime.configured_intent_id(),
                operation_id,
                &self.switch.display().to_string(),
                ActivationMode::Switch,
                timeout_ms,
            );
            let path = self.status.join(ACTIVATION_PAYLOAD_FILE);
            fs::write(&path, &bytes).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            Sha256::digest(&bytes).into()
        }
    }

    impl Drop for TestTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn executable(path: &Path) -> PathBuf {
        fs::write(path, b"binary").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        path.to_path_buf()
    }

    fn service(
        runtime: Arc<ActivationRuntime>,
        authorized: bool,
        now_ms: u64,
    ) -> Arc<ActivationServiceV2> {
        let concrete = FakeDriver::new();
        let driver: Arc<dyn ComponentSessionDriver> = concrete;
        let access = GuestServiceAccess {
            session: Arc::clone(&driver),
            authorization: Arc::new(std::sync::atomic::AtomicU8::new(u8::from(authorized))),
            requests: Arc::new(GuestRequestTracker::new(GENERATION, driver).unwrap()),
            clock: Arc::new(FixedClock(now_ms)),
        };
        ActivationServiceV2::new(runtime, access)
    }

    fn request(runtime: &ActivationRuntime, digest: [u8; 32]) -> common::ServiceRequest {
        common::ServiceRequest {
            metadata: MessageField::some(common::RequestMetadata {
                request_id: vec![0x11; 16],
                idempotency_key: vec![0x22; 32],
                issued_at_unix_ms: NOW_MS,
                expires_at_unix_ms: NOW_MS + 5_000,
                session_generation: GENERATION,
                ..Default::default()
            }),
            scope: MessageField::some(common::IdentityScope {
                realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
                workload_id: WORKLOAD_ID.to_owned(),
                ..Default::default()
            }),
            resource_id: runtime.configured_intent_id().to_owned(),
            operation_id: OPERATION_ID.to_owned(),
            request_digest: digest.to_vec(),
            desired_state: EnumOrUnknown::new(DesiredState::DESIRED_STATE_RUNNING),
            ..Default::default()
        }
    }

    fn inspect_request(runtime: &ActivationRuntime, digest: [u8; 32]) -> common::ServiceRequest {
        let mut request = request(runtime, digest);
        let metadata = request.metadata.as_mut().unwrap();
        metadata.request_id = vec![0x33; 16];
        metadata.idempotency_key.clear();
        request.desired_state = EnumOrUnknown::new(DesiredState::DESIRED_STATE_UNSPECIFIED);
        request
    }

    fn context() -> ttrpc::r#async::TtrpcContext {
        ttrpc::r#async::TtrpcContext {
            mh: ttrpc::proto::MessageHeader::new_request(1, 0),
            metadata: Default::default(),
            timeout_nano: Duration::from_secs(5).as_nanos() as i64,
        }
    }

    #[tokio::test]
    async fn activation_admission_is_strict_for_auth_scope_generation_and_deadline() {
        let tree = TestTree::new("admission");
        let runtime = tree.runtime(Arc::new(FakeUnits::default()));
        let digest = tree.install_payload(&runtime, OPERATION_ID, 5_000);

        assert!(
            service(Arc::clone(&runtime), false, NOW_MS)
                .activate(&context(), request(&runtime, digest))
                .await
                .is_err()
        );
        let authorized = service(Arc::clone(&runtime), true, NOW_MS);
        let mut wrong_scope = request(&runtime, digest);
        wrong_scope.scope.as_mut().unwrap().workload_id = "ccccccccccccccccccca".to_owned();
        assert!(authorized.activate(&context(), wrong_scope).await.is_err());
        let mut wrong_generation = request(&runtime, digest);
        wrong_generation
            .metadata
            .as_mut()
            .unwrap()
            .session_generation += 1;
        assert!(
            authorized
                .activate(&context(), wrong_generation)
                .await
                .is_err()
        );
        let expired = service(Arc::clone(&runtime), true, NOW_MS + 10_000);
        assert!(
            expired
                .activate(&context(), request(&runtime, digest))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn activation_service_starts_inspects_rejoins_cancels_and_replays() {
        let tree = TestTree::new("lifecycle");
        let units = Arc::new(FakeUnits::default());
        let runtime = tree.runtime(units.clone());
        let digest = tree.install_payload(&runtime, OPERATION_ID, 5_000);
        let service = service(Arc::clone(&runtime), true, NOW_MS);
        let activate = request(&runtime, digest);
        let started = service
            .activate(&context(), activate.clone())
            .await
            .unwrap();
        assert_eq!(
            started.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_ACCEPTED
        );
        assert_eq!(units.starts.load(Ordering::SeqCst), 1);

        *units.query.lock().unwrap() = Some(ActivationSnapshot {
            state: ActivationState::Succeeded,
            exit_code: Some(0),
            signal: None,
            status_code: None,
            intent_digest: digest,
        });
        let inspected = service
            .inspect(&context(), inspect_request(&runtime, digest))
            .await
            .unwrap();
        assert_eq!(
            inspected.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        let replay = service.activate(&context(), activate).await.unwrap();
        assert_eq!(
            replay.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_ACCEPTED
        );
        assert_eq!(units.starts.load(Ordering::SeqCst), 1);

        let restarted = tree.runtime(units.clone());
        let rejoined = restarted
            .inspect(
                restarted.configured_intent_id(),
                OPERATION_ID,
                Some(&digest),
            )
            .await
            .unwrap();
        assert_eq!(rejoined.state, ActivationState::Succeeded);

        let cancel_operation = "activation-fedcba9876543210fedcba9876543210";
        let cancel_digest = tree.install_payload(&runtime, cancel_operation, 5_000);
        let mut cancel_start = request(&runtime, cancel_digest);
        cancel_start.operation_id = cancel_operation.to_owned();
        cancel_start.metadata.as_mut().unwrap().request_id = vec![0x44; 16];
        cancel_start.metadata.as_mut().unwrap().idempotency_key = vec![0x55; 32];
        service.activate(&context(), cancel_start).await.unwrap();
        let cancelled = service
            .cancel(
                &context(),
                common::CancelRequest {
                    session_generation: GENERATION,
                    request_id: vec![0x44; 16],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            cancelled.outcome.enum_value().unwrap(),
            CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED
        );
        assert_eq!(units.cancels.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn activation_timeout_is_a_closed_redacted_failure() {
        let tree = TestTree::new("timeout");
        let units = Arc::new(FakeUnits::default());
        let runtime = tree.runtime(units.clone());
        let digest = tree.install_payload(&runtime, OPERATION_ID, 1_000);
        let service = service(Arc::clone(&runtime), true, NOW_MS);
        service
            .activate(&context(), request(&runtime, digest))
            .await
            .unwrap();
        *units.query.lock().unwrap() = Some(ActivationSnapshot {
            state: ActivationState::TimedOut,
            exit_code: None,
            signal: None,
            status_code: None,
            intent_digest: digest,
        });
        let response = service
            .inspect(&context(), inspect_request(&runtime, digest))
            .await
            .unwrap();
        assert_eq!(
            response.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_FAILED
        );
        assert_eq!(
            response.error.as_ref().unwrap().kind.enum_value().unwrap(),
            ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED
        );
        let rendered = format!("{service:?}");
        assert!(!rendered.contains(&tree.switch.display().to_string()));
    }
}
