//! The shared semantic Service and Binding contract bases.
//!
//! This module tree owns the common Service/Binding base spec, status, and
//! schema contract for each frozen semantic pair in the D098 catalog of
//! `ADR-046-provider-model-and-packaging`. The catalog is owned here and not
//! by any initial implementation crate, so the bases are discoverable before
//! a Provider package is selected and no implementation can copy, fork,
//! weaken, or privately redefine them.
//!
//! The eight qualified semantic ResourceTypes these bases describe are
//! installed from signed Provider schemas. They are deliberately absent from
//! the standard ResourceType registry, which is closed.
//!
//! Two spellings appear here and are never interchangeable. The API
//! ResourceType is the dot-qualified name, for example
//! `audio.d2bus.org.AudioService`, and a ResourceRef appends `/<name>` to it.
//! The schema identity is the slash form, `<namespace>/<Type>/spec` and
//! `<namespace>/<Type>/status`.
//!
//! Scope of the base. A base layer here is the frozen top-level field set of
//! `spec` and of `status.resource` for one ResourceType, plus its schema
//! identity, version, and fingerprint. That is exactly the surface the
//! specification freezes as provider-neutral. Where the specification names a
//! base field but does not fix that field's interior member names or value
//! domains, this catalog carries the field name and declines to model the
//! interior, because a fabricated interior would be binding on every
//! implementation. See the module-level notes on each family.
//!
//! Redaction. Nothing here renders a caller-supplied value through `Debug` or
//! `Display`, and no type carries a path, uid, key, secret, locator, or
//! handle. Errors are closed discriminants.

use std::collections::BTreeSet;

use super::{
    identity::{ResourceTypeName, SchemaFingerprint},
    provider::{BindingTargetType, Exportability, ProjectionFactory, ProviderContractError},
    resource::ResourceSpec,
    resource_schema::{
        ObjectFieldSchema, ProviderExtensionRegistration, ResourceSchemaContract,
        ResourceSchemaError, SCHEMA_DOMAIN_TAG, SchemaVersion, canonical_digest,
    },
};

pub mod audio;
pub mod security_key;
pub mod telemetry;
pub mod usb;

/// The major component of the initial semantic base schema version.
///
/// The specification freezes the catalog contents but does not state a
/// version number for the semantic bases themselves. These are the first
/// published bases, so the initial version is `1.0`; a later change to any
/// frozen field set moves it under the ordinary schema-version rules.
pub const SEMANTIC_BASE_SCHEMA_MAJOR: u32 = 1;

/// The minor component of the initial semantic base schema version.
pub const SEMANTIC_BASE_SCHEMA_MINOR: u32 = 0;

/// The semantic projection-protocol version bound into a factory
/// fingerprint.
///
/// The specification requires the factory fingerprint to bind this value and
/// requires it to exclude Provider and adapter identity, but it does not fix
/// the value's spelling. This is the initial value for the initial bases.
pub const SEMANTIC_PROJECTION_PROTOCOL_VERSION: &str = "1.0";

/// The reserved base spec field naming the selected implementation.
pub const PROVIDER_REF_FIELD: &str = "providerRef";

/// The reserved base spec field carrying the provider-neutral update policy.
pub const UPDATE_POLICY_FIELD: &str = "updatePolicy";

/// One frozen semantic family in the D098 catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum SemanticFamily {
    /// The audio family.
    Audio,
    /// The security-key family.
    SecurityKey,
    /// The telemetry family.
    Telemetry,
    /// The USB family.
    Usb,
}

impl SemanticFamily {
    /// The qualified schema namespace this family's types are named under.
    pub const fn namespace(self) -> &'static str {
        match self {
            Self::Audio => "audio.d2bus.org",
            Self::SecurityKey => "security-key.d2bus.org",
            Self::Telemetry => "telemetry.d2bus.org",
            Self::Usb => "usb.d2bus.org",
        }
    }

    /// Borrow this family's frozen pair contract.
    pub fn contract(self) -> &'static SemanticPairContract {
        match self {
            Self::Audio => audio::contract(),
            Self::SecurityKey => security_key::contract(),
            Self::Telemetry => telemetry::contract(),
            Self::Usb => usb::contract(),
        }
    }
}

/// Whether a catalog member is the owner authority type or the local
/// consumer intent type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum SemanticRole {
    /// The `*Service` type: owner authority and consumer projection type.
    Service,
    /// The `*Binding` type: local consumer intent.
    Binding,
}

/// Which schema layer of one ResourceType a schema identity names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum SemanticLayer {
    /// The desired-state layer, `<namespace>/<Type>/spec`.
    Spec,
    /// The observed-state layer, `<namespace>/<Type>/status`.
    Status,
}

impl SemanticLayer {
    /// The canonical trailing segment of a schema identity.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spec => "spec",
            Self::Status => "status",
        }
    }
}

/// Reason a semantic catalog value could not be constructed or admitted.
///
/// Every variant is a closed reason. None carries a field value, a
/// reference, a path, or any other caller-supplied material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SemanticContractError {
    /// A base spec or status layer failed its frozen field-name schema.
    SchemaViolation,
    /// A projection spec carried a `spec.provider` extension. A Core-generated
    /// projection permits only `providerRef`, the semantic base and import
    /// fields, and ResourceImport ownership.
    ProjectionProviderExtensionForbidden,
    /// A supplied minimal-base fixture did not supply exactly the required
    /// base field names.
    MinimalBaseFieldSetMismatch,
    /// A minimal-base fixture supplied a reserved field the envelope owns.
    MinimalBaseReservedField,
    /// The reference does not name this catalog member's ResourceType.
    WrongResourceType,
    /// The specification does not fix this family's closed set of allowed
    /// same-Zone backing reference types, so a signed projection factory
    /// cannot be derived from the semantic base alone.
    BackingRefTypesUndetermined,
    /// A projection factory could not be constructed from the derived
    /// semantic binding.
    ProjectionFactoryInvalid,
}

impl SemanticContractError {
    /// The closed diagnostic label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SchemaViolation => "semantic-base-schema-violation",
            Self::ProjectionProviderExtensionForbidden => {
                "semantic-projection-provider-extension-forbidden"
            }
            Self::MinimalBaseFieldSetMismatch => "semantic-minimal-base-field-set-mismatch",
            Self::MinimalBaseReservedField => "semantic-minimal-base-reserved-field",
            Self::WrongResourceType => "semantic-wrong-resource-type",
            Self::BackingRefTypesUndetermined => "semantic-backing-ref-types-undetermined",
            Self::ProjectionFactoryInvalid => "semantic-projection-factory-invalid",
        }
    }
}

impl core::fmt::Display for SemanticContractError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for SemanticContractError {}

impl From<ResourceSchemaError> for SemanticContractError {
    fn from(_: ResourceSchemaError) -> Self {
        Self::SchemaViolation
    }
}

impl From<ProviderContractError> for SemanticContractError {
    fn from(_: ProviderContractError) -> Self {
        Self::ProjectionFactoryInvalid
    }
}

/// The slash-form schema identity of one base layer.
///
/// This is a schema identity only. It is never a ResourceType, never a
/// ResourceRef prefix, and never an alias for the dot-qualified API name.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemanticSchemaId {
    rendered: String,
}

impl SemanticSchemaId {
    fn new(namespace: &str, type_segment: &str, layer: SemanticLayer) -> Self {
        Self {
            rendered: format!("{namespace}/{type_segment}/{}", layer.as_str()),
        }
    }

    /// Borrow the canonical identity.
    pub fn as_str(&self) -> &str {
        &self.rendered
    }

    /// Render the canonical identity for an authorized encoding surface.
    pub fn to_canonical_string(&self) -> String {
        self.rendered.clone()
    }
}

impl core::fmt::Debug for SemanticSchemaId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SemanticSchemaId(<redacted>)")
    }
}

impl core::fmt::Display for SemanticSchemaId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SemanticSchemaId(<redacted>)")
    }
}

/// A closed top-level field-name set, validated without borrowing the
/// private validator of [`ObjectFieldSchema`].
fn validate_field_names<'a>(
    allowed: &BTreeSet<&'static str>,
    required: &BTreeSet<&'static str>,
    names: impl IntoIterator<Item = &'a str>,
) -> Result<(), SemanticContractError> {
    let names: BTreeSet<&str> = names.into_iter().collect();
    for name in &names {
        if !allowed.contains(name) {
            return Err(SemanticContractError::SchemaViolation);
        }
    }
    for name in required {
        if !names.contains(name) {
            return Err(SemanticContractError::SchemaViolation);
        }
    }
    Ok(())
}

/// One frozen base layer: its identity, version, field set, and fingerprint.
#[derive(Clone, PartialEq, Eq)]
pub struct SemanticLayerSchema {
    schema_id: SemanticSchemaId,
    version: SchemaVersion,
    allowed: BTreeSet<&'static str>,
    required: BTreeSet<&'static str>,
    fields: ObjectFieldSchema,
    fingerprint: SchemaFingerprint,
}

impl SemanticLayerSchema {
    fn build(
        namespace: &str,
        type_segment: &str,
        layer: SemanticLayer,
        allowed: &'static [&'static str],
        required: &'static [&'static str],
    ) -> Self {
        let schema_id = SemanticSchemaId::new(namespace, type_segment, layer);
        let version = SchemaVersion::new(SEMANTIC_BASE_SCHEMA_MAJOR, SEMANTIC_BASE_SCHEMA_MINOR)
            .expect("the catalog base schema version is a valid non-zero major version");
        let fields = ObjectFieldSchema::new(
            allowed.iter().map(|name| (*name).to_owned()),
            required.iter().map(|name| (*name).to_owned()),
        )
        .expect("a catalog field set is a valid closed object schema");
        let fingerprint = layer_fingerprint(&schema_id, version, allowed, required);
        Self {
            schema_id,
            version,
            allowed: allowed.iter().copied().collect(),
            required: required.iter().copied().collect(),
            fields,
            fingerprint,
        }
    }

    /// Borrow the slash-form schema identity.
    pub const fn schema_id(&self) -> &SemanticSchemaId {
        &self.schema_id
    }

    /// The frozen schema version.
    pub const fn version(&self) -> SchemaVersion {
        self.version
    }

    /// Borrow the frozen closed field set.
    pub const fn fields(&self) -> &ObjectFieldSchema {
        &self.fields
    }

    /// The frozen allowed top-level field names.
    pub fn allowed_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.allowed.iter().copied()
    }

    /// The frozen required top-level field names.
    pub fn required_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.required.iter().copied()
    }

    /// Admit a set of top-level field names against this frozen layer.
    pub fn validate_names<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), SemanticContractError> {
        validate_field_names(&self.allowed, &self.required, names)
    }

    /// Borrow the fingerprint of the canonical schema declaration.
    pub const fn fingerprint(&self) -> &SchemaFingerprint {
        &self.fingerprint
    }
}

impl core::fmt::Debug for SemanticLayerSchema {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SemanticLayerSchema(<redacted>)")
    }
}

fn layer_fingerprint(
    schema_id: &SemanticSchemaId,
    version: SchemaVersion,
    allowed: &'static [&'static str],
    required: &'static [&'static str],
) -> SchemaFingerprint {
    let allowed: BTreeSet<&str> = allowed.iter().copied().collect();
    let required: BTreeSet<&str> = required.iter().copied().collect();
    let declaration = serde_json::json!({
        "schemaId": schema_id.as_str(),
        "schemaVersion": version.to_canonical_string(),
        "additionalProperties": false,
        "allowed": allowed.iter().copied().collect::<Vec<_>>(),
        "required": required.iter().copied().collect::<Vec<_>>(),
    });
    let bytes = super::resource_schema::canonical_json_bytes(&declaration)
        .expect("a catalog schema declaration is canonicalizable");
    SchemaFingerprint::parse(canonical_digest(SCHEMA_DOMAIN_TAG, &bytes))
        .expect("a domain-separated SHA-256 digest is a valid schema fingerprint")
}

/// The frozen base contract for one catalog ResourceType.
#[derive(Clone, PartialEq, Eq)]
pub struct SemanticTypeContract {
    resource_type: ResourceTypeName,
    role: SemanticRole,
    spec: SemanticLayerSchema,
    status: SemanticLayerSchema,
}

impl SemanticTypeContract {
    fn build(
        namespace: &str,
        type_segment: &str,
        role: SemanticRole,
        spec_allowed: &'static [&'static str],
        spec_required: &'static [&'static str],
        status_allowed: &'static [&'static str],
        status_required: &'static [&'static str],
    ) -> Self {
        let resource_type = ResourceTypeName::parse(format!("{namespace}.{type_segment}"))
            .expect("a catalog ResourceType is a valid qualified name");
        Self {
            resource_type,
            role,
            spec: SemanticLayerSchema::build(
                namespace,
                type_segment,
                SemanticLayer::Spec,
                spec_allowed,
                spec_required,
            ),
            status: SemanticLayerSchema::build(
                namespace,
                type_segment,
                SemanticLayer::Status,
                status_allowed,
                status_required,
            ),
        }
    }

    /// Borrow the dot-qualified API ResourceType.
    pub const fn resource_type(&self) -> &ResourceTypeName {
        &self.resource_type
    }

    /// Whether this member is the Service or the Binding of its pair.
    pub const fn role(&self) -> SemanticRole {
        self.role
    }

    /// Borrow the frozen base spec layer.
    pub const fn spec(&self) -> &SemanticLayerSchema {
        &self.spec
    }

    /// Borrow the frozen base `status.resource` layer.
    pub const fn status(&self) -> &SemanticLayerSchema {
        &self.status
    }

    /// The frozen required base spec field names.
    ///
    /// This is exactly the field set the canonical minimal valid base Spec
    /// supplies, and every conformant implementation must accept it without
    /// a `spec.provider` extension.
    pub fn required_spec_fields(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.spec.required_names()
    }

    /// Build the resource-store schema contract for this ResourceType.
    ///
    /// `provider_extensions` carries the registrations of whichever Provider
    /// implementations are installed. Passing none yields the discoverable
    /// common base with no Provider package selected.
    pub fn schema_contract(
        &self,
        provider_extensions: impl IntoIterator<Item = ProviderExtensionRegistration>,
    ) -> Result<ResourceSchemaContract, SemanticContractError> {
        Ok(ResourceSchemaContract::new(
            self.resource_type.clone(),
            super::resource_schema::BaseSchemaBinding {
                spec: super::resource_schema::BaseSchemaIdentity {
                    version: self.spec.version,
                    fingerprint: self.spec.fingerprint.clone(),
                },
                status: super::resource_schema::BaseSchemaIdentity {
                    version: self.status.version,
                    fingerprint: self.status.fingerprint.clone(),
                },
            },
            self.spec.fields.clone(),
            self.status.fields.clone(),
            provider_extensions,
        )?)
    }

    /// Assemble the canonical minimal valid base Spec, without a
    /// `spec.provider` extension.
    ///
    /// `base_values` must supply exactly the required base field names other
    /// than `providerRef`, which the envelope owns. The catalog supplies the
    /// frozen field set; the caller supplies the values, because the
    /// specification does not fix an interior for every required field. See
    /// each family module for which interiors it leaves open.
    pub fn minimal_base_spec(
        &self,
        provider_ref: super::resource_ref::ResourceRef,
        base_values: super::resource_schema::CanonicalJsonObject,
    ) -> Result<ResourceSpec, SemanticContractError> {
        for key in base_values.keys() {
            if matches!(key, "provider" | PROVIDER_REF_FIELD | UPDATE_POLICY_FIELD) {
                return Err(SemanticContractError::MinimalBaseReservedField);
            }
        }
        let supplied: BTreeSet<&str> = base_values.keys().collect();
        let expected: BTreeSet<&str> = self
            .required_spec_fields()
            .filter(|name| *name != PROVIDER_REF_FIELD)
            .collect();
        if supplied != expected {
            return Err(SemanticContractError::MinimalBaseFieldSetMismatch);
        }
        ResourceSpec::new(Some(provider_ref), None, base_values, None)
            .map_err(|_| SemanticContractError::SchemaViolation)
    }
}

impl core::fmt::Debug for SemanticTypeContract {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SemanticTypeContract")
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

/// The semantic projection binding of one catalog pair.
///
/// This is the provider-neutral half of a signed D096 projection factory.
/// It carries the Service and Binding ResourceTypes, the closed backing and
/// target reference sets, the strict projection schema, and the two
/// fingerprints. It never carries Provider or adapter identity, which is why
/// the factory fingerprint is stable across a change of either.
#[derive(Clone, PartialEq, Eq)]
pub struct SemanticProjectionBinding {
    service_type: ResourceTypeName,
    binding_type: ResourceTypeName,
    allowed_backing_ref_types: Option<BTreeSet<ResourceTypeName>>,
    allowed_binding_target_ref_types: BTreeSet<BindingTargetType>,
    projection_allowed: BTreeSet<&'static str>,
    projection_required: BTreeSet<&'static str>,
    projection_spec_fields: ObjectFieldSchema,
    projection_schema_fingerprint: SchemaFingerprint,
    factory_fingerprint: SchemaFingerprint,
}

impl SemanticProjectionBinding {
    fn build(
        namespace: &str,
        service_type: ResourceTypeName,
        binding_type: ResourceTypeName,
        allowed_backing_ref_types: Option<&'static [&'static str]>,
        allowed_binding_target_ref_types: &[BindingTargetType],
        projection_allowed: &'static [&'static str],
        projection_required: &'static [&'static str],
    ) -> Self {
        let allowed_backing_ref_types = allowed_backing_ref_types.map(|names| {
            names
                .iter()
                .map(|name| {
                    ResourceTypeName::parse(*name)
                        .expect("a catalog backing ref type is a valid ResourceType")
                })
                .collect::<BTreeSet<_>>()
        });
        let allowed_binding_target_ref_types: BTreeSet<_> =
            allowed_binding_target_ref_types.iter().copied().collect();
        let projection_spec_fields = ObjectFieldSchema::new(
            projection_allowed.iter().map(|name| (*name).to_owned()),
            projection_required.iter().map(|name| (*name).to_owned()),
        )
        .expect("a catalog projection field set is a valid closed object schema");

        let projection_schema_id = SemanticSchemaId {
            rendered: format!("{namespace}/projection/spec"),
        };
        let version = SchemaVersion::new(SEMANTIC_BASE_SCHEMA_MAJOR, SEMANTIC_BASE_SCHEMA_MINOR)
            .expect("the catalog base schema version is a valid non-zero major version");
        let projection_schema_fingerprint = layer_fingerprint(
            &projection_schema_id,
            version,
            projection_allowed,
            projection_required,
        );
        let factory_fingerprint = factory_fingerprint(
            &service_type,
            &binding_type,
            allowed_backing_ref_types.as_ref(),
            &allowed_binding_target_ref_types,
            &projection_schema_fingerprint,
        );

        Self {
            service_type,
            binding_type,
            allowed_backing_ref_types,
            allowed_binding_target_ref_types,
            projection_allowed: projection_allowed.iter().copied().collect(),
            projection_required: projection_required.iter().copied().collect(),
            projection_spec_fields,
            projection_schema_fingerprint,
            factory_fingerprint,
        }
    }

    /// The owner authority and consumer projection ResourceType.
    pub const fn service_type(&self) -> &ResourceTypeName {
        &self.service_type
    }

    /// The local consumer intent ResourceType.
    pub const fn binding_type(&self) -> &ResourceTypeName {
        &self.binding_type
    }

    /// The closed same-Zone backing reference set the owner Service may name.
    ///
    /// `None` means the specification does not fix this family's set from the
    /// semantic base alone. It is never an empty permitted set.
    pub const fn allowed_backing_ref_types(&self) -> Option<&BTreeSet<ResourceTypeName>> {
        self.allowed_backing_ref_types.as_ref()
    }

    /// The closed target set a Binding of this pair may name.
    pub const fn allowed_binding_target_ref_types(&self) -> &BTreeSet<BindingTargetType> {
        &self.allowed_binding_target_ref_types
    }

    /// Borrow the strict projection schema field set.
    pub const fn projection_spec_fields(&self) -> &ObjectFieldSchema {
        &self.projection_spec_fields
    }

    /// The strict projection schema's allowed top-level field names.
    pub fn projection_allowed_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.projection_allowed.iter().copied()
    }

    /// The strict projection schema's required top-level field names.
    pub fn projection_required_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.projection_required.iter().copied()
    }

    /// Borrow the strict projection schema fingerprint.
    pub const fn projection_schema_fingerprint(&self) -> &SchemaFingerprint {
        &self.projection_schema_fingerprint
    }

    /// Borrow the semantic factory fingerprint.
    pub const fn factory_fingerprint(&self) -> &SchemaFingerprint {
        &self.factory_fingerprint
    }

    /// Derive the provider-neutral half of a signed projection factory.
    ///
    /// Fails with [`SemanticContractError::BackingRefTypesUndetermined`] when
    /// the specification does not fix this family's backing set, rather than
    /// substituting a plausible one.
    pub fn projection_factory(&self) -> Result<ProjectionFactory, SemanticContractError> {
        let backing = self
            .allowed_backing_ref_types
            .as_ref()
            .ok_or(SemanticContractError::BackingRefTypesUndetermined)?;
        Ok(ProjectionFactory::new(
            self.service_type.clone(),
            self.binding_type.clone(),
            backing.iter().cloned(),
            self.allowed_binding_target_ref_types.iter().copied(),
            self.projection_schema_fingerprint.clone(),
            self.factory_fingerprint.clone(),
            Exportability::ExplicitExport,
        )?)
    }

    /// Admit a Core-generated projection Service spec.
    ///
    /// A projection permits only `providerRef`, the semantic base and import
    /// fields, and ResourceImport ownership. A `spec.provider` extension is
    /// rejected: Core never synthesizes one and never copies a remote one.
    pub fn validate_projection_spec(
        &self,
        spec: &ResourceSpec,
    ) -> Result<(), SemanticContractError> {
        if spec.provider().is_some() {
            return Err(SemanticContractError::ProjectionProviderExtensionForbidden);
        }
        let mut names: Vec<&str> = spec.base().keys().collect();
        if spec.provider_ref().is_some() {
            names.push(PROVIDER_REF_FIELD);
        }
        if spec.update_policy().is_some() {
            names.push(UPDATE_POLICY_FIELD);
        }
        validate_field_names(&self.projection_allowed, &self.projection_required, names)
    }
}

impl core::fmt::Debug for SemanticProjectionBinding {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SemanticProjectionBinding")
            .field(
                "backing_ref_types_determined",
                &self.allowed_backing_ref_types.is_some(),
            )
            .field(
                "binding_target_types",
                &self.allowed_binding_target_ref_types.len(),
            )
            .finish_non_exhaustive()
    }
}

fn factory_fingerprint(
    service_type: &ResourceTypeName,
    binding_type: &ResourceTypeName,
    allowed_backing_ref_types: Option<&BTreeSet<ResourceTypeName>>,
    allowed_binding_target_ref_types: &BTreeSet<BindingTargetType>,
    projection_schema_fingerprint: &SchemaFingerprint,
) -> SchemaFingerprint {
    let backing: Option<Vec<String>> = allowed_backing_ref_types.map(|set| {
        set.iter()
            .map(ResourceTypeName::to_canonical_string)
            .collect()
    });
    let targets: Vec<&str> = allowed_binding_target_ref_types
        .iter()
        .map(|target| match target {
            BindingTargetType::Guest => "guest",
            BindingTargetType::User => "user",
            BindingTargetType::Zone => "zone",
        })
        .collect();
    let declaration = serde_json::json!({
        "serviceType": service_type.to_canonical_string(),
        "bindingType": binding_type.to_canonical_string(),
        "allowedBackingRefTypes": backing,
        "allowedBindingTargetRefTypes": targets,
        "projectionSchemaFingerprint": projection_schema_fingerprint.as_str(),
        "projectionProtocolVersion": SEMANTIC_PROJECTION_PROTOCOL_VERSION,
    });
    let bytes = super::resource_schema::canonical_json_bytes(&declaration)
        .expect("a catalog factory declaration is canonicalizable");
    SchemaFingerprint::parse(canonical_digest(SCHEMA_DOMAIN_TAG, &bytes))
        .expect("a domain-separated SHA-256 digest is a valid schema fingerprint")
}

/// The frozen contract for one semantic Service and Binding pair.
#[derive(Clone, PartialEq, Eq)]
pub struct SemanticPairContract {
    family: SemanticFamily,
    service: SemanticTypeContract,
    binding: SemanticTypeContract,
    projection: SemanticProjectionBinding,
}

/// The per-family declaration a catalog module supplies.
pub(crate) struct SemanticPairDeclaration {
    pub(crate) family: SemanticFamily,
    pub(crate) service_type_segment: &'static str,
    pub(crate) binding_type_segment: &'static str,
    pub(crate) service_spec_allowed: &'static [&'static str],
    pub(crate) service_spec_required: &'static [&'static str],
    pub(crate) service_status_allowed: &'static [&'static str],
    pub(crate) binding_spec_allowed: &'static [&'static str],
    pub(crate) binding_spec_required: &'static [&'static str],
    pub(crate) binding_status_allowed: &'static [&'static str],
    pub(crate) allowed_backing_ref_types: Option<&'static [&'static str]>,
    pub(crate) allowed_binding_target_ref_types: &'static [BindingTargetType],
    pub(crate) projection_spec_allowed: &'static [&'static str],
    pub(crate) projection_spec_required: &'static [&'static str],
}

impl SemanticPairContract {
    pub(crate) fn build(declaration: &SemanticPairDeclaration) -> Self {
        let namespace = declaration.family.namespace();
        let service = SemanticTypeContract::build(
            namespace,
            declaration.service_type_segment,
            SemanticRole::Service,
            declaration.service_spec_allowed,
            declaration.service_spec_required,
            declaration.service_status_allowed,
            &[],
        );
        let binding = SemanticTypeContract::build(
            namespace,
            declaration.binding_type_segment,
            SemanticRole::Binding,
            declaration.binding_spec_allowed,
            declaration.binding_spec_required,
            declaration.binding_status_allowed,
            &[],
        );
        let projection = SemanticProjectionBinding::build(
            namespace,
            service.resource_type.clone(),
            binding.resource_type.clone(),
            declaration.allowed_backing_ref_types,
            declaration.allowed_binding_target_ref_types,
            declaration.projection_spec_allowed,
            declaration.projection_spec_required,
        );
        Self {
            family: declaration.family,
            service,
            binding,
            projection,
        }
    }

    /// The family this pair belongs to.
    pub const fn family(&self) -> SemanticFamily {
        self.family
    }

    /// Borrow the owner authority and projection ResourceType contract.
    pub const fn service(&self) -> &SemanticTypeContract {
        &self.service
    }

    /// Borrow the local consumer intent ResourceType contract.
    pub const fn binding(&self) -> &SemanticTypeContract {
        &self.binding
    }

    /// Borrow the semantic projection binding.
    pub const fn projection(&self) -> &SemanticProjectionBinding {
        &self.projection
    }

    /// Admit a Binding's `serviceRef` and consuming target.
    ///
    /// Both must be same-Zone, which the caller establishes by resolving them
    /// in the Binding's Zone before calling. This admits only the type half:
    /// `serviceRef` must name this pair's Service ResourceType, and the target
    /// must be in this pair's closed allowed target set.
    pub fn admit_binding_refs(
        &self,
        service_ref: &super::resource_ref::ResourceRef,
        target: BindingTargetType,
    ) -> Result<(), SemanticContractError> {
        if service_ref.resource_type() != &self.service.resource_type {
            return Err(SemanticContractError::WrongResourceType);
        }
        if !self
            .projection
            .allowed_binding_target_ref_types
            .contains(&target)
        {
            return Err(SemanticContractError::WrongResourceType);
        }
        Ok(())
    }

    /// Admit a `ResourceExport.resourceRef`.
    ///
    /// It must target the owner Service, never a `Device`, an `Endpoint`, or
    /// a `*Binding`.
    pub fn admit_export_target(
        &self,
        resource_ref: &super::resource_ref::ResourceRef,
    ) -> Result<(), SemanticContractError> {
        if resource_ref.resource_type() == &self.service.resource_type {
            Ok(())
        } else {
            Err(SemanticContractError::WrongResourceType)
        }
    }
}

impl core::fmt::Debug for SemanticPairContract {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SemanticPairContract")
            .field("family", &self.family)
            .finish_non_exhaustive()
    }
}

/// The complete frozen catalog, in family order.
pub fn catalog() -> [&'static SemanticPairContract; 4] {
    [
        audio::contract(),
        security_key::contract(),
        telemetry::contract(),
        usb::contract(),
    ]
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::v3::{
        ResourceName,
        resource_ref::ResourceRef,
        resource_schema::{
            CanonicalJsonObject, CanonicalJsonValue, ExtensionSchemaId, ExtensionSchemaLayer,
        },
    };

    /// A `Provider/<name>` reference.
    pub(crate) fn provider_ref(name: &str) -> ResourceRef {
        ResourceRef::new(
            ResourceTypeName::parse("Provider").expect("Provider is a standard ResourceType"),
            ResourceName::parse(name).expect("a test provider name is a valid resource name"),
        )
    }

    /// Parse a canonical JSON object literal.
    pub(crate) fn object(json: &str) -> CanonicalJsonObject {
        CanonicalJsonObject::parse(json.as_bytes()).expect("a test fixture is canonical JSON")
    }

    /// Assert that the canonical minimal base Spec is accepted with no
    /// `spec.provider`, and that it survives a strict serde and canonical
    /// JSON round trip unchanged.
    pub(crate) fn assert_minimal_base_round_trips(member: &SemanticTypeContract, base: &str) {
        let contract = member
            .schema_contract(std::iter::empty())
            .expect("the base contract builds with no Provider installed");
        let spec = member
            .minimal_base_spec(provider_ref("some-provider"), object(base))
            .expect("the minimal fixture supplies exactly the required base fields");
        assert!(spec.provider().is_none());
        contract
            .validate_minimal_base_spec(&spec)
            .expect("the canonical minimal base is accepted without a Provider extension");

        let encoded = serde_json::to_vec(&spec).expect("a spec serializes");
        let decoded: ResourceSpec = serde_json::from_slice(&encoded).expect("a spec deserializes");
        assert_eq!(
            spec.canonical_bytes().expect("canonical bytes"),
            decoded.canonical_bytes().expect("canonical bytes")
        );
        contract
            .validate_minimal_base_spec(&decoded)
            .expect("the round-tripped minimal base is still accepted");
    }

    /// The Provider-observable base surface of one catalog member, recorded
    /// while a specific Provider extension is the installed implementation.
    ///
    /// Everything a downstream consumer can see of the common base is in
    /// here: both schema identities, both versions, both frozen field sets,
    /// both base fingerprints, the semantic factory fingerprint recomputed
    /// from its declared inputs, and the canonical bytes of the identical
    /// minimal base fixture after that installed contract admitted it.
    #[derive(Debug, PartialEq, Eq)]
    pub(crate) struct ObservedBase {
        spec_schema_id: String,
        status_schema_id: String,
        spec_version: String,
        status_version: String,
        spec_allowed: Vec<&'static str>,
        spec_required: Vec<&'static str>,
        status_allowed: Vec<&'static str>,
        status_required: Vec<&'static str>,
        spec_fingerprint: String,
        status_fingerprint: String,
        projection_schema_fingerprint: String,
        factory_fingerprint: String,
        minimal_base_bytes: Vec<u8>,
        enforced_base_probes: Vec<(String, String)>,
    }

    /// The Provider-specific settings field each installed implementation
    /// registers under `spec.provider`, and a name no implementation
    /// registers at all. Every observation probes all three, so a base that
    /// admitted one implementation's detail - or admitted an arbitrary extra
    /// field for one implementation and not the other - moves a probe.
    const PROBE_EXTRA_FIELDS: [&str; 3] = [
        INITIAL_EXTENSION_FIELD,
        ALTERNATE_EXTENSION_FIELD,
        "unregisteredForeignField",
    ];

    /// The settings field the initial implementation registers.
    const INITIAL_EXTENSION_FIELD: &str = "initialImplementationDetail";

    /// The settings field the alternate implementation registers.
    const ALTERNATE_EXTENSION_FIELD: &str = "alternateImplementationDetail";

    /// Map the base spec schema the installed contract actually enforces.
    ///
    /// The frozen field set is not readable from the built contract, so it is
    /// probed instead: the minimal fixture plus one extra field name maps the
    /// enforced allowed set, and the fixture minus one field maps the
    /// enforced required set. An implementation that widened or narrowed the
    /// common base for itself moves at least one probe outcome, which a
    /// comparison of only the accepted fixture would miss.
    fn probe_enforced_base(
        contract: &ResourceSchemaContract,
        member: &SemanticTypeContract,
        provider_ref: &ResourceRef,
        base: &CanonicalJsonObject,
    ) -> Vec<(String, String)> {
        let mut probes = Vec::new();
        let mut candidates: Vec<String> =
            member.spec().allowed_names().map(str::to_owned).collect();
        candidates.extend(PROBE_EXTRA_FIELDS.iter().map(|name| (*name).to_owned()));
        candidates.sort();
        candidates.dedup();

        for candidate in &candidates {
            let mut values = base.clone().into_inner();
            values.insert(candidate.clone(), CanonicalJsonValue::Bool(true));
            probes.push((
                format!("plus:{candidate}"),
                probe_outcome(
                    contract,
                    provider_ref,
                    CanonicalJsonObject::from_inner(values),
                ),
            ));
        }

        let present: Vec<String> = base.keys().map(str::to_owned).collect();
        for absent in &present {
            let mut values = base.clone().into_inner();
            values.remove(absent);
            probes.push((
                format!("minus:{absent}"),
                probe_outcome(
                    contract,
                    provider_ref,
                    CanonicalJsonObject::from_inner(values),
                ),
            ));
        }
        probes
    }

    /// The closed outcome label of one base-schema probe.
    fn probe_outcome(
        contract: &ResourceSchemaContract,
        provider_ref: &ResourceRef,
        values: CanonicalJsonObject,
    ) -> String {
        let spec = match ResourceSpec::new(Some(provider_ref.clone()), None, values, None) {
            Ok(spec) => spec,
            Err(_) => return "spec-rejected".to_owned(),
        };
        match contract.validate_minimal_base_spec(&spec) {
            Ok(()) => "accepted".to_owned(),
            Err(error) => format!("rejected:{error:?}"),
        }
    }

    /// Register a distinct Provider extension for `member`, then observe the
    /// common base through the contract that Provider installation produced.
    ///
    /// The extension carries a Provider-specific settings field so the two
    /// installations are genuinely different registrations, not the same
    /// value under two names.
    fn observe_base_with_provider_installed(
        pair: &SemanticPairContract,
        member: &SemanticTypeContract,
        base: &str,
        provider: &str,
        extension_field: &'static str,
    ) -> (ResourceSchemaContract, ObservedBase) {
        let provider_ref = provider_ref(provider);
        let provider_name =
            ResourceName::parse(provider).expect("a test provider name is a valid resource name");
        let registration = ProviderExtensionRegistration {
            provider_ref: provider_ref.clone(),
            spec_schema_id: ExtensionSchemaId::new(
                provider_name.clone(),
                member.resource_type().clone(),
                ExtensionSchemaLayer::Spec,
            ),
            spec_schema_version: SchemaVersion::new(1, 0).expect("1.0 is a valid schema version"),
            spec_settings: ObjectFieldSchema::new(
                [extension_field.to_owned()],
                std::iter::empty::<String>(),
            )
            .expect("a single-field extension schema is valid"),
            status_schema_id: ExtensionSchemaId::new(
                provider_name,
                member.resource_type().clone(),
                ExtensionSchemaLayer::Status,
            ),
            status_schema_version: SchemaVersion::new(1, 0).expect("1.0 is a valid schema version"),
            status_details: ObjectFieldSchema::new(
                [extension_field.to_owned()],
                std::iter::empty::<String>(),
            )
            .expect("a single-field extension schema is valid"),
        };

        let contract = member
            .schema_contract([registration])
            .expect("the common base admits an implementation's extension registration");

        let spec = member
            .minimal_base_spec(provider_ref.clone(), object(base))
            .expect("the identical fixture supplies the required base fields");
        assert!(spec.provider().is_none());
        contract
            .validate_minimal_base_spec(&spec)
            .expect("the installed implementation admits the identical base fixture");
        let enforced_base_probes =
            probe_enforced_base(&contract, member, &provider_ref, spec.base());

        let observed = ObservedBase {
            spec_schema_id: member.spec().schema_id().to_canonical_string(),
            status_schema_id: member.status().schema_id().to_canonical_string(),
            spec_version: member.spec().version().to_canonical_string(),
            status_version: member.status().version().to_canonical_string(),
            spec_allowed: member.spec().allowed_names().collect(),
            spec_required: member.spec().required_names().collect(),
            status_allowed: member.status().allowed_names().collect(),
            status_required: member.status().required_names().collect(),
            spec_fingerprint: member.spec().fingerprint().as_str().to_owned(),
            status_fingerprint: member.status().fingerprint().as_str().to_owned(),
            projection_schema_fingerprint: pair
                .projection()
                .projection_schema_fingerprint()
                .as_str()
                .to_owned(),
            factory_fingerprint: super::factory_fingerprint(
                pair.projection().service_type(),
                pair.projection().binding_type(),
                pair.projection().allowed_backing_ref_types(),
                pair.projection().allowed_binding_target_ref_types(),
                pair.projection().projection_schema_fingerprint(),
            )
            .as_str()
            .to_owned(),
            minimal_base_bytes: spec.base().to_canonical_bytes(),
            enforced_base_probes,
        };
        (contract, observed)
    }

    /// Prove the base is genuinely Provider-neutral.
    ///
    /// Two different implementations are installed in turn - each with its
    /// own registered `spec.provider` / `status.provider` extension - and the
    /// entire Provider-observable base surface is captured under each. The
    /// two observations must be equal: same schema identities, same versions,
    /// same frozen field sets, same base and factory fingerprints, and the
    /// same canonical bytes for the identical minimal base fixture. A base
    /// shaped around one implementation moves at least one of those.
    ///
    /// The comparison is guarded against degenerating into "a constant equals
    /// itself" by two negative controls. The two installed contracts must
    /// differ from each other, which shows the Provider identity really did
    /// reach the object under test; and the fingerprint functions must move
    /// when one of their declared inputs moves, which shows a fingerprint
    /// comparison is capable of failing at all.
    pub(crate) fn assert_base_is_provider_neutral(
        pair: &SemanticPairContract,
        service_base: &str,
        binding_base: &str,
        initial_provider: &str,
        alternate_provider: &str,
    ) {
        assert_ne!(initial_provider, alternate_provider);
        for (member, base) in [
            (pair.service(), service_base),
            (pair.binding(), binding_base),
        ] {
            let (initial_contract, initial_observed) = observe_base_with_provider_installed(
                pair,
                member,
                base,
                initial_provider,
                INITIAL_EXTENSION_FIELD,
            );
            let (alternate_contract, alternate_observed) = observe_base_with_provider_installed(
                pair,
                member,
                base,
                alternate_provider,
                ALTERNATE_EXTENSION_FIELD,
            );

            // Negative control: the two installations are genuinely
            // different, so an equal observation below is a claim about the
            // base and not an artifact of comparing one value with itself.
            assert_ne!(
                initial_contract, alternate_contract,
                "the two implementations must install distinct contracts"
            );

            assert_eq!(
                initial_observed, alternate_observed,
                "the observable base must not move when the implementation changes"
            );
        }

        // Negative control on the fingerprint functions themselves: each
        // moves when one of its declared inputs moves. Neither takes a
        // Provider or adapter identity as an input, which is why the
        // observations above are equal across implementations.
        const FIELDS: &[&str] = &["alpha"];
        const OTHER_FIELDS: &[&str] = &["beta"];
        let schema_id = SemanticSchemaId::new("probe.d2bus.org", "Probe", SemanticLayer::Spec);
        let version = SchemaVersion::new(1, 0).expect("1.0 is a valid schema version");
        assert_ne!(
            layer_fingerprint(&schema_id, version, FIELDS, &[]),
            layer_fingerprint(&schema_id, version, OTHER_FIELDS, &[]),
            "a layer fingerprint must move when its field set moves"
        );
        assert_ne!(
            super::factory_fingerprint(
                pair.projection().service_type(),
                pair.projection().binding_type(),
                pair.projection().allowed_backing_ref_types(),
                pair.projection().allowed_binding_target_ref_types(),
                pair.projection().projection_schema_fingerprint(),
            ),
            super::factory_fingerprint(
                pair.projection().service_type(),
                pair.projection().binding_type(),
                pair.projection().allowed_backing_ref_types(),
                pair.projection().allowed_binding_target_ref_types(),
                &layer_fingerprint(&schema_id, version, OTHER_FIELDS, &[]),
            ),
            "a factory fingerprint must move when its projection schema moves"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::{
        ResourceName,
        resource_ref::ResourceRef,
        resource_schema::{CanonicalJsonObject, ExtensionSchemaId, ExtensionSchemaLayer},
    };

    fn provider_ref(name: &str) -> ResourceRef {
        ResourceRef::new(
            ResourceTypeName::parse("Provider").unwrap(),
            ResourceName::parse(name).unwrap(),
        )
    }

    fn object(json: &str) -> CanonicalJsonObject {
        CanonicalJsonObject::parse(json.as_bytes()).unwrap()
    }

    /// Exact names: the eight qualified ResourceTypes are the frozen D098 set,
    /// in the dot-qualified API spelling.
    #[test]
    fn the_catalog_names_exactly_the_eight_frozen_resource_types() {
        let mut names: Vec<String> = Vec::new();
        for pair in catalog() {
            names.push(pair.service().resource_type().to_canonical_string());
            names.push(pair.binding().resource_type().to_canonical_string());
        }
        names.sort();
        assert_eq!(
            names,
            vec![
                "audio.d2bus.org.AudioBinding".to_owned(),
                "audio.d2bus.org.AudioService".to_owned(),
                "security-key.d2bus.org.SecurityKeyBinding".to_owned(),
                "security-key.d2bus.org.SecurityKeyService".to_owned(),
                "telemetry.d2bus.org.TelemetryBinding".to_owned(),
                "telemetry.d2bus.org.TelemetryService".to_owned(),
                "usb.d2bus.org.UsbBinding".to_owned(),
                "usb.d2bus.org.UsbService".to_owned(),
            ]
        );
    }

    /// Exact names: the schema identities are the slash form and are never
    /// confused with the dot-qualified ResourceType.
    #[test]
    fn schema_identities_use_the_slash_form_and_the_api_type_uses_the_dot_form() {
        for pair in catalog() {
            for member in [pair.service(), pair.binding()] {
                let resource_type = member.resource_type().to_canonical_string();
                let (namespace, segment) = resource_type.split_once(".d2bus.org.").unwrap();
                assert_eq!(
                    member.spec().schema_id().as_str(),
                    format!("{namespace}.d2bus.org/{segment}/spec")
                );
                assert_eq!(
                    member.status().schema_id().as_str(),
                    format!("{namespace}.d2bus.org/{segment}/status")
                );
                assert!(!member.spec().schema_id().as_str().contains(".d2bus.org."));
            }
        }
    }

    /// Rejection of every implementation-qualified and former `*State` alias.
    #[test]
    fn no_implementation_qualified_or_state_alias_is_registered() {
        let rejected = [
            "audio-pipewire.d2bus.org.AudioService",
            "audio-pipewire.d2bus.org.AudioBinding",
            "audio.d2bus.org.AudioState",
            "device-security-key.d2bus.org.SecurityKeyService",
            "security-key.d2bus.org.SecurityKeyState",
            "observability-otel.d2bus.org.TelemetryService",
            "telemetry.d2bus.org.TelemetryState",
            "device-usbip.d2bus.org.UsbService",
            "usb.d2bus.org.UsbState",
        ];
        let registered: BTreeSet<String> = catalog()
            .iter()
            .flat_map(|pair| {
                [
                    pair.service().resource_type().to_canonical_string(),
                    pair.binding().resource_type().to_canonical_string(),
                ]
            })
            .collect();
        for alias in rejected {
            assert!(
                !registered.contains(alias),
                "an alias must not resolve through the catalog"
            );
        }
    }

    /// Common base discoverability with no Provider package installed.
    #[test]
    fn every_base_contract_builds_with_no_provider_installed() {
        for pair in catalog() {
            for member in [pair.service(), pair.binding()] {
                let contract = member.schema_contract(std::iter::empty()).unwrap();
                assert_eq!(contract.resource_type(), member.resource_type());
                assert!(member.spec().fingerprint().as_str().starts_with("sha256:"));
                assert_eq!(member.spec().version().to_canonical_string(), "1.0");
            }
        }
    }

    /// Owner versus projection discrimination: the projection field set is a
    /// strict subset of the owner base spec field set.
    #[test]
    fn the_projection_field_set_is_a_strict_subset_of_the_service_base() {
        for pair in catalog() {
            let base: BTreeSet<&str> = pair.service().spec().allowed_names().collect();
            let projection: BTreeSet<&str> = pair.projection().projection_allowed_names().collect();
            assert!(projection.is_subset(&base));
            assert!(projection.len() < base.len());
        }
    }

    /// Core projection rejection of `spec.provider`.
    #[test]
    fn a_core_projection_rejects_a_provider_extension() {
        let pair = SemanticFamily::SecurityKey.contract();
        let extension = crate::v3::resource::ProviderSpecExtension::new(
            ExtensionSchemaId::new(
                ResourceName::parse("device-security-key").unwrap(),
                pair.service().resource_type().clone(),
                ExtensionSchemaLayer::Spec,
            ),
            SchemaVersion::new(1, 0).unwrap(),
            CanonicalJsonObject::empty(),
        )
        .unwrap();
        let spec = ResourceSpec::new(
            Some(provider_ref("device-security-key")),
            None,
            object(r#"{"mode":"projection"}"#),
            Some(extension),
        )
        .unwrap();
        assert_eq!(
            pair.projection().validate_projection_spec(&spec),
            Err(SemanticContractError::ProjectionProviderExtensionForbidden)
        );
    }

    /// Implementation-detail rejection: an unknown base spec field is denied
    /// on every catalog member.
    #[test]
    fn an_implementation_detail_is_rejected_from_every_base_spec() {
        for pair in catalog() {
            for member in [pair.service(), pair.binding()] {
                let contract = member.schema_contract(std::iter::empty()).unwrap();
                let spec = ResourceSpec::new(
                    Some(provider_ref("some-provider")),
                    None,
                    object(r#"{"pipeWireNodeAlias":"capture-0"}"#),
                    None,
                )
                .unwrap();
                assert!(contract.validate_minimal_base_spec(&spec).is_err());
            }
        }
    }

    /// Semantic factory-fingerprint stability under a Provider or adapter
    /// identity change: the fingerprint has no Provider input at all, so two
    /// distinct implementations derive byte-identical factory material.
    #[test]
    fn the_factory_fingerprint_is_independent_of_provider_identity() {
        for pair in catalog() {
            let first = pair.projection().factory_fingerprint().clone();
            // Recomputing from the same semantic inputs while a different
            // Provider or adapter is installed cannot change the value,
            // because no Provider or adapter identity is an input.
            let second = factory_fingerprint(
                pair.projection().service_type(),
                pair.projection().binding_type(),
                pair.projection().allowed_backing_ref_types(),
                pair.projection().allowed_binding_target_ref_types(),
                pair.projection().projection_schema_fingerprint(),
            );
            assert_eq!(first, second);
        }
    }

    /// Same-Zone refs and targets: a Binding may not name a foreign Service
    /// type or a target outside its closed set.
    #[test]
    fn binding_refs_and_targets_are_admitted_against_the_frozen_sets() {
        let audio = SemanticFamily::Audio.contract();
        let good = ResourceRef::new(
            audio.service().resource_type().clone(),
            ResourceName::parse("host-audio").unwrap(),
        );
        assert_eq!(
            audio.admit_binding_refs(&good, BindingTargetType::Guest),
            Ok(())
        );
        assert_eq!(
            audio.admit_binding_refs(&good, BindingTargetType::Zone),
            Err(SemanticContractError::WrongResourceType)
        );
        let wrong = ResourceRef::new(
            SemanticFamily::Usb
                .contract()
                .service()
                .resource_type()
                .clone(),
            ResourceName::parse("host-audio").unwrap(),
        );
        assert_eq!(
            audio.admit_binding_refs(&wrong, BindingTargetType::Guest),
            Err(SemanticContractError::WrongResourceType)
        );
    }

    /// No Device, Endpoint, or Binding projection: an export must target the
    /// owner Service.
    #[test]
    fn an_export_targets_only_the_owner_service() {
        for pair in catalog() {
            let name = ResourceName::parse("owner").unwrap();
            assert_eq!(
                pair.admit_export_target(&ResourceRef::new(
                    pair.service().resource_type().clone(),
                    name.clone()
                )),
                Ok(())
            );
            for rejected in ["Device", "Endpoint"] {
                assert_eq!(
                    pair.admit_export_target(&ResourceRef::new(
                        ResourceTypeName::parse(rejected).unwrap(),
                        name.clone()
                    )),
                    Err(SemanticContractError::WrongResourceType)
                );
            }
            assert_eq!(
                pair.admit_export_target(&ResourceRef::new(
                    pair.binding().resource_type().clone(),
                    name
                )),
                Err(SemanticContractError::WrongResourceType)
            );
        }
    }

    /// No Device, Endpoint, or Binding projection: an import materializes one
    /// same-qualified-type local projection Service and nothing else.
    #[test]
    fn a_projection_is_the_same_qualified_service_type_and_never_another_type() {
        for pair in catalog() {
            let projection = pair.projection();
            assert_eq!(projection.service_type(), pair.service().resource_type());
            assert_ne!(projection.service_type(), projection.binding_type());
            for other in ["Device", "Endpoint"] {
                assert_ne!(
                    projection.service_type().as_str(),
                    other,
                    "a projection is never a standard backing type"
                );
            }
        }
    }

    /// Common fields only under `status.resource`; implementation observation
    /// only under `status.provider`. A registered Provider extension may not
    /// shadow a common status field.
    #[test]
    fn a_provider_status_extension_may_not_shadow_a_common_status_field() {
        let pair = SemanticFamily::Usb.contract();
        let provider = provider_ref("device-usbip");
        let shadowing = ProviderExtensionRegistration {
            provider_ref: provider.clone(),
            spec_schema_id: ExtensionSchemaId::new(
                ResourceName::parse("device-usbip").unwrap(),
                pair.service().resource_type().clone(),
                ExtensionSchemaLayer::Spec,
            ),
            spec_schema_version: SchemaVersion::new(1, 0).unwrap(),
            spec_settings: ObjectFieldSchema::empty(),
            status_schema_id: ExtensionSchemaId::new(
                ResourceName::parse("device-usbip").unwrap(),
                pair.service().resource_type().clone(),
                ExtensionSchemaLayer::Status,
            ),
            status_schema_version: SchemaVersion::new(1, 0).unwrap(),
            status_details: ObjectFieldSchema::new(
                ["access".to_owned()],
                std::iter::empty::<String>(),
            )
            .unwrap(),
        };
        assert!(
            pair.service().schema_contract([shadowing]).is_err(),
            "a Provider extension may not duplicate a common status field"
        );
    }

    /// Redaction: no catalog identity renders its value.
    #[test]
    fn catalog_identities_do_not_render_their_values() {
        let pair = SemanticFamily::Audio.contract();
        let rendered = format!("{:?} {:?}", pair, pair.service().spec().schema_id());
        assert!(!rendered.contains("audio.d2bus.org"));
        assert!(!rendered.contains("AudioService"));
    }
}
