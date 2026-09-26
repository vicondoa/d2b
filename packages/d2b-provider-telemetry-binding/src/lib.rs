//! The TelemetryBinding resource type's driver, its spec decoder, and its
//! driver declaration.
//!
//! The crate owns the `telemetry.d2bus.org.TelemetryBinding` type's complete
//! resource knowledge: the relationship admission (the owner must name the
//! telemetry Provider and its Service/target rows must exist and not be
//! deleting), the provider-declared child set materialized as owned manager
//! rows, the preserved endpoint-first / process-last retirement of children
//! the desired set no longer derives, the driver's validate, recover,
//! reconcile, and delete verbs, and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! The child shapes are the Serving Provider's own declaration: the crate
//! answers with `TelemetryBindingController::child_resources` from
//! `d2b-provider-observability-otel`, so the closed child set cannot drift
//! from the collector/forwarder rows that Provider commits, and Core keeps
//! owning the child body (`materialize_child_create_payload`). KTD13: the
//! telemetry collector and forwarder are Process resources owned by the
//! Binding, so their launch belongs to the Process controller; this driver
//! never spawns a child process.
//!
//! # Contract flag (KTD3 fit; preserved from the daemon-side conversion)
//!
//! The preserved phase predicate reads its children's observed status.
//! [`ResourceContext`] offers `ensure` / `get` / `delete` / `watch` /
//! `set_status` / `requeue_after` / `children`, and none of them returns a
//! dependency's runtime status, so a converged owner reports the fail-closed
//! `Degraded` projection instead of claiming `Ready`.

#![deny(missing_docs)]

mod driver;

pub use driver::{
    TELEMETRY_BINDING_COLLECTOR_CREATION, TELEMETRY_BINDING_CREATIONS,
    TELEMETRY_BINDING_ENDPOINT_CREATION, TELEMETRY_BINDING_RESYNC, TELEMETRY_BINDING_TYPE,
    TelemetryBindingDriver, TelemetryBindingDriverError, TelemetryBindingDriverFactory,
    TelemetryBindingPhase, TelemetryBindingStatus, telemetry_binding_descriptor,
    telemetry_binding_spec_decoder,
};
