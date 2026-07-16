//! Wayland control boundary over the frozen v2 service and session contracts.
//!
//! The compositor connection remains the Wayland data plane. Control callers
//! may supply only ComponentSession-authenticated descriptor projections; this
//! module deliberately has no pathname, argv, or newline-JSON compatibility
//! variant.

pub const SERVICE_PACKAGE: &str = "d2b.wayland.v2";
pub const ENDPOINT_PURPOSE: &str = "wayland-proxy";
pub const ENDPOINT_ROLE: &str = "wayland-proxy";

pub const OPEN_DISPLAY_METHOD_ID: u32 = 2_774_385_992;
pub const INSPECT_DISPLAY_METHOD_ID: u32 = 2_338_854_252;
pub const CLOSE_DISPLAY_METHOD_ID: u32 = 2_944_622_932;
pub const BRIDGE_READY_METHOD_ID: u32 = 1_354_333_954;
pub const CANCEL_METHOD_ID: u32 = 2_668_421_128;

const MAX_OPAQUE_ID_BYTES: usize = 64;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OpaqueId(String);

impl OpaqueId {
    pub fn parse(value: impl Into<String>) -> Result<Self, ControlError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= MAX_OPAQUE_ID_BYTES
            && value.as_bytes()[0].is_ascii_lowercase()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        valid.then_some(Self(value)).ok_or(ControlError::InvalidId)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for OpaqueId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OpaqueId(<redacted>)")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SessionIdentity {
    pub realm_id: OpaqueId,
    pub workload_id: OpaqueId,
    pub provider_id: OpaqueId,
    pub role_id: OpaqueId,
}

impl std::fmt::Debug for SessionIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SessionIdentity(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    UnixSeqpacket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationOwner {
    ComponentSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportContract {
    pub kind: TransportKind,
    pub packet_atomic: bool,
    pub attachments_enabled: bool,
    pub authentication: AuthenticationOwner,
}

impl TransportContract {
    pub const COMPONENT_SESSION_LOCAL: Self = Self {
        kind: TransportKind::UnixSeqpacket,
        packet_atomic: true,
        attachments_enabled: true,
        authentication: AuthenticationOwner::ComponentSession,
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContract {
    pub identity: SessionIdentity,
    pub generation: u64,
    pub transport: TransportContract,
}

impl SessionContract {
    pub fn validate(&self) -> Result<(), ControlError> {
        if self.generation == 0 || self.transport != TransportContract::COMPONENT_SESSION_LOCAL {
            return Err(ControlError::InvalidSession);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorObject {
    WaylandSocket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorKind {
    FileDescriptor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorAccess {
    ReadWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorPurpose {
    Wayland,
    Listener,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorCreditClass {
    Packet,
    Request,
    Operation,
    Session,
    Process,
    Host,
}

pub const DESCRIPTOR_CREDIT_CLASSES: [DescriptorCreditClass; 6] = [
    DescriptorCreditClass::Packet,
    DescriptorCreditClass::Request,
    DescriptorCreditClass::Operation,
    DescriptorCreditClass::Session,
    DescriptorCreditClass::Process,
    DescriptorCreditClass::Host,
];

#[derive(Clone, PartialEq, Eq)]
pub struct AuthenticatedDescriptor {
    pub index: u16,
    pub kind: DescriptorKind,
    pub object: DescriptorObject,
    pub access: DescriptorAccess,
    pub purpose: DescriptorPurpose,
    pub service_package: &'static str,
    pub method_id: u32,
    pub request_id: [u8; 16],
    pub operation_id: [u8; 16],
    pub packet_sequence: u64,
    pub reconnect_generation: u64,
    pub cloexec_required: bool,
    pub duplicate_object_allowed: bool,
    pub credit_classes: [DescriptorCreditClass; 6],
}

impl std::fmt::Debug for AuthenticatedDescriptor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthenticatedDescriptor")
            .field("index", &self.index)
            .field("object", &self.object)
            .field("purpose", &self.purpose)
            .finish_non_exhaustive()
    }
}

impl AuthenticatedDescriptor {
    fn validate(
        &self,
        expected_index: u16,
        expected_purpose: DescriptorPurpose,
        request: &ControlRequest,
    ) -> Result<(), ControlError> {
        if self.index != expected_index
            || self.kind != DescriptorKind::FileDescriptor
            || self.object != DescriptorObject::WaylandSocket
            || self.access != DescriptorAccess::ReadWrite
            || self.purpose != expected_purpose
            || self.service_package != SERVICE_PACKAGE
            || self.method_id != OPEN_DISPLAY_METHOD_ID
            || self.request_id != request.request_id
            || self.operation_id != request.operation_id
            || self.packet_sequence == 0
            || self.reconnect_generation != request.session_generation
            || !self.cloexec_required
            || self.duplicate_object_allowed
            || self.credit_classes != DESCRIPTOR_CREDIT_CLASSES
        {
            return Err(ControlError::DescriptorMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlMethod {
    OpenDisplay,
    InspectDisplay,
    CloseDisplay,
    BridgeReady,
    Cancel,
}

impl ControlMethod {
    pub const fn method_id(self) -> u32 {
        match self {
            Self::OpenDisplay => OPEN_DISPLAY_METHOD_ID,
            Self::InspectDisplay => INSPECT_DISPLAY_METHOD_ID,
            Self::CloseDisplay => CLOSE_DISPLAY_METHOD_ID,
            Self::BridgeReady => BRIDGE_READY_METHOD_ID,
            Self::Cancel => CANCEL_METHOD_ID,
        }
    }

    pub const fn mutating(self) -> bool {
        matches!(self, Self::OpenDisplay | Self::CloseDisplay)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ControlRequest {
    pub method: ControlMethod,
    pub request_id: [u8; 16],
    pub operation_id: [u8; 16],
    pub session_generation: u64,
    pub resource_id: OpaqueId,
    pub descriptors: Vec<AuthenticatedDescriptor>,
}

impl std::fmt::Debug for ControlRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ControlRequest")
            .field("method", &self.method)
            .field("descriptor_count", &self.descriptors.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayHealth {
    Ready,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayProviderBinding {
    pub display_endpoint_id: OpaqueId,
    pub cross_domain_endpoint_id: OpaqueId,
    pub waypipe_endpoint_id: OpaqueId,
    pub proxy_endpoint_id: OpaqueId,
    pub resource_generation: u64,
    pub wayland: bool,
    pub cross_domain: bool,
    pub waypipe: bool,
    pub proxy: bool,
    pub authorization: bool,
}

impl DisplayProviderBinding {
    pub fn validate(&self) -> Result<(), ControlError> {
        let endpoints = [
            &self.display_endpoint_id,
            &self.cross_domain_endpoint_id,
            &self.waypipe_endpoint_id,
            &self.proxy_endpoint_id,
        ];
        if self.resource_generation == 0
            || !(self.wayland
                && self.cross_domain
                && self.waypipe
                && self.proxy
                && self.authorization)
            || endpoints
                .iter()
                .enumerate()
                .any(|(index, endpoint)| endpoints[index + 1..].contains(endpoint))
        {
            return Err(ControlError::InvalidDisplayBinding);
        }
        Ok(())
    }
}

pub trait DisplayProviderPort {
    fn open(
        &mut self,
        identity: &SessionIdentity,
        binding: &DisplayProviderBinding,
        request: &ControlRequest,
    ) -> Result<OpaqueId, ControlError>;

    fn inspect(
        &self,
        identity: &SessionIdentity,
        binding: &DisplayProviderBinding,
        resource: &OpaqueId,
    ) -> Result<DisplayHealth, ControlError>;

    fn close(
        &mut self,
        identity: &SessionIdentity,
        binding: &DisplayProviderBinding,
        request: &ControlRequest,
    ) -> Result<(), ControlError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationOperation {
    Open,
    Inspect,
    Close,
    Ready,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationOutcome {
    Success,
    Denied,
    Unavailable,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub operation: ObservationOperation,
    pub outcome: ObservationOutcome,
}

pub trait ObservationSink {
    fn record(&mut self, observation: Observation);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlResponse {
    Opened { resource_handle: OpaqueId },
    Inspected { health: DisplayHealth },
    Closed,
    Ready,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ControlError {
    #[error("invalid opaque control identifier")]
    InvalidId,
    #[error("component session contract mismatch")]
    InvalidSession,
    #[error("display provider binding mismatch")]
    InvalidDisplayBinding,
    #[error("descriptor contract mismatch")]
    DescriptorMismatch,
    #[error("control request does not match the frozen service contract")]
    RequestMismatch,
    #[error("display provider unavailable")]
    ProviderUnavailable,
}

pub struct WaylandControlService<P, O> {
    session: SessionContract,
    display: DisplayProviderBinding,
    provider: P,
    observations: O,
}

impl<P, O> WaylandControlService<P, O>
where
    P: DisplayProviderPort,
    O: ObservationSink,
{
    pub fn new(
        session: SessionContract,
        display: DisplayProviderBinding,
        provider: P,
        observations: O,
    ) -> Result<Self, ControlError> {
        session.validate()?;
        display.validate()?;
        Ok(Self {
            session,
            display,
            provider,
            observations,
        })
    }

    pub fn dispatch(&mut self, request: ControlRequest) -> Result<ControlResponse, ControlError> {
        if request.session_generation != self.session.generation
            || request.request_id == [0; 16]
            || request.operation_id == [0; 16]
        {
            self.observe(request.method, ObservationOutcome::Denied);
            return Err(ControlError::RequestMismatch);
        }

        let response: Result<ControlResponse, ControlError> = (|| {
            Ok(match request.method {
                ControlMethod::OpenDisplay => {
                    self.validate_open_descriptors(&request)?;
                    let handle =
                        self.provider
                            .open(&self.session.identity, &self.display, &request)?;
                    ControlResponse::Opened {
                        resource_handle: handle,
                    }
                }
                ControlMethod::InspectDisplay => {
                    require_no_descriptors(&request)?;
                    let health = self.provider.inspect(
                        &self.session.identity,
                        &self.display,
                        &request.resource_id,
                    )?;
                    ControlResponse::Inspected { health }
                }
                ControlMethod::CloseDisplay => {
                    require_no_descriptors(&request)?;
                    self.provider
                        .close(&self.session.identity, &self.display, &request)?;
                    ControlResponse::Closed
                }
                ControlMethod::BridgeReady => {
                    require_no_descriptors(&request)?;
                    ControlResponse::Ready
                }
                ControlMethod::Cancel => {
                    require_no_descriptors(&request)?;
                    ControlResponse::Cancelled
                }
            })
        })();
        let outcome = match &response {
            Ok(ControlResponse::Cancelled) => ObservationOutcome::Cancelled,
            Ok(_) => ObservationOutcome::Success,
            Err(ControlError::ProviderUnavailable) => ObservationOutcome::Unavailable,
            Err(_) => ObservationOutcome::Denied,
        };
        self.observe(request.method, outcome);
        response
    }

    fn validate_open_descriptors(&self, request: &ControlRequest) -> Result<(), ControlError> {
        let [upstream, listener] = request.descriptors.as_slice() else {
            return Err(ControlError::DescriptorMismatch);
        };
        upstream.validate(0, DescriptorPurpose::Wayland, request)?;
        listener.validate(1, DescriptorPurpose::Listener, request)
    }

    fn observe(&mut self, method: ControlMethod, outcome: ObservationOutcome) {
        let operation = match method {
            ControlMethod::OpenDisplay => ObservationOperation::Open,
            ControlMethod::InspectDisplay => ObservationOperation::Inspect,
            ControlMethod::CloseDisplay => ObservationOperation::Close,
            ControlMethod::BridgeReady => ObservationOperation::Ready,
            ControlMethod::Cancel => ObservationOperation::Cancel,
        };
        self.observations.record(Observation { operation, outcome });
    }
}

fn require_no_descriptors(request: &ControlRequest) -> Result<(), ControlError> {
    if request.descriptors.is_empty() {
        Ok(())
    } else {
        Err(ControlError::DescriptorMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeProvider;

    impl DisplayProviderPort for FakeProvider {
        fn open(
            &mut self,
            _: &SessionIdentity,
            _: &DisplayProviderBinding,
            _: &ControlRequest,
        ) -> Result<OpaqueId, ControlError> {
            OpaqueId::parse("display-handle")
        }

        fn inspect(
            &self,
            _: &SessionIdentity,
            _: &DisplayProviderBinding,
            _: &OpaqueId,
        ) -> Result<DisplayHealth, ControlError> {
            Ok(DisplayHealth::Ready)
        }

        fn close(
            &mut self,
            _: &SessionIdentity,
            _: &DisplayProviderBinding,
            _: &ControlRequest,
        ) -> Result<(), ControlError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct Observations(Vec<Observation>);

    impl ObservationSink for Observations {
        fn record(&mut self, observation: Observation) {
            self.0.push(observation);
        }
    }

    fn id(value: &str) -> OpaqueId {
        OpaqueId::parse(value).unwrap()
    }

    fn service() -> WaylandControlService<FakeProvider, Observations> {
        WaylandControlService::new(
            SessionContract {
                identity: SessionIdentity {
                    realm_id: id("realm"),
                    workload_id: id("workload"),
                    provider_id: id("provider"),
                    role_id: id("role"),
                },
                generation: 7,
                transport: TransportContract::COMPONENT_SESSION_LOCAL,
            },
            DisplayProviderBinding {
                display_endpoint_id: id("display"),
                cross_domain_endpoint_id: id("cross-domain"),
                waypipe_endpoint_id: id("waypipe"),
                proxy_endpoint_id: id("proxy"),
                resource_generation: 9,
                wayland: true,
                cross_domain: true,
                waypipe: true,
                proxy: true,
                authorization: true,
            },
            FakeProvider,
            Observations::default(),
        )
        .unwrap()
    }

    fn open_request() -> ControlRequest {
        let request_id = [1; 16];
        let operation_id = [2; 16];
        let descriptor = |index, purpose| AuthenticatedDescriptor {
            index,
            kind: DescriptorKind::FileDescriptor,
            object: DescriptorObject::WaylandSocket,
            access: DescriptorAccess::ReadWrite,
            purpose,
            service_package: SERVICE_PACKAGE,
            method_id: OPEN_DISPLAY_METHOD_ID,
            request_id,
            operation_id,
            packet_sequence: 1,
            reconnect_generation: 7,
            cloexec_required: true,
            duplicate_object_allowed: false,
            credit_classes: DESCRIPTOR_CREDIT_CLASSES,
        };
        ControlRequest {
            method: ControlMethod::OpenDisplay,
            request_id,
            operation_id,
            session_generation: 7,
            resource_id: id("display"),
            descriptors: vec![
                descriptor(0, DescriptorPurpose::Wayland),
                descriptor(1, DescriptorPurpose::Listener),
            ],
        }
    }

    #[test]
    fn open_accepts_only_exact_component_session_descriptors() {
        assert_eq!(
            service().dispatch(open_request()).unwrap(),
            ControlResponse::Opened {
                resource_handle: id("display-handle")
            }
        );

        for mutate in 0..7 {
            let mut request = open_request();
            match mutate {
                0 => request.descriptors[0].index = 1,
                1 => request.descriptors[0].purpose = DescriptorPurpose::Listener,
                2 => request.descriptors[0].method_id = INSPECT_DISPLAY_METHOD_ID,
                3 => request.descriptors[0].cloexec_required = false,
                4 => request.descriptors[0].duplicate_object_allowed = true,
                5 => {
                    request.descriptors.pop();
                }
                6 => request.descriptors[0].credit_classes.swap(0, 1),
                _ => unreachable!(),
            }
            assert_eq!(
                service().dispatch(request),
                Err(ControlError::DescriptorMismatch)
            );
        }
    }

    #[test]
    fn session_and_display_provider_contracts_fail_closed() {
        let mut session = SessionContract {
            identity: SessionIdentity {
                realm_id: id("realm"),
                workload_id: id("workload"),
                provider_id: id("provider"),
                role_id: id("role"),
            },
            generation: 0,
            transport: TransportContract::COMPONENT_SESSION_LOCAL,
        };
        assert_eq!(session.validate(), Err(ControlError::InvalidSession));
        session.generation = 1;
        session.transport.attachments_enabled = false;
        assert_eq!(session.validate(), Err(ControlError::InvalidSession));

        let binding = DisplayProviderBinding {
            display_endpoint_id: id("same"),
            cross_domain_endpoint_id: id("same"),
            waypipe_endpoint_id: id("waypipe"),
            proxy_endpoint_id: id("proxy"),
            resource_generation: 1,
            wayland: true,
            cross_domain: true,
            waypipe: true,
            proxy: true,
            authorization: true,
        };
        assert_eq!(binding.validate(), Err(ControlError::InvalidDisplayBinding));
    }

    #[test]
    fn non_open_methods_reject_all_descriptors() {
        let mut request = open_request();
        request.method = ControlMethod::BridgeReady;
        assert_eq!(
            service().dispatch(request),
            Err(ControlError::DescriptorMismatch)
        );
    }

    #[test]
    fn rejected_descriptor_is_observed_with_closed_labels() {
        let mut service = service();
        let mut request = open_request();
        request.descriptors[0].credit_classes.swap(0, 1);
        assert_eq!(
            service.dispatch(request),
            Err(ControlError::DescriptorMismatch)
        );
        assert_eq!(
            service.observations.0,
            vec![Observation {
                operation: ObservationOperation::Open,
                outcome: ObservationOutcome::Denied,
            }]
        );
    }

    #[test]
    fn opaque_ids_match_the_frozen_identifier_grammar() {
        assert_eq!(OpaqueId::parse("-display"), Err(ControlError::InvalidId));
        assert_eq!(OpaqueId::parse("1display"), Err(ControlError::InvalidId));
        assert_eq!(OpaqueId::parse("display").unwrap().as_str(), "display");
    }
}
