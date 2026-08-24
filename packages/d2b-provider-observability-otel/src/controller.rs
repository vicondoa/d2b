//! Telemetry Service and Binding reconciliation through typed child intents.

use d2b_contracts_provider::v3::semantic_services::{
    SemanticFamily,
    child_resources::{
        BindingChildKind, BindingChildPlacement, BindingChildRequest, BindingChildSet,
        explicit_binding_children,
    },
};
use d2b_contracts_resource::v3::{ExecutionDomain, ResourceRef};

use crate::{
    IdentityCanaries, Ingress, IngressErrorClass, IngressOutcome, IngressPolicyGate, MetricFrame,
};

const TELEMETRY_PROVIDER_REF: &str = "Provider/observability-otel";

const TELEMETRY_ZONE_BINDING_CHILD_REQUESTS: [BindingChildRequest; 2] = [
    BindingChildRequest::process(
        BindingChildKind::Process,
        BindingChildPlacement::Host,
        "collector",
        "Provider/system-minijail",
        "otel-collector",
        ExecutionDomain::System,
        "service",
    ),
    BindingChildRequest::endpoint(BindingChildPlacement::Host, "ingest-endpoint", "collector"),
];

const TELEMETRY_GUEST_BINDING_CHILD_REQUESTS: [BindingChildRequest; 4] = [
    BindingChildRequest::process(
        BindingChildKind::Process,
        BindingChildPlacement::Host,
        "collector",
        "Provider/system-minijail",
        "otel-collector",
        ExecutionDomain::System,
        "service",
    ),
    BindingChildRequest::endpoint(BindingChildPlacement::Host, "ingest-endpoint", "collector"),
    BindingChildRequest::process(
        BindingChildKind::Process,
        BindingChildPlacement::Host,
        "forwarder",
        "Provider/system-minijail",
        "otel-vsock-forwarder",
        ExecutionDomain::System,
        "worker",
    ),
    BindingChildRequest::endpoint(
        BindingChildPlacement::Host,
        "forwarder-endpoint",
        "forwarder",
    ),
];

/// Closed lifecycle phase for one telemetry Binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryBindingPhase {
    /// The Binding's collector children or route are not ready.
    Pending,
    /// The route accepted the most recent frame.
    Ready,
    /// The route rejected or quarantined a frame.
    Degraded,
    /// The Binding's children have been released.
    Deleted,
}

/// Status observed by the telemetry Binding controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelemetryBindingStatus {
    /// Current route lifecycle phase.
    pub phase: TelemetryBindingPhase,
    /// Most recent ingress outcome.
    pub outcome: Option<IngressOutcome>,
    /// Most recent policy error, when one was reported.
    pub error_class: Option<IngressErrorClass>,
}

/// Reconcile result including explicit child-resource intents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryReconcileResult {
    /// Binding status after the route decision.
    pub status: TelemetryBindingStatus,
    /// UID-free collector Process and Endpoint intents.
    pub children: BindingChildSet,
}

/// Closed telemetry controller failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryControllerError {
    /// Binding, Service, target, or Provider admission failed.
    Admission,
    /// Reconciliation was attempted after finalization.
    Finalized,
}

impl core::fmt::Display for TelemetryControllerError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Admission => "telemetry-controller-admission-failed",
            Self::Finalized => "telemetry-controller-finalized",
        })
    }
}

impl std::error::Error for TelemetryControllerError {}

/// Provider-owned telemetry Binding controller.
///
/// The controller owns only bounded ingress policy state and child intent
/// declarations. Process launch, Endpoint publication, and cleanup remain
/// Core-managed resource effects.
pub struct TelemetryBindingController {
    gate: IngressPolicyGate,
    phase: TelemetryBindingPhase,
}

impl core::fmt::Debug for TelemetryBindingController {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("TelemetryBindingController")
            .field("phase", &self.phase)
            .field("gate", &self.gate)
            .finish()
    }
}

impl Default for TelemetryBindingController {
    fn default() -> Self {
        Self::new()
    }
}

impl TelemetryBindingController {
    /// Construct an empty telemetry Binding controller.
    pub fn new() -> Self {
        Self {
            gate: IngressPolicyGate::default(),
            phase: TelemetryBindingPhase::Pending,
        }
    }

    /// Return the current Binding lifecycle phase.
    pub const fn phase(&self) -> TelemetryBindingPhase {
        self.phase
    }

    /// Build the explicit collector children for one authored Binding.
    ///
    /// The telemetry collector is host-placed even when its producer is a
    /// Guest or Zone. The `target_ref` is the producer target from the
    /// semantic Binding and is still required for Core admission.
    pub fn child_resources(
        binding_ref: &ResourceRef,
        service_ref: &ResourceRef,
        target_ref: &ResourceRef,
    ) -> Result<BindingChildSet, TelemetryControllerError> {
        let declarations = if target_ref.resource_type().as_str() == "Guest" {
            &TELEMETRY_GUEST_BINDING_CHILD_REQUESTS[..]
        } else {
            &TELEMETRY_ZONE_BINDING_CHILD_REQUESTS[..]
        };
        explicit_binding_children(
            SemanticFamily::Telemetry,
            binding_ref.clone(),
            service_ref.clone(),
            target_ref.clone(),
            ResourceRef::parse(TELEMETRY_PROVIDER_REF)
                .expect("telemetry Provider reference is canonical"),
            declarations,
        )
        .map_err(|_| TelemetryControllerError::Admission)
    }

    /// Reconcile one bounded ingress frame and return the children owned by
    /// the explicit Binding.
    pub fn reconcile(
        &mut self,
        binding_ref: &ResourceRef,
        service_ref: &ResourceRef,
        target_ref: &ResourceRef,
        ingress: Ingress,
        connection_id: u64,
        frame: &MetricFrame,
        canaries: &IdentityCanaries,
        capacity_available: bool,
    ) -> Result<TelemetryReconcileResult, TelemetryControllerError> {
        if self.phase == TelemetryBindingPhase::Deleted {
            return Err(TelemetryControllerError::Finalized);
        }
        let children = Self::child_resources(binding_ref, service_ref, target_ref)?;
        let (outcome, error_class) = self.gate.admit_for_connection(
            ingress,
            connection_id,
            frame,
            canaries,
            capacity_available,
        );
        self.phase = match outcome {
            IngressOutcome::Accepted => TelemetryBindingPhase::Ready,
            IngressOutcome::Rejected | IngressOutcome::Quarantined => {
                TelemetryBindingPhase::Degraded
            }
        };
        Ok(TelemetryReconcileResult {
            status: TelemetryBindingStatus {
                phase: self.phase,
                outcome: Some(outcome),
                error_class: Some(error_class),
            },
            children,
        })
    }

    /// Release the Binding's child intents before its finalizer is removed.
    pub fn finalize(&mut self) -> Result<(), TelemetryControllerError> {
        if self.phase == TelemetryBindingPhase::Deleted {
            return Ok(());
        }
        self.phase = TelemetryBindingPhase::Deleted;
        Ok(())
    }
}
