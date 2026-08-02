//! integration-target: container
//!
//! The executable scenario belongs to the container lane once the production
//! ComponentSession adapter and Provider supervisor are wired.

/// Production surfaces required by the session scenario.
pub const REQUIRED_SURFACES: &[&str] = &[
    "authenticated-component-session",
    "provider-supervisor",
    "provider-agent-dispatch",
];

/// Events that the scenario verifies without retaining session payloads.
pub const EXPECTED_EVENTS: &[&str] = &["session-connect", "process-effect"];
