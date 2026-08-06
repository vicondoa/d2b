//! integration-target: container
//!
//! The executable scenario belongs to the container lane once a real
//! Binding-owned collector and OTLP exporter can be started.

/// Every ingress must use the same structural admission gate.
pub const INGRESSES: &[&str] = &["emitter_unix", "otlp_unix", "otlp_vsock", "import_stream"];

/// Production surfaces required by the pipeline scenario.
pub const REQUIRED_SURFACES: &[&str] = &[
    "binding-owned-collector",
    "component-session-stream",
    "otlp-exporter",
    "signoz-authority",
];
