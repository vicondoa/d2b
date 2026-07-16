//! Authenticated security-key ComponentSession composition.

pub const SERVICE_PACKAGE: &str = "d2b.security-key.v2";
pub const ENDPOINT_PURPOSE: &str = "security-key";
pub const ENDPOINT_ROLE: &str = "security-key-frontend";

use std::{collections::VecDeque, fmt, sync::Arc, time::Instant};

use d2b_contracts::{
    v2_component_session::{
        AttachmentPolicy, AttachmentPolicyKind, EndpointPolicy, EndpointPurpose, EndpointRole,
        IdentityEvidenceRequirement, LimitProfile, Locality, NoiseProfile, PurposeClass,
        ServicePackage, TransportBinding, TransportClass,
    },
    v2_identity::ProviderType,
    v2_provider::ProviderHealthState,
    v2_services::{SERVICE_INVENTORY, service_schema_fingerprint},
};
use d2b_provider_observability_local::{
    ClosedMetricLabels, LocalObservationRecord, MetricLabel, OperationLabel, OutcomeLabel,
    ProjectionKind,
};
use d2b_session::{
    ComponentSessionDriver, HandshakeCredentials, SessionEngine, StreamEvent, StreamId,
};

use crate::{framing::CTAPHID_REPORT_LEN, vsock::VsockTransport};

const REPORT_STREAM_ID: u16 = 1;
const REPORT_STREAM_CREDIT: u32 = 64 * 64;
const MAX_FRONTEND_OBSERVATIONS: usize = 64;

#[derive(Clone)]
pub struct SessionConfig {
    channel_binding: [u8; 32],
    reconnect_generation: u64,
}

impl fmt::Debug for SessionConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionConfig")
            .field("channel_binding", &"<redacted>")
            .field("reconnect_generation", &"<redacted>")
            .finish()
    }
}

impl SessionConfig {
    pub fn new(channel_binding: [u8; 32], reconnect_generation: u64) -> Result<Self, &'static str> {
        if channel_binding == [0; 32] {
            return Err("invalid-channel-binding");
        }
        if reconnect_generation == 0 {
            return Err("invalid-reconnect-generation");
        }
        Ok(Self {
            channel_binding,
            reconnect_generation,
        })
    }

    pub fn from_env() -> Result<Self, &'static str> {
        let binding =
            std::env::var("D2B_SK_CHANNEL_BINDING_HEX").map_err(|_| "missing-channel-binding")?;
        let generation = std::env::var("D2B_SK_RECONNECT_GENERATION")
            .map_err(|_| "missing-reconnect-generation")?
            .parse::<u64>()
            .map_err(|_| "invalid-reconnect-generation")?;
        Self::new(decode_binding(&binding)?, generation)
    }

    fn policy(&self) -> EndpointPolicy {
        let service = SERVICE_INVENTORY
            .iter()
            .find(|service| service.package == SERVICE_PACKAGE)
            .expect("frozen security-key service exists");
        EndpointPolicy {
            purpose: EndpointPurpose::SecurityKey,
            purpose_class: PurposeClass::Local,
            initiator_role: EndpointRole::SecurityKeyFrontend,
            responder_role: EndpointRole::SecurityKeyController,
            service: ServicePackage::SecurityKeyV2,
            schema_fingerprint: service_schema_fingerprint(service),
            noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
            limits: LimitProfile::local_default(),
            transport_binding: TransportBinding {
                transport: TransportClass::NativeVsock,
                locality: Locality::GuestLocal,
                channel_binding: self.channel_binding,
                identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
            },
            reconnect_generation: self.reconnect_generation,
            attachment_policy: AttachmentPolicy {
                kind: AttachmentPolicyKind::Disabled,
                max_per_packet: 0,
                max_per_request: 0,
                max_per_operation: 0,
                max_per_session: 0,
                credentials_allowed: false,
            },
        }
    }
}

fn decode_binding(value: &str) -> Result<[u8; 32], &'static str> {
    if value.len() != 64 || !value.is_ascii() {
        return Err("invalid-channel-binding");
    }
    let mut decoded = [0; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        decoded[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Ok(decoded)
}

fn hex_nibble(value: u8) -> Result<u8, &'static str> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err("invalid-channel-binding"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportStreamError {
    Authentication,
    Stream,
    InvalidReport,
}

pub struct ReportStream {
    driver: Arc<dyn ComponentSessionDriver>,
    stream: StreamId,
}

impl fmt::Debug for ReportStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ReportStream([authenticated])")
    }
}

impl ReportStream {
    pub async fn establish(
        transport: VsockTransport,
        config: SessionConfig,
    ) -> Result<Self, ReportStreamError> {
        let engine = SessionEngine::establish_initiator(
            transport,
            config.policy(),
            HandshakeCredentials::Nn,
            Instant::now(),
        )
        .await
        .map_err(|_| ReportStreamError::Authentication)?;
        let driver: Arc<dyn ComponentSessionDriver> = Arc::new(engine.into_driver());
        let stream = StreamId::new(REPORT_STREAM_ID).map_err(|_| ReportStreamError::Stream)?;
        driver
            .open_named_stream(stream, REPORT_STREAM_CREDIT, REPORT_STREAM_CREDIT)
            .await
            .map_err(|_| ReportStreamError::Stream)?;
        Ok(Self { driver, stream })
    }

    pub async fn send_report(
        &self,
        report: &[u8; CTAPHID_REPORT_LEN],
    ) -> Result<(), ReportStreamError> {
        self.driver
            .send_named_stream(self.stream, report.to_vec())
            .await
            .map_err(|_| ReportStreamError::Stream)
    }

    pub async fn receive_report(&mut self) -> Result<[u8; CTAPHID_REPORT_LEN], ReportStreamError> {
        match self
            .driver
            .receive_named_stream()
            .await
            .map_err(|_| ReportStreamError::Stream)?
        {
            StreamEvent::Data { stream, bytes } if stream == self.stream => {
                let report: [u8; CTAPHID_REPORT_LEN] = bytes
                    .try_into()
                    .map_err(|_| ReportStreamError::InvalidReport)?;
                self.driver
                    .grant_named_stream_credit(self.stream, CTAPHID_REPORT_LEN as u32)
                    .await
                    .map_err(|_| ReportStreamError::Stream)?;
                Ok(report)
            }
            StreamEvent::RemoteClosed { stream } | StreamEvent::Reset { stream }
                if stream == self.stream =>
            {
                Err(ReportStreamError::Stream)
            }
            _ => Err(ReportStreamError::InvalidReport),
        }
    }

    pub async fn drive_keepalive(&self) -> Result<(), ReportStreamError> {
        self.driver
            .drive_keepalive(Instant::now())
            .await
            .map_err(|_| ReportStreamError::Stream)
    }

    pub async fn reset(&self) -> Result<(), ReportStreamError> {
        self.driver
            .reset_named_stream(self.stream)
            .await
            .map_err(|_| ReportStreamError::Stream)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryOutcome {
    Success,
    Denied,
    Unavailable,
}

#[derive(Default)]
pub struct FrontendObservability {
    records: VecDeque<LocalObservationRecord>,
}

impl FrontendObservability {
    pub fn record(&mut self, observed_at_unix_ms: u64, outcome: TelemetryOutcome) {
        let (health, outcome) = match outcome {
            TelemetryOutcome::Success => (ProviderHealthState::Healthy, OutcomeLabel::Success),
            TelemetryOutcome::Denied => (ProviderHealthState::Healthy, OutcomeLabel::Denied),
            TelemetryOutcome::Unavailable => {
                (ProviderHealthState::Unavailable, OutcomeLabel::Unavailable)
            }
        };
        let labels = ClosedMetricLabels::new(
            ProviderType::Device,
            health,
            MetricLabel::OperationTotal,
            OperationLabel::Attach,
            outcome,
        );
        if let Ok(record) =
            LocalObservationRecord::new(observed_at_unix_ms, ProjectionKind::Metrics, labels, 1)
        {
            if self.records.len() == MAX_FRONTEND_OBSERVATIONS {
                self.records.pop_front();
            }
            self.records.push_back(record);
        }
    }

    pub fn retained(&self) -> usize {
        self.records.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_policy_is_exact_and_attachment_free() {
        let policy = SessionConfig::new([0x22; 32], 7).unwrap().policy();
        assert_eq!(policy.purpose, EndpointPurpose::SecurityKey);
        assert_eq!(policy.initiator_role, EndpointRole::SecurityKeyFrontend);
        assert_eq!(policy.responder_role, EndpointRole::SecurityKeyController);
        assert_eq!(policy.service, ServicePackage::SecurityKeyV2);
        assert_eq!(
            policy.transport_binding.transport,
            TransportClass::NativeVsock
        );
        assert_eq!(policy.transport_binding.locality, Locality::GuestLocal);
        assert_eq!(
            policy.attachment_policy.kind,
            AttachmentPolicyKind::Disabled
        );
        assert_ne!(policy.schema_fingerprint, [0; 32]);
        policy
            .attachment_policy
            .validate(policy.transport_binding.transport)
            .unwrap();
    }

    #[test]
    fn channel_binding_is_strict_lowercase_hex() {
        assert_eq!(decode_binding(&"01".repeat(32)).unwrap(), [1; 32]);
        for invalid in [
            "",
            "0",
            &"GG".repeat(32),
            &"AA".repeat(32),
            &"00".repeat(31),
        ] {
            assert_eq!(decode_binding(invalid), Err("invalid-channel-binding"));
        }
        assert!(SessionConfig::new([0; 32], 1).is_err());
        assert!(SessionConfig::new([1; 32], 0).is_err());
    }

    #[test]
    fn observations_are_bounded_and_closed_labelled() {
        let mut observations = FrontendObservability::default();
        for index in 0..100 {
            observations.record(index, TelemetryOutcome::Success);
        }
        assert_eq!(observations.retained(), MAX_FRONTEND_OBSERVATIONS);
    }
}
