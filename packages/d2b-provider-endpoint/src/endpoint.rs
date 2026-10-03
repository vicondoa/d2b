//! Provider-neutral stable endpoint resource contracts.
//!
//! An endpoint is an identity and policy object, not a locator.  The
//! transport, address, descriptor, and credential used to resolve it remain
//! private to the effect adapter.  Keeping the endpoint contract here makes
//! it possible for Resource API, Nix, and Provider code to share one strict
//! vocabulary without importing any runtime transport type.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use d2b_contracts_resource::v3::{
    ResourceRef,
    endpoint_binding::EndpointAttachmentKind,
    execution_policy::{BoundedText, BoundedToken, PrimitiveSpecError, redacted_debug},
};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, JsonSchema)]
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

    /// Whether this endpoint admits one more `attach` relationship while
    /// `live` attachments are outstanding.
    ///
    /// Capacity is counted on the endpoint the owner declared, so a second
    /// consumer reaching for the same stream is refused by the endpoint's
    /// own ceiling rather than by a directory it happens to share.
    pub const fn admits_attachment(&self, live: u16) -> bool {
        self.supported && live < self.max_attachments
    }
}

impl<'de> Deserialize<'de> for EndpointAttachmentPolicy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            supported: bool,
            #[serde(default)]
            max_attachments: u16,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.supported, wire.max_attachments).map_err(serde::de::Error::custom)
    }
}

/// The only fine-grained endpoint consumer policy.
#[derive(Clone, PartialEq, Eq, Default, Serialize, JsonSchema)]
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

    /// Borrow the operation allowlist.
    pub fn allowed_operations(&self) -> &[EndpointOperation] {
        &self.allowed_operations
    }

    /// Borrow the consumer allowlist.
    ///
    /// This is the endpoint OWNER's own subject allowlist: the set of
    /// consumers this endpoint admits, as opposed to the graph's
    /// authorization evidence. An `EndpointBinding` is admitted against
    /// both, and neither one is a directory the consumer may reach.
    pub fn allowed_subjects(&self) -> &[ResourceRef] {
        &self.allowed_subjects
    }

    /// Borrow the signed provider-component allowlist.
    pub fn allowed_provider_components(&self) -> &[BoundedToken] {
        &self.allowed_provider_components
    }

    /// Whether this policy admits `consumer` as a subject.
    ///
    /// An empty allowlist is the deliberately unconstrained policy
    /// ([`EndpointConsumerPolicy::unrestricted`]), so an empty list admits
    /// any subject rather than none - the same rule the other two
    /// allowlists follow, and the opposite of a "deny by default" reading
    /// that would make [`EndpointConsumerPolicy::default`] unusable.
    pub fn admits_subject(&self, consumer: &ResourceRef) -> bool {
        self.allowed_subjects.is_empty() || self.allowed_subjects.contains(consumer)
    }

    /// Whether this policy admits one signed provider component.
    pub fn admits_provider_component(&self, component: &BoundedToken) -> bool {
        self.allowed_provider_components.is_empty()
            || self.allowed_provider_components.contains(component)
    }

    /// Whether this policy admits `operation` on the endpoint.
    pub fn admits_operation(&self, operation: EndpointOperation) -> bool {
        self.allowed_operations.is_empty() || self.allowed_operations.contains(&operation)
    }

    /// The endpoint operation one attachment kind performs on it.
    ///
    /// The mapping is the endpoint's own vocabulary, so a binding cannot
    /// reach the endpoint through an operation the endpoint never declared
    /// by spelling a different attachment kind.
    pub const fn operation_for(attachment: EndpointAttachmentKind) -> EndpointOperation {
        match attachment {
            EndpointAttachmentKind::Connect => EndpointOperation::Resolve,
            EndpointAttachmentKind::Listen => EndpointOperation::Observe,
            EndpointAttachmentKind::Attach => EndpointOperation::Attach,
        }
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

/// What one `Endpoint` row publishes as `EndpointBinding` relationships
/// (KTD4: publication, authorization, and delivery are three owners).
///
/// Publication intent is a SEPARATE axis from [`EndpointConsumerPolicy`],
/// and the separation is the point:
///
/// - [`Self::None`] publishes no relationship at all. It is NOT the same
///   thing as an unconstrained consumer policy: an endpoint that admits
///   everyone but publishes nothing still delivers nothing, and an endpoint
///   that publishes a subject it does not authorize is refused rather than
///   repaired.
/// - [`Self::Named`] names the exact consumer subjects this endpoint
///   publishes a row for. The row is a function of that list alone, so the
///   set of relationships a consumer can ever reach is declared by the
///   endpoint OWNER and is not widened by a consumer asking for a different
///   slot, operation, or endpoint.
///
/// The default is [`Self::None`]: an endpoint that never declared publication
/// intent derives no relationship, so this field cannot widen any existing
/// row's reach.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EndpointBindingPublication {
    /// This endpoint publishes no `EndpointBinding` row.
    #[default]
    None,
    /// This endpoint publishes exactly one row per named consumer subject.
    Named(Vec<ResourceRef>),
}

impl EndpointBindingPublication {
    /// Build a publication intent over one unordered subject list.
    ///
    /// # Errors
    ///
    /// Refuses with [`EndpointSpecError::TooManyConsumerEntries`] past
    /// [`MAX_ENDPOINT_CONSUMER_ENTRIES`] and with
    /// [`EndpointSpecError::DuplicateConsumerEntry`] on a repeat, so the
    /// declared set is the same set a reader re-derives from the row.
    pub fn named(mut subjects: Vec<ResourceRef>) -> Result<Self, EndpointSpecError> {
        if subjects.len() > MAX_ENDPOINT_CONSUMER_ENTRIES {
            return Err(EndpointSpecError::TooManyConsumerEntries);
        }
        subjects.sort_by_key(ResourceRef::to_canonical_string);
        if has_duplicates(&subjects) {
            return Err(EndpointSpecError::DuplicateConsumerEntry);
        }
        Ok(Self::Named(subjects))
    }

    /// Whether this endpoint publishes no relationship at all.
    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// The subjects this endpoint publishes a row for.
    pub fn subjects(&self) -> &[ResourceRef] {
        match self {
            Self::None => &[],
            Self::Named(subjects) => subjects,
        }
    }

    /// Whether this endpoint publishes a row for `subject`.
    ///
    /// This is publication ALONE. Whether the relationship is admitted is
    /// the consumer policy's separate question
    /// ([`EndpointConsumerPolicy::admits_subject`]), and both must hold
    /// before a row exists.
    pub fn publishes_to(&self, subject: &ResourceRef) -> bool {
        self.subjects().contains(subject)
    }
}

redacted_debug!(EndpointBindingPublication);

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
    /// Which consumer subjects this endpoint publishes an `EndpointBinding`
    /// row for (KTD4). `None` is the default and is distinct from an
    /// unconstrained consumer policy.
    binding_publication: EndpointBindingPublication,
}

/// The opaque token that names ONE realization incarnation of one exact
/// endpoint (KTD8).
///
/// A socket path, an `(dev, ino)` pair, and a host error are all facts a
/// consumer must never read, so the token that proves "this is the same
/// realization I observed" is a domain-separated digest over the committed
/// identities instead of any of them. Two observations of one incarnation
/// produce the same token; a replaced socket, a new producer generation, a
/// new reconnect generation, or a changed service fingerprint produces a
/// different one, which is exactly the comparison a launch gate needs
/// (R17, R18).
///
/// # Ownership
///
/// U4 owns this TYPE and the comparison
/// ([`Self::same_incarnation`]); the display shapes' own evidence
/// contributes the private socket facts to the derivation in U5. What
/// crosses any projection here is the token value and nothing else, so the
/// refinement cannot widen what a consumer reads.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct RealizationIncarnation(BoundedToken);

impl RealizationIncarnation {
    /// Derive the incarnation token of one exact realization.
    ///
    /// Every input is a COMMITTED identity the endpoint owner already
    /// declared, so the derivation is a pure function of the row: the same
    /// row at the same generation with the same producer and reconnect
    /// evidence yields the same token, and any change to those yields a
    /// different one.
    ///
    /// # Errors
    ///
    /// Returns [`EndpointSpecError::Primitive`] when the derived value is
    /// not a bounded token.
    pub fn derive(
        zone: &str,
        endpoint_ref: &ResourceRef,
        endpoint_generation: u64,
        producer_uid: &str,
        producer_generation: u64,
        reconnect_generation: u64,
        fingerprint: Option<&str>,
    ) -> Result<Self, EndpointSpecError> {
        let mut frame = std::collections::BTreeMap::new();
        frame.insert(
            "domain".to_owned(),
            serde_json::Value::String(REALIZATION_INCARNATION_DOMAIN.to_owned()),
        );
        frame.insert("zone".to_owned(), serde_json::Value::String(zone.to_owned()));
        frame.insert(
            "endpoint".to_owned(),
            serde_json::Value::String(endpoint_ref.to_canonical_string()),
        );
        frame.insert(
            "endpointGeneration".to_owned(),
            serde_json::Value::from(endpoint_generation),
        );
        frame.insert(
            "producer".to_owned(),
            serde_json::Value::String(producer_uid.to_owned()),
        );
        frame.insert(
            "producerGeneration".to_owned(),
            serde_json::Value::from(producer_generation),
        );
        frame.insert(
            "reconnectGeneration".to_owned(),
            serde_json::Value::from(reconnect_generation),
        );
        frame.insert(
            "fingerprint".to_owned(),
            match fingerprint {
                Some(value) => serde_json::Value::String(value.to_owned()),
                None => serde_json::Value::Null,
            },
        );
        let canonical = d2b_contracts_resource::v3::canonical_json_bytes(&frame)
            .map_err(|_| EndpointSpecError::Primitive(PrimitiveSpecError::InvalidText))?;
        let digest =
            d2b_contracts_resource::v3::framed_canonical_digest(REALIZATION_INCARNATION_DOMAIN, &canonical);
        // The token grammar admits at most `MAX_BOUNDED_TOKEN_BYTES` bytes and
        // only lowercase alphanumerics and `-`, while the framed digest renders
        // as a `sha256:`-prefixed 64-hex string. Carrying the whole digest
        // would put the token permanently out of grammar and no endpoint would
        // ever publish an incarnation, so the derivation takes a fixed-length
        // window of it: 48 hex characters is 192 bits, which keeps the token
        // unpredictable and domain-separated (KTD8) and leaves the prefix
        // inside the bound.
        let window = digest
            .rsplit(':')
            .next()
            .expect("the framed digest is a prefixed hex string");
        let token = BoundedToken::parse(format!("incarnation-{}", &window[..48]))
            .map_err(|_| EndpointSpecError::Primitive(PrimitiveSpecError::InvalidText))?;
        Ok(Self(token))
    }

    /// Borrow the opaque token value.
    ///
    /// This is the ONLY thing any projection, log line, or comparison carries
    /// about the realization: no path, no device or inode pair, and no host
    /// error text.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Whether two observations name the SAME realization incarnation.
    ///
    /// Equality over the derived token is the whole comparison: an endpoint
    /// that was realized again, whose socket was rebound, or whose producer
    /// generation moved cannot compare equal to the earlier observation, so a
    /// launch gated on an older token cannot be replayed against the new one.
    pub fn same_incarnation(&self, other: &Self) -> bool {
        self == other
    }
}

redacted_debug!(RealizationIncarnation);

/// The domain tag framing one realization-incarnation derivation.
const REALIZATION_INCARNATION_DOMAIN: &str = "d2b:v3:endpoint-realization-incarnation";

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
            // An endpoint that constructed its spec without a builder call has
            // declared no publication intent, and publishes no relationship
            // (KTD4). `with_binding_publication` / `publishing_to` are the
            // only ways to widen this.
            binding_publication: EndpointBindingPublication::None,
        })
    }

    /// Declare which consumer subjects this endpoint publishes an
    /// `EndpointBinding` row for (KTD4).
    ///
    /// Publication intent is separate from authorization on purpose: a
    /// consumer policy admits subjects, this field publishes relationships,
    /// and a relationship exists only where both hold. An endpoint that
    /// declares a subject its own policy does not admit is refused by the
    /// derivation rather than delivered, so that is checked where the rows
    /// are derived and not only here.
    #[must_use]
    pub fn with_binding_publication(mut self, publication: EndpointBindingPublication) -> Self {
        self.binding_publication = publication;
        self
    }

    /// Declare a publication intent over one subject list.
    ///
    /// # Errors
    ///
    /// The same refusals [`EndpointBindingPublication::named`] reports.
    pub fn publishing_to(mut self, subjects: Vec<ResourceRef>) -> Result<Self, EndpointSpecError> {
        self.binding_publication = EndpointBindingPublication::named(subjects)?;
        Ok(self)
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

    /// Borrow fine-grained consumer policy.
    pub const fn consumer_policy(&self) -> &EndpointConsumerPolicy {
        &self.consumer_policy
    }

    /// Borrow the endpoint's own attachment capacity.
    ///
    /// The capacity is a property of the endpoint the owner declared, not
    /// of the consumer asking for it: an `EndpointBinding` that attaches is
    /// refused unless this policy supports attachments and the endpoint's
    /// simultaneous-attachment ceiling still has room.
    pub const fn attachment_policy(&self) -> &EndpointAttachmentPolicy {
        &self.attachment_policy
    }

    /// Return lifecycle behavior.
    pub const fn lifecycle_policy(&self) -> EndpointLifecyclePolicy {
        self.lifecycle_policy
    }

    /// Borrow the endpoint's own binding publication intent.
    ///
    /// This is the DECLARATION of which relationships this endpoint
    /// publishes; [`Self::consumer_policy`] is the authorization that admits
    /// them. `EndpointBinding` rows are derived from this field, never from
    /// the consumer policy alone and never from rows that already exist
    /// (R16).
    pub const fn binding_publication(&self) -> &EndpointBindingPublication {
        &self.binding_publication
    }

    /// Whether this endpoint publishes a relationship for `subject` AND
    /// authorizes it.
    ///
    /// Both halves are required: a published subject the owner does not
    /// authorize is a declaration this family cannot honor, and an authorized
    /// subject the owner does not publish is a relationship that does not
    /// exist.
    pub fn publishes_and_admits(&self, subject: &ResourceRef) -> bool {
        self.binding_publication.publishes_to(subject)
            && self.consumer_policy.admits_subject(subject)
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
            #[serde(default)]
            binding_publication: EndpointBindingPublication,
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
        .map(|spec| spec.with_binding_publication(wire.binding_publication))
        .map_err(serde::de::Error::custom)
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
    use d2b_contracts_resource::v3::resource_schema::canonical_json_bytes;
    use d2b_contracts_resource::v3::execution_policy::MAX_BOUNDED_TOKEN_BYTES;

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
            br#"{"attachmentPolicy":{"maxAttachments":0,"supported":false},"bindingPublication":"none","consumerPolicy":{},"endpointClass":"service","lifecyclePolicy":"recycle-with-producer","locality":"zone-local","producerRef":"Process/wayland-proxy","providerRef":"Provider/display-wayland","purpose":"wayland-control","transport":"opaque-carriage","visibility":"provider"}"#
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
    fn attachment_policy_round_trips_and_rejects_inconsistent_shapes() {
        let policy = EndpointAttachmentPolicy::new(true, 2).unwrap();
        let value = serde_json::to_value(policy).unwrap();
        assert_eq!(
            serde_json::from_value::<EndpointAttachmentPolicy>(value).unwrap(),
            policy
        );
        for (supported, max_attachments) in [
            (false, 1),
            (true, 0),
            (true, MAX_ENDPOINT_ATTACHMENTS + 1),
        ] {
            let value = serde_json::json!({
                "supported": supported,
                "maxAttachments": max_attachments,
            });
            assert!(
                serde_json::from_value::<EndpointAttachmentPolicy>(value).is_err(),
                "illegal attachment policy ({supported}, {max_attachments}) admitted"
            );
        }
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


    #[test]
    fn a_derived_incarnation_is_a_grammar_token_and_is_stable_for_one_realization() {
        let endpoint = ResourceRef::parse("Endpoint/compositor").unwrap();
        let derive = |generation: u64, producer_generation: u64, reconnect: u64| {
            RealizationIncarnation::derive(
                "dev",
                &endpoint,
                generation,
                "00000000000000000000000000000001",
                producer_generation,
                reconnect,
                Some("sha256:compositor"),
            )
        };
        let first = derive(3, 1, 0).expect("a realization derives an incarnation token");
        assert_eq!(
            first.as_str(),
            derive(3, 1, 0)
                .expect("the same realization derives the same token")
                .as_str(),
            "the derivation is a pure function of the committed facts"
        );
        assert!(
            first.as_str().len() <= MAX_BOUNDED_TOKEN_BYTES,
            "the token fits the grammar every projection has to spell it in"
        );
        assert!(
            BoundedToken::parse(first.as_str()).is_ok(),
            "the token is a token: a derivation that fell outside the grammar \
             would leave every endpoint publishing no incarnation at all"
        );
    }

    #[test]
    fn every_realization_fact_moves_the_incarnation_token() {
        let endpoint = ResourceRef::parse("Endpoint/compositor").unwrap();
        let other = ResourceRef::parse("Endpoint/proxy").unwrap();
        let baseline = RealizationIncarnation::derive(
            "dev",
            &endpoint,
            3,
            "00000000000000000000000000000001",
            1,
            0,
            Some("sha256:compositor"),
        )
        .unwrap();
        let moved = [
            RealizationIncarnation::derive("zone", &endpoint, 3, "00000000000000000000000000000001", 1, 0, Some("sha256:compositor")).unwrap(),
            RealizationIncarnation::derive("dev", &other, 3, "00000000000000000000000000000001", 1, 0, Some("sha256:compositor")).unwrap(),
            RealizationIncarnation::derive("dev", &endpoint, 4, "00000000000000000000000000000001", 1, 0, Some("sha256:compositor")).unwrap(),
            RealizationIncarnation::derive("dev", &endpoint, 3, "00000000000000000000000000000002", 1, 0, Some("sha256:compositor")).unwrap(),
            RealizationIncarnation::derive("dev", &endpoint, 3, "00000000000000000000000000000001", 2, 0, Some("sha256:compositor")).unwrap(),
            RealizationIncarnation::derive("dev", &endpoint, 3, "00000000000000000000000000000001", 1, 1, Some("sha256:compositor")).unwrap(),
            RealizationIncarnation::derive("dev", &endpoint, 3, "00000000000000000000000000000001", 1, 0, Some("sha256:proxy")).unwrap(),
            RealizationIncarnation::derive("dev", &endpoint, 3, "00000000000000000000000000000001", 1, 0, None).unwrap(),
        ];
        for candidate in moved {
            assert!(
                !candidate.same_incarnation(&baseline),
                "a changed realization fact must not compare as the same incarnation"
            );
        }
    }
}
