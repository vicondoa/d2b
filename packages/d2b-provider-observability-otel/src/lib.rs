//! Optional, non-bootstrap observability Provider support.

#![forbid(unsafe_code)]

pub mod agent;
pub mod config;
pub mod emitter_socket;
pub mod ingress_policy;
pub mod metric_policy;
pub mod metrics;

pub const PROVIDER_NAME: &str = "observability-otel";
pub const PROVIDER_REF: &str = "Provider/observability-otel";
pub const PROVIDER_API_MAJOR: u16 = 1;

pub use agent::{
    ProviderAgentAuditEvent, ProviderAgentAuditOutcome, ProviderAgentError, ProviderAgentProcess,
};
pub use config::{ConfigError, ProviderConfig};
pub use emitter_socket::{EmitterSocket, ReceiverReadiness};
pub use ingress_policy::{
    Ingress, IngressErrorClass, IngressOutcome, IngressPolicyGate, MetricFrame, MetricPoint,
};
pub use metric_policy::{
    IdentityCanaries, LabelDescriptor, MetricDescriptor, MetricPolicyError, ResourceAttributeError,
    allowed_values, label, validate_data_point, validate_descriptor, validate_label_key,
    validate_resource_attributes,
};
