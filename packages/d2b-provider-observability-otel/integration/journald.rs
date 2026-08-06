//! integration-target: host-integration
//!
//! The executable scenario belongs to the host lane once a Provider-owned
//! collector is launched through the existing observability component.

/// Production surfaces required by the journald scenario.
pub const REQUIRED_SURFACES: &[&str] = &[
    "observability-component",
    "journald-receiver",
    "provider-redaction-filter",
];

/// Fields that must never be forwarded by the journald pipeline.
pub const DROPPED_FIELDS: &[&str] = &["MESSAGE", "_CMDLINE", "_EXE", "INVOCATION_ID"];
