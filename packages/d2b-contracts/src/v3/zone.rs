//! The Zone self-resource contract.
//!
//! A Zone is an identity anchor, not a configuration container.  Its desired
//! spec is the empty object; topology and configuration publication are
//! carried by the surrounding Zone runtime and by other resources.  Keeping
//! that distinction in the type system prevents a caller from smuggling
//! parent topology, policy, or implementation settings into the self row.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    ResourceName, ResourcePhase, ResourceRef, ResourceUid, Timestamp, ZoneId,
    execution_policy::redacted_debug,
};

/// The canonical Zone ResourceType name.
pub const ZONE_RESOURCE_TYPE: &str = "Zone";
/// The only Core finalizer a Zone may carry.
pub const ZONE_DRAIN_FINALIZER: &str = "core.zone-drain";
/// The maximum number of fixed core handlers projected in Zone status.
pub const MAX_ZONE_HANDLERS: usize = 64;

/// Identity and lifecycle failures for the Zone self-resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneContractError {
    /// The Zone desired spec was not the empty object.
    SpecNotEmpty,
    /// A self-resource name or zone did not match the store identity.
    SelfIdentityMismatch,
    /// The resource carried an owner reference.
    OwnerReferenceForbidden,
    /// The resource carried a non-Core finalizer.
    FinalizerForbidden,
    /// More than one Zone was found in a store.
    CardinalityExceeded,
    /// A UID did not match the store-generated identity.
    UidMismatch,
    /// A condition or handler projection exceeded its bound.
    BoundExceeded,
}

impl core::fmt::Display for ZoneContractError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::SpecNotEmpty => "zone-spec-invalid",
            Self::SelfIdentityMismatch => "zone-self-identity-mismatch",
            Self::OwnerReferenceForbidden => "zone-owner-forbidden",
            Self::FinalizerForbidden => "zone-finalizer-forbidden",
            Self::CardinalityExceeded => "zone-cardinality-exceeded",
            Self::UidMismatch => "zone-uid-mismatch",
            Self::BoundExceeded => "zone-bound-exceeded",
        })
    }
}

impl std::error::Error for ZoneContractError {}

/// The only valid Zone desired-state object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ZoneSpec {}

impl ZoneSpec {
    /// Construct the empty Zone spec.
    pub const fn new() -> Self {
        Self {}
    }

    /// Validate the self-resource desired state.
    pub const fn validate(&self) -> Result<(), ZoneContractError> {
        Ok(())
    }
}

impl<'de> Deserialize<'de> for ZoneSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {}
        let _ = Wire::deserialize(deserializer)?;
        Ok(Self::new())
    }
}

/// A fixed handler phase projected by the Zone controller.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneHandlerPhase {
    /// The handler has not completed startup.
    Pending,
    /// The handler is operational.
    Ready,
    /// The handler is operational with an optional impairment.
    Degraded,
    /// The handler failed.
    Failed,
    /// The handler state cannot be established.
    Unknown,
}

/// The closed fixed-handler name set.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneHandlerName {
    /// Configuration publication.
    ConfigurationPublication,
    /// Authorization index.
    Authorization,
    /// API catalog.
    ApiCatalog,
    /// Provider lifecycle.
    ProviderLifecycle,
    /// ZoneLink maintenance.
    ZoneLink,
    /// Backup cleanup.
    BackupCleanup,
}

/// One bounded handler observation.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZoneHandlerStatus {
    name: ZoneHandlerName,
    phase: ZoneHandlerPhase,
    last_reconciled_at: Option<Timestamp>,
}

impl ZoneHandlerStatus {
    /// Construct one handler observation.
    pub const fn new(
        name: ZoneHandlerName,
        phase: ZoneHandlerPhase,
        last_reconciled_at: Option<Timestamp>,
    ) -> Self {
        Self {
            name,
            phase,
            last_reconciled_at,
        }
    }

    /// Return the fixed handler name.
    pub const fn name(&self) -> ZoneHandlerName {
        self.name
    }

    /// Return the handler phase.
    pub const fn phase(&self) -> ZoneHandlerPhase {
        self.phase
    }

    /// Borrow the last reconcile timestamp.
    pub const fn last_reconciled_at(&self) -> Option<&Timestamp> {
        self.last_reconciled_at.as_ref()
    }
}

redacted_debug!(ZoneHandlerStatus);

impl<'de> Deserialize<'de> for ZoneHandlerStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            name: ZoneHandlerName,
            last_reconciled_at: Option<Timestamp>,
            phase: ZoneHandlerPhase,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self::new(wire.name, wire.phase, wire.last_reconciled_at))
    }
}

/// The ResourceType-common Zone status layer.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZoneStatusResource {
    api_catalog_revision: u64,
    policy_revision: u64,
    configuration_revision: u64,
    core_controller_phase: ResourcePhase,
    handlers: Vec<ZoneHandlerStatus>,
    installed_provider_count: u32,
    ready_provider_count: u32,
    total_resource_count: u32,
    active_configuration_generation: u64,
    generation_cleanup_pending: bool,
    cleanup_pending_count: u32,
}

impl ZoneStatusResource {
    /// Construct the bounded Zone status projection.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        api_catalog_revision: u64,
        policy_revision: u64,
        configuration_revision: u64,
        core_controller_phase: ResourcePhase,
        mut handlers: Vec<ZoneHandlerStatus>,
        installed_provider_count: u32,
        ready_provider_count: u32,
        total_resource_count: u32,
        active_configuration_generation: u64,
        generation_cleanup_pending: bool,
        cleanup_pending_count: u32,
    ) -> Result<Self, ZoneContractError> {
        if handlers.len() > MAX_ZONE_HANDLERS
            || ready_provider_count > installed_provider_count
            || (!generation_cleanup_pending && cleanup_pending_count != 0)
        {
            return Err(ZoneContractError::BoundExceeded);
        }
        handlers.sort_by_key(|handler| handler.name());
        if handlers
            .windows(2)
            .any(|pair| pair[0].name() == pair[1].name())
        {
            return Err(ZoneContractError::BoundExceeded);
        }
        Ok(Self {
            api_catalog_revision,
            policy_revision,
            configuration_revision,
            core_controller_phase,
            handlers,
            installed_provider_count,
            ready_provider_count,
            total_resource_count,
            active_configuration_generation,
            generation_cleanup_pending,
            cleanup_pending_count,
        })
    }

    /// Return the API catalog revision.
    pub const fn api_catalog_revision(&self) -> u64 {
        self.api_catalog_revision
    }

    /// Return the policy revision.
    pub const fn policy_revision(&self) -> u64 {
        self.policy_revision
    }

    /// Return the active configuration revision.
    pub const fn configuration_revision(&self) -> u64 {
        self.configuration_revision
    }

    /// Return the aggregate core phase.
    pub const fn core_controller_phase(&self) -> ResourcePhase {
        self.core_controller_phase
    }

    /// Borrow bounded handler observations.
    pub fn handlers(&self) -> &[ZoneHandlerStatus] {
        &self.handlers
    }

    /// Return the number of installed Providers.
    pub const fn installed_provider_count(&self) -> u32 {
        self.installed_provider_count
    }

    /// Return the number of ready Providers.
    pub const fn ready_provider_count(&self) -> u32 {
        self.ready_provider_count
    }

    /// Return the non-deleted resource count.
    pub const fn total_resource_count(&self) -> u32 {
        self.total_resource_count
    }

    /// Whether prior-generation cleanup remains.
    pub const fn generation_cleanup_pending(&self) -> bool {
        self.generation_cleanup_pending
    }

    /// Return the pending cleanup count.
    pub const fn cleanup_pending_count(&self) -> u32 {
        self.cleanup_pending_count
    }
}

redacted_debug!(ZoneStatusResource);

impl<'de> Deserialize<'de> for ZoneStatusResource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            api_catalog_revision: u64,
            policy_revision: u64,
            configuration_revision: u64,
            core_controller_phase: ResourcePhase,
            handlers: Vec<ZoneHandlerStatus>,
            installed_provider_count: u32,
            ready_provider_count: u32,
            total_resource_count: u32,
            active_configuration_generation: u64,
            generation_cleanup_pending: bool,
            cleanup_pending_count: u32,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.api_catalog_revision,
            wire.policy_revision,
            wire.configuration_revision,
            wire.core_controller_phase,
            wire.handlers,
            wire.installed_provider_count,
            wire.ready_provider_count,
            wire.total_resource_count,
            wire.active_configuration_generation,
            wire.generation_cleanup_pending,
            wire.cleanup_pending_count,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Alias used by generic ResourceType status adapters.
pub type ZoneStatus = ZoneStatusResource;

/// Closed Zone condition names.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneConditionType {
    /// The redb store is open and healthy.
    StoreReady,
    /// The active configuration is current.
    ConfigurationCurrent,
    /// The API catalog is bound.
    ApiCatalogReady,
    /// The authorization index is ready.
    AuthorizationReady,
    /// Required Providers are healthy.
    ProvidersHealthy,
    /// Core handlers have initialized.
    CoreHandlerReady,
    /// Prior-generation cleanup is still in progress.
    GenerationCleanupPending,
    /// Prior-generation cleanup is stuck.
    GenerationCleanupFailed,
}

/// Validate the Zone self-resource identity and ownership invariants.
#[allow(clippy::too_many_arguments)]
pub fn validate_self_resource(
    store_zone: &ZoneId,
    store_uid: &ResourceUid,
    resource_name: &ResourceName,
    resource_zone: &ZoneId,
    resource_uid: &ResourceUid,
    owner_ref: Option<&ResourceRef>,
    finalizers: &[super::FinalizerId],
    cardinality: usize,
) -> Result<(), ZoneContractError> {
    if resource_name.as_str() != store_zone.as_str() || resource_zone != store_zone {
        return Err(ZoneContractError::SelfIdentityMismatch);
    }
    if resource_uid != store_uid {
        return Err(ZoneContractError::UidMismatch);
    }
    if owner_ref.is_some() {
        return Err(ZoneContractError::OwnerReferenceForbidden);
    }
    if cardinality != 1 {
        return Err(ZoneContractError::CardinalityExceeded);
    }
    if finalizers
        .iter()
        .any(|finalizer| finalizer.as_str() != ZONE_DRAIN_FINALIZER)
    {
        return Err(ZoneContractError::FinalizerForbidden);
    }
    Ok(())
}

/// Validate the only allowed Zone finalizer shape.
pub fn validate_finalizer(finalizer: &super::FinalizerId) -> Result<(), ZoneContractError> {
    if finalizer.as_str() == ZONE_DRAIN_FINALIZER {
        Ok(())
    } else {
        Err(ZoneContractError::FinalizerForbidden)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_spec_is_exactly_empty() {
        let spec: ZoneSpec = serde_json::from_slice(br#"{}"#).unwrap();
        assert_eq!(spec, ZoneSpec::new());
        assert!(serde_json::from_slice::<ZoneSpec>(br#"{"parentZone":"root"}"#).is_err());
        assert_eq!(serde_json::to_vec(&spec).unwrap(), br#"{}"#);
    }

    #[test]
    fn self_resource_checks_store_name_uid_and_owner() {
        let zone = ZoneId::parse("dev").unwrap();
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let name = ResourceName::parse("dev").unwrap();
        assert!(validate_self_resource(&zone, &uid, &name, &zone, &uid, None, &[], 1).is_ok());
        assert_eq!(
            validate_self_resource(
                &zone,
                &uid,
                &ResourceName::parse("other").unwrap(),
                &zone,
                &uid,
                None,
                &[],
                1,
            ),
            Err(ZoneContractError::SelfIdentityMismatch)
        );
    }

    #[test]
    fn handler_status_is_sorted_and_bounded() {
        let status = ZoneStatusResource::new(
            1,
            1,
            1,
            ResourcePhase::Ready,
            vec![
                ZoneHandlerStatus::new(ZoneHandlerName::ZoneLink, ZoneHandlerPhase::Ready, None),
                ZoneHandlerStatus::new(
                    ZoneHandlerName::Authorization,
                    ZoneHandlerPhase::Ready,
                    None,
                ),
            ],
            2,
            1,
            4,
            1,
            false,
            0,
        )
        .unwrap();
        assert_eq!(status.handlers()[0].name(), ZoneHandlerName::Authorization);
    }
}
