//! Production Provider composition for `d2bd`.
//!
//! The runtime registry is [`d2b_provider::ProviderRegistry`].  This module
//! owns only the daemon composition seam: it validates the v3 bundle identity,
//! turns trusted host-catalog rows into descriptor-bound Provider instances,
//! and associates Guest runtime rows with those instances.  It deliberately
//! does not define a second registry or a second session authority.

use std::{
    collections::BTreeMap,
    sync::RwLock,
    sync::atomic::{AtomicU64, Ordering},
};

use d2b_contracts::{
    broker_wire::BrokerCallerRole,
    v3::{
        ResourceRef,
        identity::{
            ConfigurationGeneration, ResourceGeneration, ResourceName, SchemaFingerprint,
            ServiceName, ZoneId,
        },
        zone_routing::{ZoneLabelId, ZonePath},
    },
};
use d2b_core::host::HostJson;
use d2b_provider::instance::ProviderInstance;
use d2b_provider::{
    ProviderCapabilitySet, ProviderClass, ProviderDescriptor, ProviderImplementationId,
    ProviderMethodName, ProviderRegistry, ProviderRegistryBuilder, ProviderRegistryManager,
    RegistryBuildError,
};
use sha2::{Digest, Sha256};

use crate::provider_effects::{
    EffectDispatch, GuestLifecycleOperation, GuestLifecycleRequest, ProviderEffectError,
    ProviderLifecycleDispatch, ProviderLifecycleEffectPort,
};

/// Version of the v3 Provider bundle artifact.
pub const PROVIDER_BUNDLE_VERSION: u32 = 3;

/// Schema identity of the v3 Provider bundle artifact.
pub const PROVIDER_BUNDLE_SCHEMA_VERSION: &str = "v3";

/// Registry limits and snapshots are owned by the shared Provider crate.
pub use d2b_provider::{MAX_PROVIDER_REGISTRY_ENTRIES, ProviderRegistrySnapshot};

static NEXT_LIFECYCLE_OPERATION_ID: AtomicU64 = AtomicU64::new(1);

/// Allocate a bounded daemon-local idempotency key for one lifecycle request.
pub(crate) fn next_lifecycle_operation_id(operation: &str, guest: &str) -> String {
    let ordinal = NEXT_LIFECYCLE_OPERATION_ID.fetch_add(1, Ordering::Relaxed);
    format!("provider-{operation}-{guest}-{ordinal}")
}

/// Closed Provider composition failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderCompositionError {
    /// The bundle version is not the v3 version.
    BundleVersionMismatch,
    /// The schema identity is not the v3 identity.
    BundleSchemaMismatch,
    /// A Provider reference was not a Provider resource.
    ProviderRefInvalid,
    /// A Provider reference belongs to another Zone.
    ProviderZoneMismatch,
    /// A Provider name was repeated in one snapshot.
    DuplicateProvider,
    /// The registry bound more than its fixed entry limit.
    RegistryBoundExceeded,
    /// A lifecycle operation was requested for an unregistered Provider.
    ProviderNotRegistered,
    /// A generation was zero.
    GenerationInvalid,
    /// A Zone path could not be derived from the daemon's Zone identity.
    ZonePathInvalid,
    /// The host catalog could not be fingerprinted.
    CatalogFingerprintInvalid,
    /// The registry state lock was poisoned.
    StateUnavailable,
    /// The shared Provider registry rejected a descriptor or instance.
    RegistryBuild(RegistryBuildError),
}

impl ProviderCompositionError {
    /// Stable identity-free failure code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::BundleVersionMismatch => "provider-bundle-version-mismatch",
            Self::BundleSchemaMismatch => "provider-bundle-schema-mismatch",
            Self::ProviderRefInvalid => "provider-ref-invalid",
            Self::ProviderZoneMismatch => "provider-zone-mismatch",
            Self::DuplicateProvider => "provider-duplicate",
            Self::RegistryBoundExceeded => "provider-registry-bound-exceeded",
            Self::ProviderNotRegistered => "provider-not-registered",
            Self::GenerationInvalid => "provider-generation-invalid",
            Self::ZonePathInvalid => "provider-zone-path-invalid",
            Self::CatalogFingerprintInvalid => "provider-catalog-fingerprint-invalid",
            Self::StateUnavailable => "provider-registry-state-unavailable",
            Self::RegistryBuild(_) => "provider-registry-build-rejected",
        }
    }
}

impl core::fmt::Display for ProviderCompositionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ProviderCompositionError {}

impl From<RegistryBuildError> for ProviderCompositionError {
    fn from(error: RegistryBuildError) -> Self {
        match error {
            RegistryBuildError::NotAProviderRef => Self::ProviderRefInvalid,
            RegistryBuildError::ZoneMismatch => Self::ProviderZoneMismatch,
            RegistryBuildError::DuplicateProvider => Self::DuplicateProvider,
            RegistryBuildError::BoundExceeded => Self::RegistryBoundExceeded,
            RegistryBuildError::GenerationMismatch => Self::GenerationInvalid,
            other => Self::RegistryBuild(other),
        }
    }
}

/// One Provider binding from the trusted catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderBinding {
    zone: ZoneId,
    resource: ResourceRef,
    artifact_id: ResourceName,
    schema_fingerprint: SchemaFingerprint,
}

impl ProviderBinding {
    /// Construct and validate a Zone-local Provider binding.
    pub fn new(
        zone: ZoneId,
        resource: ResourceRef,
        artifact_id: ResourceName,
        schema_fingerprint: impl Into<String>,
    ) -> Result<Self, ProviderCompositionError> {
        if resource.resource_type().as_str() != "Provider" {
            return Err(ProviderCompositionError::ProviderRefInvalid);
        }
        let schema_fingerprint = SchemaFingerprint::parse(schema_fingerprint.into())
            .map_err(|_| ProviderCompositionError::BundleSchemaMismatch)?;
        Ok(Self {
            zone,
            resource,
            artifact_id,
            schema_fingerprint,
        })
    }

    /// Borrow the binding Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the Provider ResourceRef.
    pub const fn resource(&self) -> &ResourceRef {
        &self.resource
    }

    /// Borrow the selected artifact ID.
    pub const fn artifact_id(&self) -> &ResourceName {
        &self.artifact_id
    }

    /// Borrow the signed schema fingerprint.
    pub const fn schema_fingerprint(&self) -> &SchemaFingerprint {
        &self.schema_fingerprint
    }

    fn descriptor(
        &self,
        zone: &ZonePath,
        generation: u64,
    ) -> Result<ProviderDescriptor, ProviderCompositionError> {
        let registry_generation = ConfigurationGeneration::new(generation)
            .map_err(|_| ProviderCompositionError::GenerationInvalid)?;
        let provider_generation = ResourceGeneration::new(generation)
            .map_err(|_| ProviderCompositionError::GenerationInvalid)?;
        let implementation_id = ProviderImplementationId::parse(self.artifact_id.as_str())
            .map_err(|_| ProviderCompositionError::BundleSchemaMismatch)?;
        let service = ServiceName::parse("d2b.provider.v3")
            .map_err(|_| ProviderCompositionError::BundleSchemaMismatch)?;
        let methods = ["start", "stop"]
            .into_iter()
            .map(|method| {
                ProviderMethodName::parse(method)
                    .map_err(|_| ProviderCompositionError::BundleSchemaMismatch)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let capabilities =
            ProviderCapabilitySet::new(methods).map_err(ProviderCompositionError::from)?;
        ProviderDescriptor::new(
            zone.clone(),
            self.resource.clone(),
            ProviderClass::Runtime,
            implementation_id,
            registry_generation,
            provider_generation,
            service,
            capabilities,
        )
        .map_err(ProviderCompositionError::from)
    }
}

/// Convert a Zone resource identity to the shared Provider registry's
/// authenticated routing path.
pub fn zone_path(zone: &ZoneId) -> Result<ZonePath, ProviderCompositionError> {
    let label =
        ZoneLabelId::parse(zone.as_str()).map_err(|_| ProviderCompositionError::ZonePathInvalid)?;
    ZonePath::new(vec![label]).map_err(|_| ProviderCompositionError::ZonePathInvalid)
}

/// Compose one shared Provider registry from exact daemon bindings.
pub fn compose_provider_registry(
    zone: ZoneId,
    generation: u64,
    bindings: impl IntoIterator<Item = ProviderBinding>,
) -> Result<ProviderRegistry<ProviderInstance>, ProviderCompositionError> {
    let zone_path = zone_path(&zone)?;
    let mut builder = ProviderRegistryBuilder::new(
        zone_path.clone(),
        ConfigurationGeneration::new(generation)
            .map_err(|_| ProviderCompositionError::GenerationInvalid)?,
    );
    let mut count = 0usize;
    for binding in bindings {
        if binding.zone() != &zone {
            return Err(ProviderCompositionError::ProviderZoneMismatch);
        }
        count = count.saturating_add(1);
        if count > MAX_PROVIDER_REGISTRY_ENTRIES {
            return Err(ProviderCompositionError::RegistryBoundExceeded);
        }
        let descriptor = binding.descriptor(&zone_path, generation)?;
        let instance = ProviderInstance::new(
            descriptor.provider_ref().clone(),
            descriptor.provider_generation(),
        )
        .map_err(ProviderCompositionError::from)?;
        builder
            .register_instance(descriptor, instance)
            .map_err(ProviderCompositionError::from)?;
    }
    builder.finish().map_err(ProviderCompositionError::from)
}

/// Validate the v3 bundle version and schema before composition.
pub fn validate_provider_bundle_version(
    version: u32,
    schema: &str,
) -> Result<(), ProviderCompositionError> {
    if version != PROVIDER_BUNDLE_VERSION {
        return Err(ProviderCompositionError::BundleVersionMismatch);
    }
    if schema != PROVIDER_BUNDLE_SCHEMA_VERSION {
        return Err(ProviderCompositionError::BundleSchemaMismatch);
    }
    Ok(())
}

/// Result of routing a lifecycle request through the configured Provider
/// runtime.
#[derive(Debug, PartialEq, Eq)]
pub enum ProviderRuntimeDispatch<T> {
    /// No v3 catalog is present, so the caller may use the existing daemon
    /// lifecycle path for a legacy bundle.
    Legacy,
    /// The v3 registry admitted the request and the typed effect ran, or the
    /// exact idempotency key was already accepted.
    Active(EffectDispatch<T>),
}

#[derive(Debug)]
struct ActiveProviderRuntime {
    zone: ZoneId,
    registry: ProviderRegistryManager<ProviderInstance>,
    routes: BTreeMap<String, ResourceRef>,
    lifecycle: ProviderLifecycleDispatch,
}

#[derive(Debug)]
enum ProviderRuntimeState {
    /// A pre-v3 bundle has no Provider catalog and retains the existing
    /// daemon lifecycle path.
    Legacy,
    /// A validated v3 Provider registry and Guest route index.
    Active(ActiveProviderRuntime),
    /// A catalog was present but failed validation; all lifecycle effects
    /// refuse until the daemon is rebuilt with a valid catalog.
    Refused(ProviderCompositionError),
}

/// Daemon-owned Provider composition and lifecycle routing state.
#[derive(Debug)]
pub struct ProviderRuntime {
    state: RwLock<ProviderRuntimeState>,
}

impl ProviderRuntime {
    /// Start in compatibility mode until a trusted bundle supplies a v3
    /// Provider catalog.
    pub fn new() -> Self {
        Self {
            state: RwLock::new(ProviderRuntimeState::Legacy),
        }
    }

    /// Compose the v3 catalog from the trusted host artifact.
    ///
    /// An absent catalog is an explicit compatibility state.  A present but
    /// malformed catalog is stored as refused and never silently falls back.
    pub fn configure_from_host(&self, host: &HostJson) -> Result<(), ProviderCompositionError> {
        let next = if host.runtime_providers.is_empty() {
            ProviderRuntimeState::Legacy
        } else {
            match self.compose_host_runtime(host) {
                Ok(active) => ProviderRuntimeState::Active(active),
                Err(error) => {
                    let mut state = self
                        .state
                        .write()
                        .map_err(|_| ProviderCompositionError::StateUnavailable)?;
                    *state = ProviderRuntimeState::Refused(error);
                    return Err(error);
                }
            }
        };
        let mut state = self
            .state
            .write()
            .map_err(|_| ProviderCompositionError::StateUnavailable)?;
        *state = next;
        Ok(())
    }

    /// Construct an active runtime from exact bindings and Guest routes.
    ///
    /// This is also the narrow seam used by the daemon's registration tests;
    /// every production catalog goes through the same shared
    /// `d2b_provider::ProviderRegistry` builder.
    pub fn from_bindings(
        zone: ZoneId,
        generation: u64,
        bindings: impl IntoIterator<Item = ProviderBinding>,
        routes: impl IntoIterator<Item = (String, ResourceRef)>,
    ) -> Result<Self, ProviderCompositionError> {
        let registry = compose_provider_registry(zone.clone(), generation, bindings)?;
        let mut route_index = BTreeMap::new();
        for (guest, provider_ref) in routes {
            if provider_ref.resource_type().as_str() != "Provider" {
                return Err(ProviderCompositionError::ProviderRefInvalid);
            }
            if registry.descriptor(&provider_ref).is_none() {
                return Err(ProviderCompositionError::ProviderNotRegistered);
            }
            if route_index.insert(guest, provider_ref).is_some() {
                return Err(ProviderCompositionError::DuplicateProvider);
            }
        }
        Ok(Self {
            state: RwLock::new(ProviderRuntimeState::Active(ActiveProviderRuntime {
                zone: zone.clone(),
                registry: ProviderRegistryManager::new(registry),
                routes: route_index,
                lifecycle: ProviderLifecycleDispatch::new(zone),
            })),
        })
    }

    /// Whether no v3 catalog has been supplied yet.
    pub fn is_legacy(&self) -> bool {
        self.state
            .read()
            .map(|state| matches!(*state, ProviderRuntimeState::Legacy))
            .unwrap_or(false)
    }

    /// Number of Provider descriptors in the active registry.
    pub fn registered_provider_count(&self) -> usize {
        self.state
            .read()
            .ok()
            .and_then(|state| match &*state {
                ProviderRuntimeState::Active(active) => {
                    Some(active.registry.current().snapshot().descriptors().len())
                }
                ProviderRuntimeState::Legacy | ProviderRuntimeState::Refused(_) => None,
            })
            .unwrap_or(0)
    }

    /// Whether the current catalog owns a Guest route.
    ///
    /// Legacy lifecycle requests remain on the existing daemon path when no
    /// v3 route exists.  A refused catalog returns an error instead of
    /// silently taking that compatibility path.
    pub(crate) fn lifecycle_route_available(
        &self,
        guest_name: &str,
    ) -> Result<bool, ProviderEffectError> {
        let state = self
            .state
            .read()
            .map_err(|_| ProviderEffectError::StateUnavailable)?;
        match &*state {
            ProviderRuntimeState::Legacy => Ok(false),
            ProviderRuntimeState::Active(active) => Ok(active.routes.contains_key(guest_name)),
            ProviderRuntimeState::Refused(error) => {
                let _ = error.code();
                Err(ProviderEffectError::RegistryUnavailable)
            }
        }
    }

    /// Route one Guest lifecycle request through registry admission and a
    /// descriptor-bound typed effect port.
    pub fn dispatch_lifecycle<P: ProviderLifecycleEffectPort>(
        &self,
        caller: &BrokerCallerRole,
        guest_name: &str,
        operation: GuestLifecycleOperation,
        idempotency_key: impl Into<String>,
        effect: &P,
    ) -> Result<ProviderRuntimeDispatch<P::Output>, ProviderEffectError> {
        let state = self
            .state
            .read()
            .map_err(|_| ProviderEffectError::StateUnavailable)?;
        let ProviderRuntimeState::Active(active) = &*state else {
            return match &*state {
                ProviderRuntimeState::Legacy => Ok(ProviderRuntimeDispatch::Legacy),
                ProviderRuntimeState::Refused(error) => {
                    let _ = error.code();
                    Err(ProviderEffectError::RegistryUnavailable)
                }
                ProviderRuntimeState::Active(_) => unreachable!("active state matched above"),
            };
        };
        let provider_ref = active
            .routes
            .get(guest_name)
            .ok_or(ProviderEffectError::ProviderNotRegistered)?;
        let registry = active.registry.current();
        let descriptor = registry
            .descriptor(provider_ref)
            .ok_or(ProviderEffectError::ProviderNotRegistered)?;
        let method = ProviderMethodName::parse(operation.as_str())
            .map_err(|_| ProviderEffectError::ProviderCapabilityDenied)?;
        if !descriptor.capabilities().contains_method(&method) {
            return Err(ProviderEffectError::ProviderCapabilityDenied);
        }
        let guest = ResourceRef::parse(&format!("Guest/{guest_name}"))
            .map_err(|_| ProviderEffectError::GuestRefInvalid)?;
        let request =
            GuestLifecycleRequest::new(active.zone.clone(), guest, operation, idempotency_key)
                .map_err(|_| ProviderEffectError::GuestRefInvalid)?;
        active
            .lifecycle
            .dispatch(caller, &request, effect)
            .map(ProviderRuntimeDispatch::Active)
    }

    fn compose_host_runtime(
        &self,
        host: &HostJson,
    ) -> Result<ActiveProviderRuntime, ProviderCompositionError> {
        let zone =
            ZoneId::parse("local-root").map_err(|_| ProviderCompositionError::ZonePathInvalid)?;
        let mut bindings = Vec::with_capacity(host.runtime_providers.len());
        let mut provider_refs = BTreeMap::new();
        for metadata in &host.runtime_providers {
            let name = ResourceName::parse(metadata.provider.id.clone())
                .map_err(|_| ProviderCompositionError::ProviderRefInvalid)?;
            let provider_ref = ResourceRef::parse(&format!("Provider/{}", name.as_str()))
                .map_err(|_| ProviderCompositionError::ProviderRefInvalid)?;
            let fingerprint = runtime_catalog_fingerprint(metadata)?;
            bindings.push(ProviderBinding::new(
                zone.clone(),
                provider_ref.clone(),
                name,
                fingerprint,
            )?);
            if provider_refs
                .insert(metadata.provider.id.clone(), provider_ref)
                .is_some()
            {
                return Err(ProviderCompositionError::DuplicateProvider);
            }
        }
        let registry = compose_provider_registry(zone.clone(), 1, bindings)?;
        let mut routes = BTreeMap::new();
        for row in &host.vm_runtimes {
            let provider_ref = provider_refs
                .get(&row.runtime.provider.id)
                .ok_or(ProviderCompositionError::ProviderNotRegistered)?
                .clone();
            if routes.insert(row.vm.clone(), provider_ref).is_some() {
                return Err(ProviderCompositionError::DuplicateProvider);
            }
        }
        Ok(ActiveProviderRuntime {
            zone: zone.clone(),
            registry: ProviderRegistryManager::new(registry),
            routes,
            lifecycle: ProviderLifecycleDispatch::new(zone),
        })
    }
}

impl Default for ProviderRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn runtime_catalog_fingerprint(
    metadata: &d2b_core::runtime::RuntimeMetadata,
) -> Result<String, ProviderCompositionError> {
    let bytes = serde_json::to_vec(metadata)
        .map_err(|_| ProviderCompositionError::CatalogFingerprintInvalid)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("sha256:{digest:x}"))
}

/// Keep the ResourceType identity available to callers without accepting a
/// free-form type alias.
pub fn provider_resource_type() -> d2b_contracts::v3::identity::ResourceTypeName {
    d2b_contracts::v3::identity::ResourceTypeName::parse("Provider")
        .expect("Provider is in the v3 catalog")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_effects::{EffectDispatch, ProviderLifecycleEffectPort};

    const FINGERPRINT: &str =
        "sha256:0000000000000000000000000000000000000000000000000000000000000001";

    fn zone() -> ZoneId {
        ZoneId::parse("work").expect("Zone")
    }

    fn binding(name: &str) -> ProviderBinding {
        ProviderBinding::new(
            zone(),
            ResourceRef::parse(&format!("Provider/{name}")).expect("Provider ref"),
            ResourceName::parse(name).expect("Provider name"),
            FINGERPRINT,
        )
        .expect("binding")
    }

    struct RecordingEffect;

    impl ProviderLifecycleEffectPort for RecordingEffect {
        type Output = &'static str;

        fn apply(
            &self,
            _request: &GuestLifecycleRequest,
        ) -> Result<Self::Output, ProviderEffectError> {
            Ok("broker-effect-dispatched")
        }
    }

    #[test]
    fn registration_uses_the_shared_registry_and_resolves_exact_provider() {
        validate_provider_bundle_version(3, "v3").expect("v3 gate");
        assert!(validate_provider_bundle_version(2, "v2").is_err());
        let registry = compose_provider_registry(zone(), 1, [binding("system-core")])
            .expect("shared registry composition");
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.generation().get(), 1);
        assert_eq!(snapshot.descriptors().len(), 1);
        assert!(
            registry
                .descriptor(&ResourceRef::parse("Provider/system-core").expect("Provider ref"))
                .is_some()
        );
    }

    #[test]
    fn non_provider_resource_ref_cannot_enter_registry() {
        let error = ProviderBinding::new(
            zone(),
            ResourceRef::parse("Guest/workstation").expect("Guest ref"),
            ResourceName::parse("guest").expect("resource name"),
            FINGERPRINT,
        )
        .expect_err("non-Provider ref must fail");
        assert_eq!(error, ProviderCompositionError::ProviderRefInvalid);
    }

    #[test]
    fn active_registration_reaches_the_typed_effect_and_unknown_routes_refuse() {
        let provider = ResourceRef::parse("Provider/runtime").expect("Provider ref");
        let runtime = ProviderRuntime::from_bindings(
            zone(),
            1,
            [binding("runtime")],
            [("workstation".to_owned(), provider)],
        )
        .expect("runtime composition");
        assert_eq!(runtime.registered_provider_count(), 1);
        let effect = RecordingEffect;
        let caller = BrokerCallerRole::AdminUid { uid: 1000 };
        let result = runtime
            .dispatch_lifecycle(
                &caller,
                "workstation",
                GuestLifecycleOperation::Start,
                "operation-1",
                &effect,
            )
            .expect("lifecycle dispatch");
        assert_eq!(
            result,
            ProviderRuntimeDispatch::Active(EffectDispatch::Dispatched("broker-effect-dispatched"))
        );
        assert_eq!(
            runtime.dispatch_lifecycle(
                &caller,
                "unknown",
                GuestLifecycleOperation::Start,
                "operation-2",
                &effect
            ),
            Err(ProviderEffectError::ProviderNotRegistered)
        );
        assert_eq!(
            runtime.dispatch_lifecycle(
                &BrokerCallerRole::NotAuthorized,
                "workstation",
                GuestLifecycleOperation::Stop,
                "operation-3",
                &effect
            ),
            Err(ProviderEffectError::CallerRoleDenied)
        );
    }
}
