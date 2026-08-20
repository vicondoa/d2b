//! Closed v3 service and method descriptors.
//!
//! The bus routes an exact service package and method.  A descriptor
//! fingerprint is derived from the canonical ordered method set, so adding,
//! removing, or renaming a method cannot be mistaken for the old service.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use super::{
    SchemaFingerprint, ServiceName,
    execution_policy::{BoundedText, redacted_debug},
};

/// Maximum methods in one service descriptor.
pub const MAX_SERVICE_METHODS: usize = 64;
/// Maximum bytes in one service package descriptor.
pub const MAX_SERVICE_DESCRIPTOR_BYTES: usize = 16 * 1024;

/// Closed v3 service packages.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum V3Service {
    Resource,
    Zone,
    ZoneLink,
    Provider,
    Controller,
    Audit,
    Support,
    Credential,
}

impl V3Service {
    /// Stable service package name.
    pub const fn package(self) -> &'static str {
        match self {
            Self::Resource => "d2b.resource.v3",
            Self::Zone => "d2b.zone.v3",
            Self::ZoneLink => "d2b.zonelink.v3",
            Self::Provider => "d2b.provider.v3",
            Self::Controller => "d2b.controller.v3",
            Self::Audit => "d2b.audit.v3",
            Self::Support => "d2b.support.v3",
            Self::Credential => "d2b.credential.v3",
        }
    }

    /// Parse a closed service package.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|service| service.package() == value)
    }

    /// Every service package in stable order.
    pub const ALL: [Self; 8] = [
        Self::Resource,
        Self::Zone,
        Self::ZoneLink,
        Self::Provider,
        Self::Controller,
        Self::Audit,
        Self::Support,
        Self::Credential,
    ];
}

/// Closed ResourceService method set.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "PascalCase")]
pub enum ResourceMethod {
    Get,
    List,
    Watch,
    Create,
    UpdateSpec,
    UpdateStatus,
    UpdateMetadata,
    UpdateFinalizers,
    Delete,
    CommitBatch,
    ResolveRef,
    InspectSchema,
    Upgrade,
}

impl ResourceMethod {
    /// Stable generated-method name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "Get",
            Self::List => "List",
            Self::Watch => "Watch",
            Self::Create => "Create",
            Self::UpdateSpec => "UpdateSpec",
            Self::UpdateStatus => "UpdateStatus",
            Self::UpdateMetadata => "UpdateMetadata",
            Self::UpdateFinalizers => "UpdateFinalizers",
            Self::Delete => "Delete",
            Self::CommitBatch => "CommitBatch",
            Self::ResolveRef => "ResolveRef",
            Self::InspectSchema => "InspectSchema",
            Self::Upgrade => "Upgrade",
        }
    }

    /// Exact ResourceService method catalogue.
    pub const ALL: [Self; 13] = [
        Self::Get,
        Self::List,
        Self::Watch,
        Self::Create,
        Self::UpdateSpec,
        Self::UpdateStatus,
        Self::UpdateMetadata,
        Self::UpdateFinalizers,
        Self::Delete,
        Self::CommitBatch,
        Self::ResolveRef,
        Self::InspectSchema,
        Self::Upgrade,
    ];
}

/// Closed Zone service methods.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "PascalCase")]
pub enum ZoneMethod {
    Get,
    Status,
    List,
    Attach,
}

impl ZoneMethod {
    /// Stable generated-method name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "Get",
            Self::Status => "Status",
            Self::List => "List",
            Self::Attach => "Attach",
        }
    }
}

/// Closed Provider-agent methods whose payloads are generic canonical
/// request/response objects until a family-specific amendment freezes DTO
/// fields and protobuf numbers.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum ProviderMethod {
    OpenTransport,
    CloseTransport,
    ObserveTransport,
    AssessUpdate,
    PlanUpgrade,
    ExecuteUpgrade,
}

impl ProviderMethod {
    /// Stable method token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenTransport => "openTransport",
            Self::CloseTransport => "closeTransport",
            Self::ObserveTransport => "observeTransport",
            Self::AssessUpdate => "assessUpdate",
            Self::PlanUpgrade => "planUpgrade",
            Self::ExecuteUpgrade => "executeUpgrade",
        }
    }
}

/// A canonical service descriptor and its derived fingerprint.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceDescriptor {
    package: ServiceName,
    methods: Vec<String>,
    fingerprint: SchemaFingerprint,
}

impl ServiceDescriptor {
    /// Construct a descriptor from a closed package and method names.
    pub fn new(
        package: ServiceName,
        mut methods: Vec<String>,
    ) -> Result<Self, ServiceDescriptorError> {
        if V3Service::parse(package.as_str()).is_none() {
            return Err(ServiceDescriptorError::UnknownPackage);
        }
        if methods.is_empty() || methods.len() > MAX_SERVICE_METHODS {
            return Err(ServiceDescriptorError::MethodBound);
        }
        methods.sort();
        if methods.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ServiceDescriptorError::DuplicateMethod);
        }
        if methods
            .iter()
            .any(|method| BoundedText::parse(method.clone()).is_err())
        {
            return Err(ServiceDescriptorError::InvalidMethod);
        }
        let fingerprint = fingerprint(&package, &methods)?;
        Ok(Self {
            package,
            methods,
            fingerprint,
        })
    }

    /// Construct the exact ResourceService descriptor.
    pub fn resource() -> Self {
        Self::new(
            ServiceName::parse(V3Service::Resource.package()).expect("closed package"),
            ResourceMethod::ALL
                .into_iter()
                .map(|method| method.as_str().to_owned())
                .collect(),
        )
        .expect("closed ResourceService descriptor")
    }

    /// Borrow service package.
    pub const fn package(&self) -> &ServiceName {
        &self.package
    }

    /// Borrow canonical sorted method names.
    pub fn methods(&self) -> &[String] {
        &self.methods
    }

    /// Borrow derived fingerprint.
    pub const fn fingerprint(&self) -> &SchemaFingerprint {
        &self.fingerprint
    }

    /// Return whether this exact method is published.
    pub fn contains_method(&self, method: &str) -> bool {
        self.methods
            .binary_search_by(|candidate| candidate.as_str().cmp(method))
            .is_ok()
    }
}

redacted_debug!(ServiceDescriptor);

impl<'de> Deserialize<'de> for ServiceDescriptor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            package: ServiceName,
            methods: Vec<String>,
            fingerprint: SchemaFingerprint,
        }
        let wire = Wire::deserialize(deserializer)?;
        let descriptor = Self::new(wire.package, wire.methods).map_err(serde::de::Error::custom)?;
        if descriptor.fingerprint != wire.fingerprint {
            return Err(serde::de::Error::custom(
                ServiceDescriptorError::FingerprintMismatch,
            ));
        }
        Ok(descriptor)
    }
}

fn fingerprint(
    package: &ServiceName,
    methods: &[String],
) -> Result<SchemaFingerprint, ServiceDescriptorError> {
    let mut hasher = Sha256::new();
    hasher.update(b"d2b:v3:service-descriptor");
    hasher.update([0]);
    hasher.update(package.as_str().as_bytes());
    for method in methods {
        hasher.update([0]);
        hasher.update(method.as_bytes());
    }
    SchemaFingerprint::parse(format!("sha256:{:x}", hasher.finalize()))
        .map_err(|_| ServiceDescriptorError::InvalidFingerprint)
}

/// Service descriptor failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceDescriptorError {
    UnknownPackage,
    MethodBound,
    DuplicateMethod,
    InvalidMethod,
    InvalidFingerprint,
    FingerprintMismatch,
}

impl core::fmt::Display for ServiceDescriptorError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::UnknownPackage => "service-package-unknown",
            Self::MethodBound => "service-method-bound-exceeded",
            Self::DuplicateMethod => "service-method-duplicate",
            Self::InvalidMethod => "service-method-invalid",
            Self::InvalidFingerprint => "service-fingerprint-invalid",
            Self::FingerprintMismatch => "service-fingerprint-mismatch",
        })
    }
}

impl std::error::Error for ServiceDescriptorError {}

/// A closed audit coverage segment.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AuditSegment {
    Authentication,
    Authorization,
    Dispatch,
    Effect,
    Completion,
}

/// Return missing segments in an observed audit sequence.
pub fn missing_audit_segments(observed: &[AuditSegment]) -> Vec<AuditSegment> {
    AuditSegment::ALL
        .into_iter()
        .filter(|segment| !observed.contains(segment))
        .collect()
}

impl AuditSegment {
    /// Every audit segment in required order.
    pub const ALL: [Self; 5] = [
        Self::Authentication,
        Self::Authorization,
        Self::Dispatch,
        Self::Effect,
        Self::Completion,
    ];
}

/// Strictly reject an unknown field from a canonical wire object.
pub fn strict_service_object(
    value: &super::CanonicalJsonObject,
    allowed: &[&str],
) -> Result<(), ServiceDescriptorError> {
    if value.keys().all(|key| allowed.contains(&key)) {
        Ok(())
    } else {
        Err(ServiceDescriptorError::InvalidMethod)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_service_fingerprint_changes_when_method_changes() {
        let resource = ServiceDescriptor::resource();
        let mut methods = resource.methods().to_vec();
        methods.pop();
        let changed = ServiceDescriptor::new(resource.package().clone(), methods).unwrap();
        assert_ne!(resource.fingerprint(), changed.fingerprint());
    }

    #[test]
    fn audit_gap_detection_covers_each_missing_segment() {
        let missing = missing_audit_segments(&[
            AuditSegment::Authentication,
            AuditSegment::Authorization,
            AuditSegment::Effect,
            AuditSegment::Completion,
        ]);
        assert_eq!(missing, vec![AuditSegment::Dispatch]);
    }
}
