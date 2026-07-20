//! Systemd-activated ComponentSession composition for the clipboard daemon.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{
        AttachmentPolicy, AttachmentPolicyKind, CloseReason, EndpointPolicy, EndpointPurpose,
        EndpointRole, IdentityEvidenceRequirement, LimitProfile, Locality, NoiseProfile,
        PurposeClass, Remediation, ServicePackage, TransportBinding, TransportClass,
    },
    v2_services::{
        SERVICE_INVENTORY,
        clipboard_picker_ttrpc::{
            ClipboardPickerService as ClipboardPickerRpc, create_clipboard_picker_service,
        },
        clipboard_ttrpc::{ClipboardService as ClipboardRpc, create_clipboard_service},
        common::{
            CancelOutcome, CancelRequest, CancelResponse, Outcome, ServiceRequest, ServiceResponse,
        },
        service_schema_fingerprint,
    },
};
use d2b_session::{
    ComponentSessionDriver, HandshakeCredentials, SessionEngine, serve_ttrpc_services,
};
use d2b_session_unix::{
    ActivatedSeqpacketListeners, CreditPool, CreditScopeSet, PeerIdentityPolicy, SeqpacketSocket,
    UnixSeqpacketTransport, UnixSessionError,
};
use sha2::{Digest, Sha256};
use tokio::{
    runtime::Builder,
    sync::{Semaphore, mpsc, watch},
    task::JoinSet,
};

use crate::{
    framing::PickerProjectionBounds,
    protocol::{ClipboardTarget, OfferQuery, OfferSelection, OpaquePickerId},
    services::{
        ClipboardServiceError, ClipboardServices, ClipboardServicesConfig,
        ClipboardSessionTransport, EstablishedClipboardSession,
        control::{AdmittedCall, ControlInput, ControlOutcome, ControlPeer, ControlResponse},
        picker::{CancelSelection, PickerCall, PickerOperation, PickerResponse},
    },
};

pub const ACTIVATED_LISTENER_NAMES: [&str; 3] =
    ["clipboard-control", "clipboard-picker", "clipboard-bridge"];

const MAX_CONCURRENT_ENDPOINT_SESSIONS: usize = 24;
const SESSION_QUEUE_CAPACITY: usize = 8;
const DIAGNOSTIC_DRAIN_BATCH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonError {
    Activation,
    Runtime,
    Session,
}

impl DaemonError {
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::Activation => 78,
            Self::Runtime | Self::Session => 1,
        }
    }
}

impl std::fmt::Display for DaemonError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Activation => "clipboard-activation-invalid",
            Self::Runtime => "clipboard-runtime-failed",
            Self::Session => "clipboard-session-failed",
        })
    }
}

impl std::error::Error for DaemonError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Endpoint {
    Control,
    Picker,
    Bridge,
}

impl Endpoint {
    const ALL: [Self; 3] = [Self::Control, Self::Picker, Self::Bridge];

    const fn listener_name(self) -> &'static str {
        match self {
            Self::Control => "clipboard-control",
            Self::Picker => "clipboard-picker",
            Self::Bridge => "clipboard-bridge",
        }
    }

    const fn purpose(self) -> EndpointPurpose {
        match self {
            Self::Control => EndpointPurpose::ClipboardControl,
            Self::Picker => EndpointPurpose::ClipboardPicker,
            Self::Bridge => EndpointPurpose::ClipboardBridge,
        }
    }

    const fn initiator_role(self) -> EndpointRole {
        match self {
            Self::Control => EndpointRole::UserAgent,
            Self::Picker => EndpointRole::ClipboardDaemon,
            Self::Bridge => EndpointRole::WaylandProxy,
        }
    }

    const fn responder_role(self) -> EndpointRole {
        match self {
            Self::Control | Self::Bridge => EndpointRole::ClipboardDaemon,
            Self::Picker => EndpointRole::ClipboardPicker,
        }
    }

    const fn service(self) -> ServicePackage {
        match self {
            Self::Control | Self::Bridge => ServicePackage::ClipboardV2,
            Self::Picker => ServicePackage::ClipboardPickerV2,
        }
    }

    const fn package(self) -> &'static str {
        match self {
            Self::Control | Self::Bridge => "d2b.clipboard.v2",
            Self::Picker => "d2b.clipboard.picker.v2",
        }
    }

    const fn attachment_policy(self) -> AttachmentPolicy {
        match self {
            Self::Bridge => AttachmentPolicy {
                kind: AttachmentPolicyKind::PacketAtomic,
                max_per_packet: 1,
                max_per_request: 1,
                max_per_operation: 1,
                max_per_session: 1,
                credentials_allowed: false,
            },
            Self::Control | Self::Picker => AttachmentPolicy {
                kind: AttachmentPolicyKind::Disabled,
                max_per_packet: 0,
                max_per_request: 0,
                max_per_operation: 0,
                max_per_session: 0,
                credentials_allowed: false,
            },
        }
    }

    const fn local_initiates(self) -> bool {
        matches!(self, Self::Picker)
    }
}

#[derive(Clone)]
struct SessionEvidence {
    endpoint: Endpoint,
    generation: u64,
    authenticated_realm: Option<String>,
}

impl EstablishedClipboardSession for SessionEvidence {
    fn service_package(&self) -> &str {
        self.endpoint.package()
    }

    fn endpoint_purpose(&self) -> &str {
        self.endpoint.listener_name()
    }

    fn endpoint_role(&self) -> &str {
        self.endpoint.responder_role().as_str()
    }

    fn generation(&self) -> u64 {
        self.generation
    }

    fn transport(&self) -> ClipboardSessionTransport {
        ClipboardSessionTransport::UnixSeqpacket
    }

    fn authenticated_realm(&self) -> Option<&str> {
        self.authenticated_realm.as_deref()
    }

    fn is_established(&self) -> bool {
        true
    }

    fn is_authenticated(&self) -> bool {
        true
    }

    fn is_host_local(&self) -> bool {
        true
    }

    fn uses_pre_authorized_transport(&self) -> bool {
        true
    }

    fn attachments_present(&self) -> bool {
        false
    }
}

struct EstablishedEndpoint {
    evidence: SessionEvidence,
    driver: Arc<dyn ComponentSessionDriver>,
    peer: PeerKey,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PeerKey {
    uid: u32,
    gid: u32,
}

pub fn run() -> Result<(), DaemonError> {
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .thread_name("d2b-clipd")
        .build()
        .map_err(|_| DaemonError::Runtime)?;
    runtime.block_on(run_async())
}

async fn run_async() -> Result<(), DaemonError> {
    let listeners = Arc::new(
        ActivatedSeqpacketListeners::from_systemd(&ACTIVATED_LISTENER_NAMES)
            .map_err(|_| DaemonError::Activation)?,
    );
    let generation = process_generation()?;
    run_activated(listeners, generation, shutdown_signal()).await
}

async fn run_activated<F>(
    listeners: Arc<ActivatedSeqpacketListeners>,
    generation: u64,
    shutdown: F,
) -> Result<(), DaemonError>
where
    F: Future<Output = ()> + Send,
{
    if generation == 0 {
        return Err(DaemonError::Runtime);
    }
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_ENDPOINT_SESSIONS));
    let (control_tx, control_rx) = mpsc::channel(SESSION_QUEUE_CAPACITY);
    let (picker_tx, picker_rx) = mpsc::channel(SESSION_QUEUE_CAPACITY);
    let (bridge_tx, bridge_rx) = mpsc::channel(SESSION_QUEUE_CAPACITY);
    let mut acceptors = JoinSet::new();
    for (endpoint, sender) in Endpoint::ALL
        .into_iter()
        .zip([control_tx, picker_tx, bridge_tx])
    {
        acceptors.spawn(accept_endpoint(
            Arc::clone(&listeners),
            endpoint,
            generation,
            Arc::clone(&permits),
            sender,
            shutdown_rx.clone(),
        ));
    }

    tokio::pin!(shutdown);
    let composition = compose_sessions(control_rx, picker_rx, bridge_rx, shutdown_rx.clone());
    tokio::pin!(composition);
    let result = tokio::select! {
        result = &mut composition => result,
        () = &mut shutdown => {
            let _ = shutdown_tx.send(true);
            composition.await
        },
    };
    let _ = shutdown_tx.send(true);
    if tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while acceptors.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        acceptors.abort_all();
        while acceptors.join_next().await.is_some() {}
    }
    result
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut terminate) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

async fn accept_endpoint(
    listeners: Arc<ActivatedSeqpacketListeners>,
    endpoint: Endpoint,
    generation: u64,
    permits: Arc<Semaphore>,
    sender: mpsc::Sender<EstablishedEndpoint>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut handshakes = JoinSet::new();
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            result = listeners.accept(endpoint.listener_name()) => {
                let Ok(socket) = result else {
                    break;
                };
                let permit = tokio::select! {
                    permit = Arc::clone(&permits).acquire_owned() => match permit {
                        Ok(permit) => permit,
                        Err(_) => break,
                    },
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            break;
                        }
                        continue;
                    }
                };
                let sender = sender.clone();
                handshakes.spawn(async move {
                    match establish_endpoint(socket, endpoint, generation, permit).await {
                        Ok(session) => {
                            let _ = sender.send(session).await;
                        }
                        Err(error) => {
                            log::warn!(
                                "d2b-clipd: endpoint={} event=session-rejected reason={error}",
                                endpoint.listener_name()
                            );
                        }
                    }
                });
            }
            Some(_) = handshakes.join_next(), if !handshakes.is_empty() => {}
        }
    }
    handshakes.abort_all();
    while handshakes.join_next().await.is_some() {}
}

async fn establish_endpoint(
    socket: SeqpacketSocket,
    endpoint: Endpoint,
    generation: u64,
    permit: tokio::sync::OwnedSemaphorePermit,
) -> Result<EstablishedEndpoint, DaemonError> {
    let credentials = socket
        .acceptor_peer_credentials()
        .map_err(|_| DaemonError::Session)?;
    let uid = credentials.uid().as_raw();
    let gid = credentials.gid().as_raw();
    let policy = endpoint_policy(endpoint, generation, uid, gid)?;
    let credits = credit_scopes(endpoint.attachment_policy().max_per_session);
    let resolver = Arc::new(|_: &_| Err(UnixSessionError::DescriptorMismatch));
    let transport = UnixSeqpacketTransport::new(
        socket,
        Locality::HostLocal,
        policy.limits,
        policy.attachment_policy,
        credits,
        resolver,
        PeerIdentityPolicy::accepted(credentials),
    )
    .map_err(|_| DaemonError::Session)?;
    let engine = if endpoint.local_initiates() {
        SessionEngine::establish_initiator(
            transport,
            policy,
            HandshakeCredentials::Nn,
            Instant::now(),
        )
        .await
    } else {
        SessionEngine::establish_responder(
            transport,
            policy,
            HandshakeCredentials::Nn,
            Instant::now(),
        )
        .await
    }
    .map_err(|_| DaemonError::Session)?;
    let driver: Arc<dyn ComponentSessionDriver> = Arc::new(engine.into_driver());
    Ok(EstablishedEndpoint {
        evidence: SessionEvidence {
            endpoint,
            generation,
            authenticated_realm: matches!(endpoint, Endpoint::Bridge).then(|| format!("uid-{uid}")),
        },
        driver,
        peer: PeerKey { uid, gid },
        _permit: permit,
    })
}

fn endpoint_policy(
    endpoint: Endpoint,
    generation: u64,
    uid: u32,
    gid: u32,
) -> Result<EndpointPolicy, DaemonError> {
    let service = SERVICE_INVENTORY
        .iter()
        .find(|service| service.package == endpoint.package())
        .ok_or(DaemonError::Runtime)?;
    Ok(EndpointPolicy {
        purpose: endpoint.purpose(),
        purpose_class: PurposeClass::Local,
        initiator_role: endpoint.initiator_role(),
        responder_role: endpoint.responder_role(),
        service: endpoint.service(),
        schema_fingerprint: service_schema_fingerprint(service),
        noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            channel_binding: endpoint_channel_binding(endpoint, uid, gid),
            identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
        },
        reconnect_generation: generation,
        attachment_policy: endpoint.attachment_policy(),
    })
}

fn endpoint_channel_binding(endpoint: Endpoint, uid: u32, gid: u32) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b.clipboard\0unix-seqpacket\0");
    digest.update(endpoint.listener_name().as_bytes());
    digest.update(b"\0");
    digest.update(uid.to_be_bytes());
    digest.update(gid.to_be_bytes());
    digest.finalize().into()
}

fn credit_scopes(limit: u16) -> CreditScopeSet {
    let limit = usize::from(limit.max(1));
    CreditScopeSet::new(
        CreditPool::new(limit).expect("positive clipboard packet credit"),
        CreditPool::new(limit).expect("positive clipboard request credit"),
        CreditPool::new(limit).expect("positive clipboard operation credit"),
        CreditPool::new(limit).expect("positive clipboard session credit"),
        CreditPool::new(limit).expect("positive clipboard process credit"),
        CreditPool::new(limit).expect("positive clipboard host credit"),
    )
}

fn process_generation() -> Result<u64, DaemonError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DaemonError::Runtime)?;
    let mut digest = Sha256::new();
    digest.update(b"d2b-clipd-generation\0");
    digest.update(std::process::id().to_be_bytes());
    digest.update(elapsed.as_nanos().to_be_bytes());
    let bytes: [u8; 8] = digest.finalize()[..8]
        .try_into()
        .map_err(|_| DaemonError::Runtime)?;
    Ok(u64::from_be_bytes(bytes).max(1))
}

async fn compose_sessions(
    mut control_rx: mpsc::Receiver<EstablishedEndpoint>,
    mut picker_rx: mpsc::Receiver<EstablishedEndpoint>,
    mut bridge_rx: mpsc::Receiver<EstablishedEndpoint>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), DaemonError> {
    let mut pending = BTreeMap::<PeerKey, PendingSessions>::new();
    let mut groups = JoinSet::new();
    let mut requested_shutdown = false;
    loop {
        spawn_ready_groups(&mut pending, &mut groups, &shutdown);
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    requested_shutdown = true;
                    break;
                }
            }
            Some(session) = control_rx.recv() => {
                pending.entry(session.peer).or_default().controls.push_back(session);
            }
            Some(session) = picker_rx.recv() => {
                pending.entry(session.peer).or_default().pickers.push_back(session);
            }
            Some(session) = bridge_rx.recv() => {
                pending.entry(session.peer).or_default().bridges.push_back(session);
            }
            Some(result) = groups.join_next(), if !groups.is_empty() => {
                if let Ok(Err(error)) = result {
                    log::warn!("d2b-clipd: event=session-group-closed reason={error}");
                }
            }
            else => break,
        }
    }
    if requested_shutdown {
        for session in pending.into_values().flat_map(PendingSessions::into_all) {
            let _ = session
                .driver
                .close(CloseReason::Normal, Remediation::None)
                .await;
        }
        if tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while groups.join_next().await.is_some() {}
        })
        .await
        .is_err()
        {
            groups.abort_all();
            while groups.join_next().await.is_some() {}
        }
    } else {
        groups.abort_all();
        while groups.join_next().await.is_some() {}
    }
    Ok(())
}

#[derive(Default)]
struct PendingSessions {
    controls: VecDeque<EstablishedEndpoint>,
    pickers: VecDeque<EstablishedEndpoint>,
    bridges: VecDeque<EstablishedEndpoint>,
}

impl PendingSessions {
    fn ready(&self) -> bool {
        !self.controls.is_empty() && !self.pickers.is_empty() && !self.bridges.is_empty()
    }

    fn pop_group(
        &mut self,
    ) -> Option<(
        EstablishedEndpoint,
        EstablishedEndpoint,
        EstablishedEndpoint,
    )> {
        self.ready().then(|| {
            (
                self.controls.pop_front().expect("checked nonempty"),
                self.pickers.pop_front().expect("checked nonempty"),
                self.bridges.pop_front().expect("checked nonempty"),
            )
        })
    }

    fn into_all(self) -> impl Iterator<Item = EstablishedEndpoint> {
        self.controls
            .into_iter()
            .chain(self.pickers)
            .chain(self.bridges)
    }
}

fn spawn_ready_groups(
    pending: &mut BTreeMap<PeerKey, PendingSessions>,
    groups: &mut JoinSet<Result<(), DaemonError>>,
    shutdown: &watch::Receiver<bool>,
) {
    for sessions in pending.values_mut() {
        while let Some((control, picker, bridge)) = sessions.pop_group() {
            groups.spawn(serve_group(control, picker, bridge, shutdown.clone()));
        }
    }
    pending.retain(|_, sessions| {
        !sessions.controls.is_empty()
            || !sessions.pickers.is_empty()
            || !sessions.bridges.is_empty()
    });
}

async fn serve_group(
    control: EstablishedEndpoint,
    picker: EstablishedEndpoint,
    bridge: EstablishedEndpoint,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), DaemonError> {
    let services = ClipboardServices::start(
        &control.evidence,
        &bridge.evidence,
        &picker.evidence,
        ClipboardServicesConfig::default(),
    )
    .map_err(|_| DaemonError::Session)?;
    let shared = SharedServices(Arc::new(Mutex::new(services)));

    let control_services = create_clipboard_service(Arc::new(ClipboardRpcAdapter::new(
        shared.clone(),
        ControlPeer::CommandClient,
    )));
    let bridge_services = create_clipboard_service(Arc::new(ClipboardRpcAdapter::new(
        shared.clone(),
        ControlPeer::ClipboardBridge,
    )));
    let picker_services =
        create_clipboard_picker_service(Arc::new(PickerRpcAdapter(shared.clone())));

    let control_driver = Arc::clone(&control.driver);
    let picker_driver = Arc::clone(&picker.driver);
    let bridge_driver = Arc::clone(&bridge.driver);
    let control_serve = serve_ttrpc_services(Arc::clone(&control.driver), control_services);
    let picker_serve = serve_ttrpc_services(Arc::clone(&picker.driver), picker_services);
    let bridge_serve = serve_ttrpc_services(Arc::clone(&bridge.driver), bridge_services);
    tokio::pin!(control_serve, picker_serve, bridge_serve);
    let requested = tokio::select! {
        changed = shutdown.changed() => changed.is_ok() && *shutdown.borrow(),
        _ = &mut control_serve => false,
        _ = &mut picker_serve => false,
        _ = &mut bridge_serve => false,
    };
    if let Ok(mut services) = shared.0.lock() {
        if requested {
            services.shutdown();
        } else {
            services.session_unavailable();
        }
        drain_diagnostics(&mut services);
    }
    let reason = if requested {
        CloseReason::Normal
    } else {
        CloseReason::SessionLost
    };
    let _ = tokio::join!(
        control_driver.close(reason, Remediation::None),
        picker_driver.close(reason, Remediation::None),
        bridge_driver.close(reason, Remediation::None),
    );
    Ok(())
}

#[derive(Clone)]
struct SharedServices(Arc<Mutex<ClipboardServices>>);

impl SharedServices {
    fn invoke<T>(
        &self,
        operation: impl FnOnce(&mut ClipboardServices) -> Result<T, ClipboardServiceError>,
    ) -> ttrpc::Result<T> {
        let mut services = self.0.lock().map_err(|_| rpc_internal())?;
        let result = operation(&mut services).map_err(map_service_error);
        drain_diagnostics(&mut services);
        result
    }
}

fn drain_diagnostics(services: &mut ClipboardServices) {
    if let Ok(events) = services.drain_audit(DIAGNOSTIC_DRAIN_BATCH) {
        for event in events {
            if let Ok(encoded) = serde_json::to_string(&event) {
                log::info!("d2b-clipd: audit_event {encoded}");
            }
        }
    }
    if let Ok((events, dropped)) = services.drain_metrics(DIAGNOSTIC_DRAIN_BATCH) {
        if dropped > 0 {
            log::warn!("d2b-clipd: metric=dropped_diagnostic count={dropped}");
        }
        for event in events {
            if let Ok(encoded) = serde_json::to_string(&event) {
                log::debug!("d2b-clipd: metric_event {encoded}");
            }
        }
    }
}

struct ClipboardRpcAdapter {
    services: SharedServices,
    peer: ControlPeer,
}

impl ClipboardRpcAdapter {
    fn new(services: SharedServices, peer: ControlPeer) -> Self {
        Self { services, peer }
    }

    fn invoke(
        &self,
        request: ServiceRequest,
        input: impl FnOnce(&ServiceRequest) -> Result<ControlInput, ()>,
    ) -> ttrpc::Result<ServiceResponse> {
        let call = admitted_call(&request)?;
        let input = input(&request).map_err(|()| rpc_invalid())?;
        let response = self
            .services
            .invoke(|services| services.handle_control(self.peer, call, input, unix_millis()))?;
        Ok(control_response(response))
    }
}

#[async_trait]
impl ClipboardRpc for ClipboardRpcAdapter {
    async fn offer(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        _: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        Err(rpc_invalid())
    }

    async fn inspect_offer(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.invoke(request, |request| {
            opaque_resource(request).map(|offer_id| ControlInput::InspectOffer { offer_id })
        })
    }

    async fn accept_transfer(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.invoke(request, |request| {
            opaque_resource(request).map(|offer_id| ControlInput::AcceptTransfer { offer_id })
        })
    }

    async fn complete_transfer(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.invoke(request, |request| {
            opaque_resource(request).map(|offer_id| ControlInput::CompleteTransfer { offer_id })
        })
    }

    async fn cancel_transfer(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.invoke(request, |request| {
            opaque_resource(request).map(|offer_id| ControlInput::CancelTransfer { offer_id })
        })
    }

    async fn bridge_ready(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.invoke(request, |_| Ok(ControlInput::BridgeReady))
    }

    async fn cancel(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: CancelRequest,
    ) -> ttrpc::Result<CancelResponse> {
        validate_cancel(&request)?;
        Ok(cancelled_response())
    }
}

struct PickerRpcAdapter(SharedServices);

#[async_trait]
impl ClipboardPickerRpc for PickerRpcAdapter {
    async fn list_offers(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        let call = picker_call(&request)?;
        let page_size = usize::try_from(request.page_size).map_err(|_| rpc_invalid())?;
        let query = OfferQuery::new(host_scope(&request)?, page_size).map_err(|_| rpc_invalid())?;
        let response = self.0.invoke(|services| {
            services.handle_picker(
                &call,
                projection(&request),
                PickerOperation::List(query),
                unix_millis(),
            )
        })?;
        Ok(picker_response(response))
    }

    async fn select_offer(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        let call = picker_call(&request)?;
        let selection = OfferSelection::new(
            &request.operation_id,
            &request.resource_id,
            host_scope(&request)?,
        )
        .map_err(|_| rpc_invalid())?;
        let response = self.0.invoke(|services| {
            services.handle_picker(
                &call,
                projection(&request),
                PickerOperation::Select(selection),
                unix_millis(),
            )
        })?;
        Ok(picker_response(response))
    }

    async fn cancel_selection(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        let call = picker_call(&request)?;
        let cancel = CancelSelection {
            selection_id: OpaquePickerId::parse(request.resource_id.clone())
                .map_err(|_| rpc_invalid())?,
            destination: host_scope(&request)?,
        };
        let response = self.0.invoke(|services| {
            services.handle_picker(
                &call,
                projection(&request),
                PickerOperation::CancelSelection(cancel),
                unix_millis(),
            )
        })?;
        Ok(picker_response(response))
    }

    async fn cancel(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: CancelRequest,
    ) -> ttrpc::Result<CancelResponse> {
        validate_cancel(&request)?;
        Ok(cancelled_response())
    }
}

fn admitted_call(request: &ServiceRequest) -> ttrpc::Result<AdmittedCall> {
    let metadata = request.metadata.as_ref().ok_or_else(rpc_invalid)?;
    AdmittedCall::new(
        fixed_id(&metadata.request_id)?,
        (!metadata.idempotency_key.is_empty()).then_some(metadata.idempotency_key.as_slice()),
        metadata.session_generation,
        metadata.issued_at_unix_ms,
        metadata.expires_at_unix_ms,
    )
    .map_err(|_| rpc_invalid())
}

fn picker_call(request: &ServiceRequest) -> ttrpc::Result<PickerCall> {
    let metadata = request.metadata.as_ref().ok_or_else(rpc_invalid)?;
    PickerCall::new(
        fixed_id(&metadata.request_id)?,
        (!metadata.idempotency_key.is_empty()).then_some(metadata.idempotency_key.as_slice()),
        metadata.session_generation,
        metadata.issued_at_unix_ms,
        metadata.expires_at_unix_ms,
    )
    .map_err(|_| rpc_invalid())
}

fn fixed_id(bytes: &[u8]) -> ttrpc::Result<[u8; 16]> {
    bytes.try_into().map_err(|_| rpc_invalid())
}

fn opaque_resource(request: &ServiceRequest) -> Result<String, ()> {
    let value = &request.resource_id;
    let valid = !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    valid.then(|| value.clone()).ok_or(())
}

fn host_scope(request: &ServiceRequest) -> ttrpc::Result<ClipboardTarget> {
    let scope = request.scope.as_ref().ok_or_else(rpc_invalid)?;
    if scope.realm_id == "host"
        && scope.workload_id.is_empty()
        && scope.provider_id.is_empty()
        && scope.role_id.is_empty()
    {
        Ok(ClipboardTarget::Host)
    } else {
        Err(rpc_invalid())
    }
}

fn projection(request: &ServiceRequest) -> PickerProjectionBounds {
    PickerProjectionBounds {
        encoded_bytes: 1,
        offer_count: request.page_size.max(1) as usize,
        thumbnail_bytes: 0,
        attachment_count: request.attachment_indexes.len(),
    }
}

fn control_response(response: ControlResponse) -> ServiceResponse {
    let mut wire = ServiceResponse::new();
    wire.outcome = match response.outcome {
        ControlOutcome::Succeeded | ControlOutcome::AlreadyApplied => Outcome::OUTCOME_SUCCEEDED,
        ControlOutcome::Denied => Outcome::OUTCOME_DENIED,
        ControlOutcome::Cancelled => Outcome::OUTCOME_CANCELLED,
        ControlOutcome::DeadlineExpired | ControlOutcome::Unavailable => Outcome::OUTCOME_FAILED,
    }
    .into();
    wire.resource_handle = response.offer_id.unwrap_or_default();
    wire
}

fn picker_response(response: PickerResponse) -> ServiceResponse {
    let mut wire = ServiceResponse::new();
    wire.outcome = match response {
        PickerResponse::Offers(_) | PickerResponse::Selected(_) => Outcome::OUTCOME_SUCCEEDED,
        PickerResponse::Cancelled => Outcome::OUTCOME_CANCELLED,
    }
    .into();
    wire
}

fn validate_cancel(request: &CancelRequest) -> ttrpc::Result<()> {
    if request.session_generation == 0 || request.request_id.len() != 16 {
        Err(rpc_invalid())
    } else {
        Ok(())
    }
}

fn cancelled_response() -> CancelResponse {
    let mut response = CancelResponse::new();
    response.outcome = CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED.into();
    response
}

fn map_service_error(error: ClipboardServiceError) -> ttrpc::Error {
    match error {
        ClipboardServiceError::Unavailable => rpc_status(ttrpc::Code::UNAVAILABLE, "unavailable"),
        ClipboardServiceError::TransferCapacityExhausted
        | ClipboardServiceError::Picker(
            crate::services::picker::PickerServiceError::ResourceExhausted,
        ) => rpc_status(ttrpc::Code::RESOURCE_EXHAUSTED, "resource-exhausted"),
        ClipboardServiceError::TransferNotFound
        | ClipboardServiceError::Picker(crate::services::picker::PickerServiceError::NotFound) => {
            rpc_status(ttrpc::Code::NOT_FOUND, "not-found")
        }
        ClipboardServiceError::TransferNotAuthorized
        | ClipboardServiceError::PickerConfirmationRequired
        | ClipboardServiceError::PickerOfferPolicy(_)
        | ClipboardServiceError::Control(
            crate::services::control::ControlError::Policy(_)
            | crate::services::control::ControlError::Unauthorized,
        ) => rpc_status(ttrpc::Code::PERMISSION_DENIED, "policy-denied"),
        _ => rpc_invalid(),
    }
}

fn rpc_invalid() -> ttrpc::Error {
    rpc_status(ttrpc::Code::INVALID_ARGUMENT, "invalid-request")
}

fn rpc_internal() -> ttrpc::Error {
    rpc_status(ttrpc::Code::INTERNAL, "internal")
}

fn rpc_status(code: ttrpc::Code, message: &str) -> ttrpc::Error {
    ttrpc::Error::RpcStatus(ttrpc::get_status(code, message.to_owned()))
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            elapsed.as_millis().min(u128::from(u64::MAX)) as u64
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_names_and_endpoint_contracts_are_exact() {
        assert_eq!(
            Endpoint::ALL.map(Endpoint::listener_name),
            ACTIVATED_LISTENER_NAMES
        );
        for endpoint in Endpoint::ALL {
            let policy = endpoint_policy(endpoint, 7, 1000, 100).unwrap();
            assert_eq!(policy.purpose, endpoint.purpose());
            assert_eq!(policy.service, endpoint.service());
            assert_eq!(policy.reconnect_generation, 7);
            assert_eq!(
                policy.transport_binding.transport,
                TransportClass::UnixSeqpacket
            );
            assert_eq!(policy.transport_binding.locality, Locality::HostLocal);
        }
        assert_eq!(
            endpoint_policy(Endpoint::Bridge, 7, 1000, 100)
                .unwrap()
                .attachment_policy
                .max_per_session,
            1
        );
    }

    #[test]
    fn channel_binding_is_closed_over_endpoint_and_peer_credentials() {
        let base = endpoint_channel_binding(Endpoint::Control, 1000, 100);
        assert_ne!(base, endpoint_channel_binding(Endpoint::Bridge, 1000, 100));
        assert_ne!(base, endpoint_channel_binding(Endpoint::Control, 1001, 100));
        assert_ne!(base, endpoint_channel_binding(Endpoint::Control, 1000, 101));
        assert_ne!(base, [0; 32]);
    }

    #[test]
    fn wire_adapters_reject_payload_substitution_and_non_host_picker_scope() {
        let mut request = ServiceRequest::new();
        request.resource_id = "offer-1".to_owned();
        assert_eq!(opaque_resource(&request), Ok("offer-1".to_owned()));
        request.resource_id = "payload/value".to_owned();
        assert_eq!(opaque_resource(&request), Err(()));
        assert!(host_scope(&request).is_err());
    }

    #[test]
    fn daemon_errors_are_redacted_and_activation_is_config_exit() {
        assert_eq!(
            DaemonError::Activation.to_string(),
            "clipboard-activation-invalid"
        );
        assert_eq!(DaemonError::Activation.exit_code(), 78);
        assert_eq!(DaemonError::Session.exit_code(), 1);
    }
}
