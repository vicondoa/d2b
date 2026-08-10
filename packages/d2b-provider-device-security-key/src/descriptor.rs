//! Catalog-derived semantic metadata for the security-key Provider.
//!
//! The enclosing Provider manifest signs this fragment. The semantic factory
//! is always obtained from the shared catalog so this crate cannot publish a
//! provider-specific Service or Binding type, choose a physical backing type,
//! or alter the projection protocol independently of Core.

use d2b_contracts::v3::{
    ExtensionSchemaRegistration, ProjectionFactory, ProviderContractError, ResourceApiBinding,
    ResourceSpec, SemanticContractError, SemanticFamily, SemanticTypeContract,
    StandardCapabilityMatrix,
};
use serde::{Deserialize, Deserializer, Serialize};

/// The provider-neutral security-key Service ResourceType.
pub const SECURITY_KEY_SERVICE_RESOURCE_TYPE: &str = "security-key.d2bus.org.SecurityKeyService";

/// The provider-neutral security-key Binding ResourceType.
pub const SECURITY_KEY_BINDING_RESOURCE_TYPE: &str = "security-key.d2bus.org.SecurityKeyBinding";

/// The projection protocol version used by the catalog-derived factory.
pub const SECURITY_KEY_PROJECTION_PROTOCOL_VERSION: &str =
    d2b_contracts::v3::SEMANTIC_PROJECTION_PROTOCOL_VERSION;

/// The semantic, catalog-derived portion of the signed Provider descriptor.
///
/// This value contains the exact base bindings for the Service and Binding
/// plus the signed projection factory. Physical Device claims, relay
/// Endpoints, Process declarations, and effect tickets remain outside this
/// semantic fragment.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityKeySemanticDescriptor {
    service_binding: ResourceApiBinding,
    binding_binding: ResourceApiBinding,
    projection_factory: ProjectionFactory,
}

impl SecurityKeySemanticDescriptor {
    /// Build the semantic descriptor from the shared D098 catalog.
    ///
    /// The catalog makes the empty backing set determinate and constructs it
    /// as a deny-all set. No physical Device or Endpoint type is introduced
    /// here.
    pub fn from_catalog() -> Result<Self, ProviderContractError> {
        let pair = SemanticFamily::SecurityKey.contract();
        let service_binding = base_binding(pair.service())?;
        let binding_binding = base_binding(pair.binding())?;
        let projection_factory = pair
            .projection()
            .projection_factory()
            .map_err(|_| ProviderContractError::ProjectionFactoryInvalid)?;
        Ok(Self {
            service_binding,
            binding_binding,
            projection_factory,
        })
    }

    /// Borrow the signed Service base binding.
    pub const fn service_binding(&self) -> &ResourceApiBinding {
        &self.service_binding
    }

    /// Borrow the signed Binding base binding.
    pub const fn binding_binding(&self) -> &ResourceApiBinding {
        &self.binding_binding
    }

    /// Borrow the catalog-derived signed projection factory.
    pub const fn projection_factory(&self) -> &ProjectionFactory {
        &self.projection_factory
    }

    /// Return the Service and Binding base bindings in stable order.
    pub fn api_bindings(&self) -> [&ResourceApiBinding; 2] {
        [&self.service_binding, &self.binding_binding]
    }

    /// Return the catalog's projection schema fingerprint.
    pub const fn projection_schema_fingerprint(&self) -> &d2b_contracts::v3::SchemaFingerprint {
        self.projection_factory.projection_schema_fingerprint()
    }

    /// Return the catalog's semantic factory fingerprint.
    pub const fn factory_fingerprint(&self) -> &d2b_contracts::v3::SchemaFingerprint {
        self.projection_factory.factory_fingerprint()
    }

    /// Validate a Core-created projection Service spec against the catalog.
    ///
    /// Core-created projections have no Provider extension and use the
    /// provider-neutral projection branch. This method deliberately does not
    /// inspect or construct any physical backing claim.
    pub fn validate_projection_spec(
        &self,
        spec: &ResourceSpec,
    ) -> Result<(), SemanticContractError> {
        SemanticFamily::SecurityKey
            .contract()
            .projection()
            .validate_projection_spec(spec)
    }

    /// Verify that this descriptor still equals the current catalog output.
    ///
    /// This is useful at the signed-descriptor loading boundary: a serialized
    /// fragment must not be accepted after the catalog changes underneath it.
    pub fn validate_against_catalog(&self) -> Result<(), ProviderContractError> {
        let expected = Self::from_catalog()?;
        if self == &expected {
            Ok(())
        } else {
            Err(ProviderContractError::ProjectionFactoryInvalid)
        }
    }
}

impl core::fmt::Debug for SecurityKeySemanticDescriptor {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SecurityKeySemanticDescriptor")
            .field("service_binding", &self.service_binding)
            .field("binding_binding", &self.binding_binding)
            .field("projection_factory", &self.projection_factory)
            .finish()
    }
}

impl<'de> Deserialize<'de> for SecurityKeySemanticDescriptor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            service_binding: ResourceApiBinding,
            binding_binding: ResourceApiBinding,
            projection_factory: ProjectionFactory,
        }

        let wire = Wire::deserialize(deserializer)?;
        let descriptor = Self {
            service_binding: wire.service_binding,
            binding_binding: wire.binding_binding,
            projection_factory: wire.projection_factory,
        };
        descriptor
            .validate_against_catalog()
            .map_err(serde::de::Error::custom)?;
        Ok(descriptor)
    }
}

/// Build the signed semantic descriptor for `Provider/device-security-key`.
pub fn security_key_semantic_descriptor()
-> Result<SecurityKeySemanticDescriptor, ProviderContractError> {
    SecurityKeySemanticDescriptor::from_catalog()
}

/// Build the catalog-derived security-key projection factory.
pub fn security_key_projection_factory() -> Result<ProjectionFactory, ProviderContractError> {
    Ok(security_key_semantic_descriptor()?
        .projection_factory()
        .clone())
}

/// Borrow the catalog-derived security-key projection schema fingerprint.
pub fn security_key_projection_schema_fingerprint()
-> Result<&'static d2b_contracts::v3::SchemaFingerprint, ProviderContractError> {
    Ok(SemanticFamily::SecurityKey
        .contract()
        .projection()
        .projection_schema_fingerprint())
}

/// Borrow the catalog-derived security-key factory fingerprint.
pub fn security_key_factory_fingerprint()
-> Result<&'static d2b_contracts::v3::SchemaFingerprint, ProviderContractError> {
    Ok(SemanticFamily::SecurityKey
        .contract()
        .projection()
        .factory_fingerprint())
}

fn base_binding(
    member: &SemanticTypeContract,
) -> Result<ResourceApiBinding, ProviderContractError> {
    ResourceApiBinding::new(
        member.resource_type().clone(),
        member.spec().version(),
        member.spec().fingerprint().clone(),
        member.status().version(),
        member.status().fingerprint().clone(),
        StandardCapabilityMatrix::default(),
        None::<ExtensionSchemaRegistration>,
        None::<ExtensionSchemaRegistration>,
    )
}
