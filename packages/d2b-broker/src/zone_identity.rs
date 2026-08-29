//! Broker-local adapters for immutable Zone resource identity.
//!
//! The broker only accepts identities that are bound to the Zone, resource
//! reference, resource UID, desired-state generation, and committed revision.
//! Human names are therefore lookup labels, never global authority keys.

use std::collections::BTreeMap;

use d2b_contracts::workload::WorkloadProviderKind;
use d2b_contracts_resource::v3::{ResourceRef, ZoneId, ZoneResourceIdentity};
use serde::{Deserialize, Serialize};

/// Provider-neutral identity metadata retained by a broker adapter.
///
/// This is metadata only: provider credentials, paths, argv, and remote
/// configuration never cross into this contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ZoneResourceBinding {
    identity: ZoneResourceIdentity,
    provider_kind: WorkloadProviderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_ref: Option<ResourceRef>,
}

impl ZoneResourceBinding {
    /// Construct a binding after validating its provider reference.
    pub fn new(
        identity: ZoneResourceIdentity,
        provider_kind: WorkloadProviderKind,
        provider_ref: Option<ResourceRef>,
    ) -> Result<Self, ZoneIdentityError> {
        let binding = Self {
            identity,
            provider_kind,
            provider_ref,
        };
        binding.validate()?;
        Ok(binding)
    }

    /// Borrow the immutable Zone/resource identity.
    pub const fn identity(&self) -> &ZoneResourceIdentity {
        &self.identity
    }

    /// Return the provider-neutral runtime family.
    pub const fn provider_kind(&self) -> WorkloadProviderKind {
        self.provider_kind
    }

    /// Borrow the optional current Provider reference.
    pub const fn provider_ref(&self) -> Option<&ResourceRef> {
        self.provider_ref.as_ref()
    }

    fn validate(&self) -> Result<(), ZoneIdentityError> {
        if self
            .provider_ref
            .as_ref()
            .is_some_and(|reference| reference.resource_type().as_str() != "Provider")
        {
            return Err(ZoneIdentityError::ProviderRefMustNameProvider);
        }
        Ok(())
    }

    /// Return a path-free, stable key suitable for broker audit correlation.
    pub fn operation_key(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("ZoneResourceBinding is serializable");
        d2b_contracts_resource::v3::canonical_digest(
            "d2b:broker-zone-resource-operation:v1",
            &bytes,
        )
    }
}

impl<'de> Deserialize<'de> for ZoneResourceBinding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            identity: ZoneResourceIdentity,
            provider_kind: WorkloadProviderKind,
            #[serde(default)]
            provider_ref: Option<ResourceRef>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.identity, wire.provider_kind, wire.provider_ref)
            .map_err(serde::de::Error::custom)
    }
}

/// Closed identity failures returned by the broker-local index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneIdentityError {
    ProviderRefMustNameProvider,
    DuplicateResource,
    NotFound,
    StaleZoneUid,
    StaleResourceUid,
    StaleGeneration,
    StaleRevision,
}

impl core::fmt::Display for ZoneIdentityError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::ProviderRefMustNameProvider => "provider reference must name a Provider resource",
            Self::DuplicateResource => "Zone resource identity is already registered",
            Self::NotFound => "Zone resource identity is not registered",
            Self::StaleZoneUid => "Zone resource identity has a stale Zone UID",
            Self::StaleResourceUid => "Zone resource identity has a stale resource UID",
            Self::StaleGeneration => "Zone resource identity has a stale generation",
            Self::StaleRevision => "Zone resource identity has a stale revision",
        })
    }
}

impl std::error::Error for ZoneIdentityError {}

type ResourceLookupKey = (ZoneId, ResourceRef);

/// Exact-match index for broker-local Zone resource bindings.
///
/// The index is intentionally keyed by `(Zone name, ResourceRef)` for lookup,
/// then fences the result with both UIDs and both version values. Equal names
/// in separate Zones occupy separate entries and can never resolve through one
/// another.
#[derive(Debug, Clone, Default)]
pub struct ZoneResourceIndex {
    entries: BTreeMap<ResourceLookupKey, ZoneResourceBinding>,
}

impl ZoneResourceIndex {
    /// Build an index and reject duplicate current resources.
    pub fn from_bindings(
        bindings: impl IntoIterator<Item = ZoneResourceBinding>,
    ) -> Result<Self, ZoneIdentityError> {
        let mut index = Self::default();
        for binding in bindings {
            binding.validate()?;
            index.insert(binding)?;
        }
        Ok(index)
    }

    /// Register one current resource binding.
    pub fn insert(&mut self, binding: ZoneResourceBinding) -> Result<(), ZoneIdentityError> {
        binding.validate()?;
        let key = (
            binding.identity.zone().clone(),
            binding.identity.resource_ref().clone(),
        );
        if self.entries.contains_key(&key) {
            return Err(ZoneIdentityError::DuplicateResource);
        }
        self.entries.insert(key, binding);
        Ok(())
    }

    /// Resolve only an exact current identity.
    pub fn resolve(
        &self,
        expected: &ZoneResourceIdentity,
    ) -> Result<&ZoneResourceBinding, ZoneIdentityError> {
        let key = (expected.zone().clone(), expected.resource_ref().clone());
        let Some(binding) = self.entries.get(&key) else {
            return Err(ZoneIdentityError::NotFound);
        };
        let actual = &binding.identity;
        if actual.zone_uid() != expected.zone_uid() {
            return Err(ZoneIdentityError::StaleZoneUid);
        }
        if actual.resource_uid() != expected.resource_uid() {
            return Err(ZoneIdentityError::StaleResourceUid);
        }
        if actual.generation() != expected.generation() {
            return Err(ZoneIdentityError::StaleGeneration);
        }
        if actual.revision() != expected.revision() {
            return Err(ZoneIdentityError::StaleRevision);
        }
        if !actual.matches(
            expected.zone(),
            expected.zone_uid(),
            expected.resource_ref(),
            expected.resource_uid(),
            expected.generation(),
            expected.revision(),
        ) {
            return Err(ZoneIdentityError::NotFound);
        }
        Ok(binding)
    }

    /// Return the number of current bindings.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return whether no current bindings are registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::{ResourceGeneration, ResourceUid, ZoneRevision};

    const ZONE_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
    const OTHER_ZONE_UID: &str = "223e4567-e89b-42d3-a456-426614174001";
    const RESOURCE_UID: &str = "323e4567-e89b-42d3-a456-426614174002";

    fn identity(zone: &str, zone_uid: &str, resource_uid: &str) -> ZoneResourceIdentity {
        ZoneResourceIdentity::new(
            ZoneId::parse(zone).expect("zone"),
            ResourceUid::parse(zone_uid).expect("zone UID"),
            ResourceRef::parse("Guest/browser").expect("resource ref"),
            ResourceUid::parse(resource_uid).expect("resource UID"),
            ResourceGeneration::new(3).expect("generation"),
            ZoneRevision::new(9),
        )
    }

    fn binding(zone: &str, zone_uid: &str, resource_uid: &str) -> ZoneResourceBinding {
        ZoneResourceBinding::new(
            identity(zone, zone_uid, resource_uid),
            WorkloadProviderKind::LocalVm,
            Some(ResourceRef::parse("Provider/runtime-cloud-hypervisor").expect("provider ref")),
        )
        .expect("binding")
    }

    #[test]
    fn exact_identity_resolution_keeps_provider_metadata_neutral() {
        let binding = binding("work", ZONE_UID, RESOURCE_UID);
        let index = ZoneResourceIndex::from_bindings([binding.clone()]).expect("index");
        let resolved = index.resolve(binding.identity()).expect("exact identity");

        assert_eq!(index.len(), 1);
        assert_eq!(resolved.provider_kind(), WorkloadProviderKind::LocalVm);
        assert_eq!(
            resolved
                .provider_ref()
                .map(ResourceRef::to_canonical_string),
            Some("Provider/runtime-cloud-hypervisor".to_owned())
        );
        assert!(!resolved.operation_key().is_empty());
        assert_ne!(
            resolved.operation_key(),
            binding.identity().resource_uid().as_str()
        );
    }

    #[test]
    fn same_named_resources_in_different_zones_do_not_collide() {
        let work = binding("work", ZONE_UID, RESOURCE_UID);
        let personal = binding(
            "personal",
            OTHER_ZONE_UID,
            "423e4567-e89b-42d3-a456-426614174003",
        );
        let index = ZoneResourceIndex::from_bindings([work.clone(), personal.clone()])
            .expect("same resource name is valid across Zones");

        assert_eq!(index.len(), 2);
        assert!(index.resolve(work.identity()).is_ok());
        assert!(index.resolve(personal.identity()).is_ok());
        assert_ne!(work.operation_key(), personal.operation_key());
    }

    #[test]
    fn duplicate_current_resource_is_refused_without_replacing_the_binding() {
        let current = binding("work", ZONE_UID, RESOURCE_UID);
        let mut index = ZoneResourceIndex::from_bindings([current.clone()]).expect("index");
        assert_eq!(
            index.insert(current.clone()),
            Err(ZoneIdentityError::DuplicateResource)
        );
        assert_eq!(index.len(), 1);
        assert_eq!(
            index
                .resolve(current.identity())
                .expect("original binding remains")
                .provider_kind(),
            WorkloadProviderKind::LocalVm
        );
    }

    #[test]
    fn stale_uids_generations_and_revisions_refuse_before_effects() {
        let current = binding("work", ZONE_UID, RESOURCE_UID);
        let index = ZoneResourceIndex::from_bindings([current.clone()]).expect("index");

        let stale_zone = ZoneResourceIdentity::new(
            current.identity().zone().clone(),
            ResourceUid::parse(OTHER_ZONE_UID).expect("other zone UID"),
            current.identity().resource_ref().clone(),
            current.identity().resource_uid().clone(),
            current.identity().generation(),
            current.identity().revision(),
        );
        assert_eq!(
            index.resolve(&stale_zone),
            Err(ZoneIdentityError::StaleZoneUid)
        );

        let stale_resource = ZoneResourceIdentity::new(
            current.identity().zone().clone(),
            current.identity().zone_uid().clone(),
            current.identity().resource_ref().clone(),
            ResourceUid::parse("423e4567-e89b-42d3-a456-426614174003").expect("other resource UID"),
            current.identity().generation(),
            current.identity().revision(),
        );
        assert_eq!(
            index.resolve(&stale_resource),
            Err(ZoneIdentityError::StaleResourceUid)
        );

        let stale_generation = ZoneResourceIdentity::new(
            current.identity().zone().clone(),
            current.identity().zone_uid().clone(),
            current.identity().resource_ref().clone(),
            current.identity().resource_uid().clone(),
            ResourceGeneration::new(4).expect("new generation"),
            current.identity().revision(),
        );
        assert_eq!(
            index.resolve(&stale_generation),
            Err(ZoneIdentityError::StaleGeneration)
        );

        let stale_revision = ZoneResourceIdentity::new(
            current.identity().zone().clone(),
            current.identity().zone_uid().clone(),
            current.identity().resource_ref().clone(),
            current.identity().resource_uid().clone(),
            current.identity().generation(),
            ZoneRevision::new(10),
        );
        assert_eq!(
            index.resolve(&stale_revision),
            Err(ZoneIdentityError::StaleRevision)
        );
    }

    #[test]
    fn provider_reference_and_host_authority_fields_are_closed() {
        let identity = identity("work", ZONE_UID, RESOURCE_UID);
        let wrong_provider = ZoneResourceBinding::new(
            identity.clone(),
            WorkloadProviderKind::LocalVm,
            Some(ResourceRef::parse("Guest/browser").expect("guest ref")),
        );
        assert_eq!(
            wrong_provider,
            Err(ZoneIdentityError::ProviderRefMustNameProvider)
        );

        let mut guest_provider = serde_json::to_value(binding("work", ZONE_UID, RESOURCE_UID))
            .expect("serialize binding");
        guest_provider["providerRef"] = serde_json::json!("Guest/browser");
        let error = serde_json::from_value::<ZoneResourceBinding>(guest_provider)
            .expect_err("Guest providerRef must be refused at the JSON boundary");
        assert!(
            error.to_string().contains("Provider"),
            "provider type refusal should remain typed and bounded: {error}"
        );

        let mut json = serde_json::to_value(binding("work", ZONE_UID, RESOURCE_UID))
            .expect("serialize binding");
        json["credentialRef"] = serde_json::json!("secret");
        json["path"] = serde_json::json!("/var/lib/d2b/private");
        assert!(
            serde_json::from_value::<ZoneResourceBinding>(json).is_err(),
            "host credentials and paths must not be accepted by the neutral binding"
        );

        let malformed = ZoneResourceBinding {
            identity,
            provider_kind: WorkloadProviderKind::LocalVm,
            provider_ref: Some(ResourceRef::parse("Guest/browser").expect("guest ref")),
        };
        let mut index = ZoneResourceIndex::default();
        assert_eq!(
            index.insert(malformed.clone()),
            Err(ZoneIdentityError::ProviderRefMustNameProvider)
        );
        assert_eq!(
            ZoneResourceIndex::from_bindings([malformed]).unwrap_err(),
            ZoneIdentityError::ProviderRefMustNameProvider
        );
    }

    #[test]
    fn debug_and_operation_key_are_redacted() {
        let binding = binding("work", ZONE_UID, RESOURCE_UID);
        let debug = format!("{binding:?}");
        assert!(!debug.contains("work"));
        assert!(!debug.contains(RESOURCE_UID));
        assert!(d2b_contracts_resource::v3::is_canonical_digest(
            &binding.operation_key()
        ));
    }
}
