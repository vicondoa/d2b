//! Provider lifecycle effect adapter.

use std::collections::BTreeMap;

use d2b_telemetry::BoundedEmitter;

/// Closed Provider component phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentPhase {
    /// Waiting for installation.
    Pending,
    /// Ready to serve.
    Ready,
    /// Serving with reduced health.
    Degraded,
    /// Failed closed.
    Failed,
    /// No observation yet.
    Unknown,
}

impl ComponentPhase {
    /// Stable label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }
}

/// A Provider lifecycle dispatch entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderLifecycleDispatch {
    provider_class: String,
    components: BTreeMap<String, ComponentPhase>,
}

impl ProviderLifecycleDispatch {
    /// Construct an empty dispatch for one fixed Provider class.
    pub fn new(provider_class: impl Into<String>) -> Self {
        Self {
            provider_class: provider_class.into(),
            components: BTreeMap::new(),
        }
    }

    /// Set one closed component phase.
    pub fn set_phase(
        &mut self,
        component_type: impl Into<String>,
        phase: ComponentPhase,
    ) -> Result<(), ProviderEffectError> {
        let component_type = component_type.into();
        if !matches!(component_type.as_str(), "controller" | "service" | "worker") {
            return Err(ProviderEffectError::ComponentTypeUnknown);
        }
        self.components.insert(component_type, phase);
        Ok(())
    }

    /// Set one component phase and emit its bounded lifecycle gauge.
    pub fn set_phase_with_telemetry(
        &mut self,
        component_type: impl Into<String>,
        phase: ComponentPhase,
        emitter: &BoundedEmitter,
    ) -> Result<(), ProviderEffectError> {
        let component_type = component_type.into();
        self.set_phase(component_type.clone(), phase)?;
        let _ = crate::metrics::ControllerMetrics::new(emitter.clone())
            .provider_component_phase(component_type, phase.as_str());
        Ok(())
    }

    /// Emit the current component phase snapshot through a bounded emitter.
    pub fn emit_phases(&self, emitter: &BoundedEmitter) {
        let metrics = crate::metrics::ControllerMetrics::new(emitter.clone());
        for (component_type, phase) in &self.components {
            let _ = metrics.provider_component_phase(component_type.clone(), phase.as_str());
        }
    }

    /// Provider class used for the fixed metric label.
    pub fn provider_class(&self) -> &str {
        &self.provider_class
    }

    /// Component phases.
    pub const fn components(&self) -> &BTreeMap<String, ComponentPhase> {
        &self.components
    }
}

/// Provider effect failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderEffectError {
    /// Component class is outside the closed set.
    ComponentTypeUnknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_dispatch_uses_closed_component_phases() {
        let mut dispatch = ProviderLifecycleDispatch::new("observability-otel");
        dispatch
            .set_phase("service", ComponentPhase::Ready)
            .unwrap();
        assert_eq!(dispatch.components()["service"].as_str(), "ready");
        assert!(
            dispatch
                .set_phase("resource-name", ComponentPhase::Ready)
                .is_err()
        );
    }

    #[test]
    fn lifecycle_phase_callsite_emits_only_closed_labels() {
        let emitter = d2b_telemetry::BoundedEmitter::new("/nonexistent", 2048).unwrap();
        let mut dispatch = ProviderLifecycleDispatch::new("observability-otel");
        dispatch
            .set_phase_with_telemetry("service", ComponentPhase::Ready, &emitter)
            .unwrap();
        dispatch.emit_phases(&emitter);
    }
}
