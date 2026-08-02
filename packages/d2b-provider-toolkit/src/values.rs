//! Provider-neutral health, inspection, and observability values.
//!
//! These values are deliberately small and redacted. They carry enough
//! identity and generation information for conformance assertions while
//! keeping Provider names, payloads, paths, and credentials out of debug
//! output and canonical response fixtures.

use std::fmt;

use d2b_contracts::v3::CanonicalJsonObject;
use d2b_provider::{ProviderClass, ProviderDescriptor, RegistryBuildError};

use crate::error::ProviderToolkitError;

/// Error constructing a bounded Provider value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValuesError {
    /// The descriptor failed its contract validation.
    DescriptorInvalid,
    /// A generated canonical response could not be parsed.
    CanonicalResponseInvalid,
}

impl fmt::Display for ValuesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DescriptorInvalid => "provider values descriptor is invalid",
            Self::CanonicalResponseInvalid => "provider values response is invalid",
        })
    }
}

impl std::error::Error for ValuesError {}

/// Health state returned by a fake Provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderHealthState {
    /// The Provider can serve all published methods.
    Healthy,
    /// The Provider can serve but has a non-fatal condition.
    Degraded,
    /// The Provider cannot serve the request.
    Failed,
}

impl ProviderHealthState {
    /// Return the stable lower-kebab spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
        }
    }
}

/// Bounded health observation.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderHealth {
    state: ProviderHealthState,
    observed_at_unix_ms: u64,
    schema_version: u32,
    registry_generation: u64,
    provider_generation: u64,
    capability_count: usize,
}

impl fmt::Debug for ProviderHealth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderHealth")
            .field("state", &self.state)
            .field("observed_at_unix_ms", &self.observed_at_unix_ms)
            .field("schema_version", &self.schema_version)
            .field("registry_generation", &self.registry_generation)
            .field("provider_generation", &self.provider_generation)
            .field("capability_count", &self.capability_count)
            .finish()
    }
}

impl ProviderHealth {
    /// Return the health state.
    pub const fn state(&self) -> ProviderHealthState {
        self.state
    }

    /// Return the observation timestamp.
    pub const fn observed_at_unix_ms(&self) -> u64 {
        self.observed_at_unix_ms
    }

    /// Return the registry generation.
    pub const fn registry_generation(&self) -> u64 {
        self.registry_generation
    }

    /// Return the Provider resource generation.
    pub const fn provider_generation(&self) -> u64 {
        self.provider_generation
    }

    /// Return the published method count.
    pub const fn capability_count(&self) -> usize {
        self.capability_count
    }
}

/// Provider descriptor inspection result.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderInspection {
    class: ProviderClass,
    schema_version: u32,
    service: String,
    capability_count: usize,
}

impl fmt::Debug for ProviderInspection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderInspection")
            .field("class", &self.class)
            .field("schema_version", &self.schema_version)
            .field("capability_count", &self.capability_count)
            .finish_non_exhaustive()
    }
}

impl ProviderInspection {
    /// Return the Provider family.
    pub const fn class(&self) -> ProviderClass {
        self.class
    }

    /// Return the schema version.
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Return the selected service name.
    pub fn service(&self) -> &str {
        &self.service
    }

    /// Return the published method count.
    pub const fn capability_count(&self) -> usize {
        self.capability_count
    }
}

/// A bounded observability result for the fake Provider.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderObservability {
    observed_at_unix_ms: u64,
    registry_generation: u64,
    sequence: &'static [&'static str],
}

impl fmt::Debug for ProviderObservability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderObservability")
            .field("observed_at_unix_ms", &self.observed_at_unix_ms)
            .field("registry_generation", &self.registry_generation)
            .field("sequence_len", &self.sequence.len())
            .finish()
    }
}

impl ProviderObservability {
    /// Return the observation timestamp.
    pub const fn observed_at_unix_ms(&self) -> u64 {
        self.observed_at_unix_ms
    }

    /// Return the registry generation.
    pub const fn registry_generation(&self) -> u64 {
        self.registry_generation
    }

    /// Return the closed event sequence.
    pub const fn sequence(&self) -> &'static [&'static str] {
        self.sequence
    }
}

/// Values bound to one Provider descriptor and one deterministic timestamp.
#[derive(Clone)]
pub struct ProviderValues {
    descriptor: ProviderDescriptor,
    now_unix_ms: u64,
}

impl fmt::Debug for ProviderValues {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderValues")
            .field("class", &self.descriptor.class())
            .field("schema_version", &self.descriptor.schema_version())
            .field(
                "provider_generation",
                &self.descriptor.provider_generation(),
            )
            .field("now_unix_ms", &self.now_unix_ms)
            .finish_non_exhaustive()
    }
}

impl ProviderValues {
    /// Bind values to a validated descriptor.
    pub fn new(descriptor: &ProviderDescriptor, now_unix_ms: u64) -> Result<Self, ValuesError> {
        descriptor
            .validate()
            .map_err(|_: RegistryBuildError| ValuesError::DescriptorInvalid)?;
        if now_unix_ms == 0 {
            return Err(ValuesError::DescriptorInvalid);
        }
        Ok(Self {
            descriptor: descriptor.clone(),
            now_unix_ms,
        })
    }

    /// Borrow the bound descriptor.
    pub const fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }

    /// Return the deterministic timestamp.
    pub const fn now_unix_ms(&self) -> u64 {
        self.now_unix_ms
    }

    /// Build a healthy observation.
    pub fn health(&self) -> ProviderHealth {
        ProviderHealth {
            state: ProviderHealthState::Healthy,
            observed_at_unix_ms: self.now_unix_ms,
            schema_version: self.descriptor.schema_version(),
            registry_generation: self.descriptor.registry_generation().get(),
            provider_generation: self.descriptor.provider_generation().get(),
            capability_count: self.descriptor.capabilities().len(),
        }
    }

    /// Build a descriptor inspection result.
    pub fn inspection(&self) -> ProviderInspection {
        ProviderInspection {
            class: self.descriptor.class(),
            schema_version: self.descriptor.schema_version(),
            service: self.descriptor.service().as_str().to_owned(),
            capability_count: self.descriptor.capabilities().len(),
        }
    }

    /// Build the closed health/inspection/observability sequence.
    pub fn observability(&self) -> ProviderObservability {
        ProviderObservability {
            observed_at_unix_ms: self.now_unix_ms,
            registry_generation: self.descriptor.registry_generation().get(),
            sequence: &["health", "inspect", "observability"],
        }
    }

    /// Render the health value as a canonical response object.
    pub fn health_payload(&self) -> Result<CanonicalJsonObject, ProviderToolkitError> {
        let health = self.health();
        parse_payload(format!(
            r#"{{"capabilityCount":{},"providerGeneration":{},"registryGeneration":{},"schemaVersion":{},"state":"{}","observedAtUnixMs":{}}}"#,
            health.capability_count(),
            health.provider_generation(),
            health.registry_generation(),
            self.descriptor.schema_version(),
            health.state().as_str(),
            health.observed_at_unix_ms(),
        ))
    }

    /// Render the inspection value as a canonical response object.
    pub fn inspection_payload(&self) -> Result<CanonicalJsonObject, ProviderToolkitError> {
        let inspection = self.inspection();
        parse_payload(format!(
            r#"{{"capabilityCount":{},"class":"{}","schemaVersion":{},"service":"{}"}}"#,
            inspection.capability_count(),
            inspection.class().as_str(),
            inspection.schema_version(),
            inspection.service(),
        ))
    }

    /// Render the observability value as a canonical response object.
    pub fn observability_payload(&self) -> Result<CanonicalJsonObject, ProviderToolkitError> {
        let observability = self.observability();
        parse_payload(format!(
            r#"{{"observedAtUnixMs":{},"registryGeneration":{},"sequence":["health","inspect","observability"]}}"#,
            observability.observed_at_unix_ms(),
            observability.registry_generation(),
        ))
    }
}

fn parse_payload(value: String) -> Result<CanonicalJsonObject, ProviderToolkitError> {
    CanonicalJsonObject::parse(value.as_bytes()).map_err(|_| ProviderToolkitError::WireInvalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FakeProvider, Fixture};

    #[test]
    fn values_bind_generation_and_timestamp_without_debug_identity() {
        let fixture = Fixture::new(ProviderClass::Runtime, 0).expect("fixture");
        let values = ProviderValues::new(&fixture.descriptor, fixture.now_unix_ms).expect("values");
        let rendered = format!("{values:?}");
        assert!(!rendered.contains(fixture.descriptor.provider_ref().name().as_str()));
        assert_eq!(
            values.health().provider_generation(),
            fixture.descriptor.provider_generation().get()
        );
        assert_eq!(values.health().observed_at_unix_ms(), fixture.now_unix_ms);
    }

    #[test]
    fn generated_payloads_are_canonical_objects() {
        let fixture = Fixture::new(ProviderClass::Runtime, 0).expect("fixture");
        let values = ProviderValues::new(&fixture.descriptor, fixture.now_unix_ms).expect("values");
        assert_eq!(values.health_payload().expect("health").len(), 6);
        assert_eq!(values.inspection_payload().expect("inspection").len(), 4);
        assert_eq!(
            values.observability_payload().expect("observability").len(),
            3
        );
        FakeProvider::new(fixture)
            .conformance_sequence()
            .expect("sequence");
    }
}
