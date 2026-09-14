//! The TelemetryService resource type's driver, its spec decoder, and its
//! driver declaration.
//!
//! The crate owns the `telemetry.d2bus.org.TelemetryService` type's complete
//! resource knowledge: the observed-phase projection the old reconciler
//! published, the admission rule over the spec's declared `serviceRole`, the
//! driver's validate, recover, reconcile, and delete verbs, and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! The Service realizes nothing on a target: its observed evidence is the
//! durable ingest-`Endpoint` rows reconcile re-reads, and it owns no child, so
//! the crate declares no effect port and no child creation. The telemetry
//! Serving Provider stays the authority for what a Service *means* - the
//! `telemetry.d2bus.org.TelemetryService` contract this crate keys on lives in
//! `d2b-contracts-provider`, beside the sibling Binding type.
//!
//! # Contract flag (KTD3 fit; preserved from the daemon-side conversion)
//!
//! The preserved phase predicate reads *another resource's* observed state:
//! the ingest Endpoints' status. [`ResourceContext`] offers
//! `ensure` / `get` / `delete` / `watch` / `set_status` / `requeue_after` /
//! `children`, and none of them returns a dependency's runtime status, so the
//! readiness term evaluates fail-closed (see
//! [`DEPENDENCY_READINESS_PROVEN`]) and a dependency-proven `Ready` phase is
//! unreachable until the surface carries observed state.

#![deny(missing_docs)]

mod driver;

pub use driver::{
    DEPENDENCY_READINESS_PROVEN, PHASE_DEGRADED, PHASE_PENDING, PHASE_READY,
    TELEMETRY_SERVICE_TYPE, TELEMETRY_SERVICE_RESYNC, TelemetryServiceDriver,
    TelemetryServiceDriverError, TelemetryServiceDriverFactory, TelemetryServiceStatus,
    telemetry_service_descriptor, telemetry_service_spec_decoder,
};
