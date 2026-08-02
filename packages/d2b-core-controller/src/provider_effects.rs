//! Provider lifecycle effect adapter.

use std::collections::BTreeMap;

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
}
