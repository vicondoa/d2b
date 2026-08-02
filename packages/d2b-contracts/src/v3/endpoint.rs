//! Provider-neutral stable endpoint resource contracts.
//!
//! An endpoint is an identity and policy object, not a locator.  The
//! transport, address, descriptor, and credential used to resolve it remain
//! private to the effect adapter.  Keeping the endpoint contract here makes
//! it possible for Resource API, Nix, and Provider code to share one strict
//! vocabulary without importing any runtime transport type.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    ResourceRef,
    execution_policy::{BoundedText, BoundedToken, PrimitiveSpecError, redacted_debug},
};

/// The canonical ResourceType name for endpoints.
pub const ENDPOINT_RESOURCE_TYPE: &str = "Endpoint";
/// The maximum number of entries in one endpoint consumer allowlist.
pub const MAX_ENDPOINT_CONSUMER_ENTRIES: usize = 64;
/// The maximum attachment count an endpoint may advertise.
pub const MAX_ENDPOINT_ATTACHMENTS: u16 = 64;
/// The maximum signed component entries in one consumer policy.
pub const MAX_ENDPOINT_PROVIDER_COMPONENTS: usize = 32;
/// The maximum operation entries in one consumer policy.
pub const MAX_ENDPOINT_OPERATIONS: usize = 3;
/// The maximum service fingerprint bytes.
pub const MAX_ENDPOINT_FINGERPRINT_BYTES: usize = 71;

/// The semantic class of a stable endpoint.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum EndpointClass {
    /// A typed service API.
    Service,
    /// A device-facing endpoint.
    Device,
    /// A stable transport attachment.
    Transport,
    /// A lifecycle or control endpoint.
    Control,
    /// A data endpoint.
    Data,
}

/// The opaque transport class used to resolve an endpoint.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum EndpointTransport {
    /// A Unix-domain transport resolved by the local effect owner.
    Unix,
    /// A guest vsock transport resolved by the owning runtime.
    Vsock,
    /// A policy-authorized TCP transport.
    Tcp,
    /// A descriptor supplied through the ComponentSession attachment path.
    FdAttachment,
    /// Provider-owned carriage with no public locator.
    OpaqueCarriage,
}

/// The locality class observed and requested for an endpoint.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum EndpointLocality {
    /// Resolved only on the Host that owns the producer.
    HostLocal,
    /// Resolved only inside the Guest that owns the producer.
    GuestLocal,
    /// Resolved across a declared execution-domain boundary.
    CrossDomain,
    /// Resolved within the current Zone.
    ZoneLocal,
}

/// The coarse visibility scope for endpoint candidates.
///
/// This enum is deliberately closed.  In particular, `private`,
/// `provider-internal`, and `authorized-consumers` are not aliases.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum EndpointVisibility {
    /// Only the exact owner is a candidate.
    Owner,
    /// Authenticated Provider subjects and signed components are candidates.
    Provider,
    /// Same-Zone subjects are candidates, subject to normal authorization.
    Zone,
}

/// A fine-grained operation a consumer may perform.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum EndpointOperation {
    /// Resolve the endpoint to an opaque carriage.
    Resolve,
    /// Attach a named stream or descriptor.
    Attach,
    /// Read bounded endpoint observations.
    Observe,
}

/// Endpoint attachment capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EndpointAttachmentPolicy {
    /// Whether this endpoint accepts attachments at all.
    #[serde(default)]
    pub supported: bool,
    /// Maximum simultaneous attachments.
    #[serde(default)]
    pub max_attachments: u16,
}

impl EndpointAttachmentPolicy {
    /// Construct a bounded attachment policy.
    pub fn new(supported: bool, max_attachments: u16) -> Result<Self, EndpointSpecError> {
        if max_attachments > MAX_ENDPOINT_ATTACHMENTS
            || (!supported && max_attachments != 0)
            || (supported && max_attachments == 0)
        {
            return Err(EndpointSpecError::InvalidAttachmentPolicy);
        }
        Ok(Self {
            supported,
            max_attachments,
        })
    }
}

/// The only fine-grained endpoint consumer policy.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EndpointConsumerPolicy {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_subjects: Vec<ResourceRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_provider_components: Vec<BoundedToken>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_operations: Vec<EndpointOperation>,
}

impl EndpointConsumerPolicy {
    /// Construct an allowlist policy, canonicalizing each unordered list.
    pub fn new(
        mut allowed_subjects: Vec<ResourceRef>,
        mut allowed_provider_components: Vec<BoundedToken>,
        mut allowed_operations: Vec<EndpointOperation>,
    ) -> Result<Self, EndpointSpecError> {
        if allowed_subjects.len() > MAX_ENDPOINT_CONSUMER_ENTRIES
            || allowed_provider_components.len() > MAX_ENDPOINT_PROVIDER_COMPONENTS
            || allowed_operations.len() > MAX_ENDPOINT_OPERATIONS
        {
            return Err(EndpointSpecError::TooManyConsumerEntries);
        }
        allowed_subjects.sort_by_key(ResourceRef::to_canonical_string);
        allowed_provider_components.sort();
        allowed_operations.sort();
        if has_duplicates(&allowed_subjects)
            || has_duplicates(&allowed_provider_components)
            || has_duplicates(&allowed_operations)
        {
            return Err(EndpointSpecError::DuplicateConsumerEntry);
        }
        Ok(Self {
            allowed_subjects,
            allowed_provider_components,
            allowed_operations,
        })
    }

    /// Construct the unconstrained fine-grained policy.
    pub fn unrestricted() -> Self {
        Self {
            allowed_subjects: Vec::new(),
            allowed_provider_components: Vec::new(),
            allowed_operations: Vec::new(),
        }
    }

    /// Borrow the exact subject allowlist.
    pub fn allowed_subjects(&self) -> &[ResourceRef] {
        &self.allowed_subjects
    }

    /// Borrow the signed Provider component allowlist.
    pub fn allowed_provider_components(&self) -> &[BoundedToken] {
        &self.allowed_provider_components
    }

    /// Borrow the operation allowlist.
    pub fn allowed_operations(&self) -> &[EndpointOperation] {
        &self.allowed_operations
    }
}

redacted_debug!(EndpointConsumerPolicy);

impl<'de> Deserialize<'de> for EndpointConsumerPolicy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            allowed_subjects: Vec<ResourceRef>,
            #[serde(default)]
            allowed_provider_components: Vec<BoundedToken>,
            #[serde(default)]
            allowed_operations: Vec<EndpointOperation>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.allowed_subjects,
            wire.allowed_provider_components,
            wire.allowed_operations,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Endpoint lifecycle and generation behavior.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum EndpointLifecyclePolicy {
    /// Keep the endpoint identity until an explicit delete.
    Pinned,
    /// Recycle it whenever the producer recycles.
    RecycleWithProducer,
    /// Recreate it whenever the producer generation changes.
    RecreateOnGeneration,
}

/// The stable, locator-free Endpoint base spec.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EndpointSpec {
    provider_ref: ResourceRef,
    producer_ref: ResourceRef,
    endpoint_class: EndpointClass,
    transport: EndpointTransport,
    purpose: BoundedToken,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_fingerprint: Option<BoundedText>,
    locality: EndpointLocality,
    visibility: EndpointVisibility,
    attachment_policy: EndpointAttachmentPolicy,
    consumer_policy: EndpointConsumerPolicy,
    lifecycle_policy: EndpointLifecyclePolicy,
}

impl EndpointSpec {
    /// Construct a strict endpoint specification.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider_ref: ResourceRef,
        producer_ref: ResourceRef,
        endpoint_class: EndpointClass,
        transport: EndpointTransport,
        purpose: BoundedToken,
        service_fingerprint: Option<BoundedText>,
        locality: EndpointLocality,
        visibility: EndpointVisibility,
        attachment_policy: EndpointAttachmentPolicy,
        consumer_policy: EndpointConsumerPolicy,
        lifecycle_policy: EndpointLifecyclePolicy,
    ) -> Result<Self, EndpointSpecError> {
        if provider_ref.resource_type().as_str() != "Provider" {
            return Err(EndpointSpecError::WrongProviderRef);
        }
        if service_fingerprint
            .as_ref()
            .is_some_and(|fingerprint| fingerprint.as_str().len() > MAX_ENDPOINT_FINGERPRINT_BYTES)
        {
            return Err(EndpointSpecError::Primitive(
                PrimitiveSpecError::InvalidText,
            ));
        }
        let producer_type = producer_ref.resource_type().as_str();
        if !matches!(
            producer_type,
            "Process" | "EphemeralProcess" | "Device" | "Guest" | "Host"
        ) && !producer_type.contains(".d2bus.org.")
        {
            return Err(EndpointSpecError::InvalidProducerRef);
        }
        attachment_policy.validate()?;
        Ok(Self {
            provider_ref,
            producer_ref,
            endpoint_class,
            transport,
            purpose,
            service_fingerprint,
            locality,
            visibility,
            attachment_policy,
            consumer_policy,
            lifecycle_policy,
        })
    }

    /// Borrow the selected semantic Provider.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the producing resource.
    pub const fn producer_ref(&self) -> &ResourceRef {
        &self.producer_ref
    }

    /// Return the endpoint class.
    pub const fn endpoint_class(&self) -> EndpointClass {
        self.endpoint_class
    }

    /// Return the opaque transport class.
    pub const fn transport(&self) -> EndpointTransport {
        self.transport
    }

    /// Borrow the bounded purpose.
    pub const fn purpose(&self) -> &BoundedToken {
        &self.purpose
    }

    /// Borrow the optional service fingerprint.
    pub const fn service_fingerprint(&self) -> Option<&BoundedText> {
        self.service_fingerprint.as_ref()
    }

    /// Return endpoint locality.
    pub const fn locality(&self) -> EndpointLocality {
        self.locality
    }

    /// Return coarse visibility.
    pub const fn visibility(&self) -> EndpointVisibility {
        self.visibility
    }

    /// Borrow attachment policy.
    pub const fn attachment_policy(&self) -> EndpointAttachmentPolicy {
        self.attachment_policy
    }

    /// Borrow fine-grained consumer policy.
    pub const fn consumer_policy(&self) -> &EndpointConsumerPolicy {
        &self.consumer_policy
    }

    /// Return lifecycle behavior.
    pub const fn lifecycle_policy(&self) -> EndpointLifecyclePolicy {
        self.lifecycle_policy
    }
}

redacted_debug!(EndpointSpec);

impl<'de> Deserialize<'de> for EndpointSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            provider_ref: ResourceRef,
            producer_ref: ResourceRef,
            endpoint_class: EndpointClass,
            transport: EndpointTransport,
            purpose: BoundedToken,
            #[serde(default)]
            service_fingerprint: Option<BoundedText>,
            locality: EndpointLocality,
            #[serde(default = "provider_visibility")]
            visibility: EndpointVisibility,
            #[serde(default)]
            attachment_policy: EndpointAttachmentPolicy,
            #[serde(default)]
            consumer_policy: EndpointConsumerPolicy,
            #[serde(default = "recycle_with_producer")]
            lifecycle_policy: EndpointLifecyclePolicy,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.provider_ref,
            wire.producer_ref,
            wire.endpoint_class,
            wire.transport,
            wire.purpose,
            wire.service_fingerprint,
            wire.locality,
            wire.visibility,
            wire.attachment_policy,
            wire.consumer_policy,
            wire.lifecycle_policy,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl Default for EndpointConsumerPolicy {
    fn default() -> Self {
        Self::unrestricted()
    }
}

/// Stable endpoint contract errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointSpecError {
    /// `providerRef` does not name a Provider.
    WrongProviderRef,
    /// The producing resource is not an admitted producer type.
    InvalidProducerRef,
    /// Attachment bounds or support fields conflict.
    InvalidAttachmentPolicy,
    /// One consumer allowlist is too large.
    TooManyConsumerEntries,
    /// One consumer allowlist contains a duplicate.
    DuplicateConsumerEntry,
    /// A primitive field was invalid.
    Primitive(PrimitiveSpecError),
}

impl core::fmt::Display for EndpointSpecError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::WrongProviderRef => "endpoint provider reference must name Provider",
            Self::InvalidProducerRef => "endpoint producer reference is not admitted",
            Self::InvalidAttachmentPolicy => "endpoint attachment policy is invalid",
            Self::TooManyConsumerEntries => "endpoint consumer policy is too large",
            Self::DuplicateConsumerEntry => "endpoint consumer policy contains a duplicate",
            Self::Primitive(error) => return error.fmt(formatter),
        })
    }
}

impl std::error::Error for EndpointSpecError {}

impl From<PrimitiveSpecError> for EndpointSpecError {
    fn from(value: PrimitiveSpecError) -> Self {
        Self::Primitive(value)
    }
}

fn has_duplicates<T: PartialEq>(values: &[T]) -> bool {
    values.windows(2).any(|pair| pair[0] == pair[1])
}

fn provider_visibility() -> EndpointVisibility {
    EndpointVisibility::Provider
}

fn recycle_with_producer() -> EndpointLifecyclePolicy {
    EndpointLifecyclePolicy::RecycleWithProducer
}

impl EndpointAttachmentPolicy {
    fn validate(self) -> Result<(), EndpointSpecError> {
        Self::new(self.supported, self.max_attachments).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::resource_schema::canonical_json_bytes;

    fn minimal() -> EndpointSpec {
        EndpointSpec::new(
            ResourceRef::parse("Provider/display-wayland").unwrap(),
            ResourceRef::parse("Process/wayland-proxy").unwrap(),
            EndpointClass::Service,
            EndpointTransport::OpaqueCarriage,
            BoundedToken::parse("wayland-control").unwrap(),
            None,
            EndpointLocality::ZoneLocal,
            EndpointVisibility::Provider,
            EndpointAttachmentPolicy::default(),
            EndpointConsumerPolicy::default(),
            EndpointLifecyclePolicy::RecycleWithProducer,
        )
        .unwrap()
    }

    #[test]
    fn minimal_endpoint_vector_is_strict_and_canonical() {
        let endpoint = minimal();
        let bytes = canonical_json_bytes(&endpoint).unwrap();
        assert_eq!(
            bytes,
            br#"{"attachmentPolicy":{"maxAttachments":0,"supported":false},"consumerPolicy":{},"endpointClass":"service","lifecyclePolicy":"recycle-with-producer","locality":"zone-local","producerRef":"Process/wayland-proxy","providerRef":"Provider/display-wayland","purpose":"wayland-control","transport":"opaque-carriage","visibility":"provider"}"#
        );
        assert_eq!(
            serde_json::from_slice::<EndpointSpec>(&bytes).unwrap(),
            endpoint
        );
    }

    #[test]
    fn visibility_aliases_and_scalar_consumer_policy_are_rejected() {
        for value in ["private", "provider-internal", "authorized-consumers"] {
            let json = format!(
                r#"{{"providerRef":"Provider/display-wayland","producerRef":"Process/wayland-proxy","endpointClass":"service","transport":"opaque-carriage","purpose":"p","locality":"zone-local","visibility":"{value}"}}"#
            );
            assert!(serde_json::from_str::<EndpointSpec>(&json).is_err());
        }
        let mut object = serde_json::to_value(minimal()).unwrap();
        object["consumerPolicy"] = serde_json::json!("attach");
        assert!(serde_json::from_value::<EndpointSpec>(object).is_err());
        let mut object = serde_json::to_value(minimal()).unwrap();
        object["consumerPolicy"] = serde_json::json!(["attach"]);
        assert!(serde_json::from_value::<EndpointSpec>(object).is_err());
    }

    #[test]
    fn producer_and_provider_references_are_type_checked() {
        let mut object = serde_json::to_value(minimal()).unwrap();
        object["providerRef"] = serde_json::json!("Host/host-system");
        assert!(serde_json::from_value::<EndpointSpec>(object).is_err());
        let mut object = serde_json::to_value(minimal()).unwrap();
        object["producerRef"] = serde_json::json!("User/alice");
        assert!(serde_json::from_value::<EndpointSpec>(object).is_err());
    }
}
