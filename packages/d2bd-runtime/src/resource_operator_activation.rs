//! Production orchestration for the Wave 6 operator acceptance boundary.
//!
//! The Zone runtime owns the authenticated Resource API and the ordering
//! contract.  Provider-specific effects stay behind this boundary so the
//! same path can be exercised with the shipped Volume, Network, Device TPM,
//! and Cloud Hypervisor controllers without teaching the Resource API about
//! provider implementation details.

use std::collections::BTreeMap;

use async_trait::async_trait;
use d2b_contracts_resource::resource_proto as wire;
use d2b_contracts_resource::v3::{
    ResourceGeneration,
    ResourceRef,
    ResourceUid,
};
use d2b_resource_api::{RedbBackend, ResourceApiClient, service::UnavailableUpgradeDispatcher};
use protobuf::{EnumOrUnknown, MessageField};
use serde_json::Value;

/// The exact resource set required by the Wave 6 operator acceptance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Wave6ResourceKind {
    /// Local durable state.
    Volume,
    /// Host network realization.
    Network,
    /// Device TPM state and endpoint.
    DeviceTpm,
    /// Cloud Hypervisor guest process.
    CloudHypervisorGuest,
}

impl Wave6ResourceKind {
    /// Return the public Resource API type for this acceptance resource.
    pub const fn resource_type(self) -> &'static str {
        match self {
            Self::Volume => "Volume",
            Self::Network => "Network",
            Self::DeviceTpm => "Device",
            Self::CloudHypervisorGuest => "Guest",
        }
    }

    const fn operation_id(self) -> &'static str {
        match self {
            Self::Volume => "wave6-operator-list-volume",
            Self::Network => "wave6-operator-list-network",
            Self::DeviceTpm => "wave6-operator-list-device-tpm",
            Self::CloudHypervisorGuest => "wave6-operator-list-guest",
        }
    }
}

/// One resource admitted by the authenticated operator subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wave6Resource {
    /// The canonical public identity.
    pub resource_ref: ResourceRef,
    /// The durable identity that must survive adoption.
    pub uid: ResourceUid,
    /// The current desired generation.
    pub generation: ResourceGeneration,
    /// The provider route carried by the public resource spec.
    pub provider_ref: ResourceRef,
    /// The exact canonical payload returned by the Resource API.
    pub canonical_json: Vec<u8>,
}

/// The four resources selected from one Zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wave6ResourceSet {
    /// Local state Volume.
    pub volume: Wave6Resource,
    /// Host Network.
    pub network: Wave6Resource,
    /// Device TPM.
    pub device_tpm: Wave6Resource,
    /// Cloud Hypervisor Guest.
    pub cloud_hypervisor_guest: Wave6Resource,
}

impl Wave6ResourceSet {
    fn insert(
        resources: &mut BTreeMap<Wave6ResourceKind, Wave6Resource>,
        kind: Wave6ResourceKind,
        resource: Wave6Resource,
    ) {
        resources.insert(kind, resource);
    }

    fn from_resources(
        resources: BTreeMap<Wave6ResourceKind, Wave6Resource>,
    ) -> Result<Self, Wave6BoundaryError> {
        Ok(Self {
            volume: resources
                .get(&Wave6ResourceKind::Volume)
                .cloned()
                .ok_or(Wave6BoundaryError::ResourceSelection)?,
            network: resources
                .get(&Wave6ResourceKind::Network)
                .cloned()
                .ok_or(Wave6BoundaryError::ResourceSelection)?,
            device_tpm: resources
                .get(&Wave6ResourceKind::DeviceTpm)
                .cloned()
                .ok_or(Wave6BoundaryError::ResourceSelection)?,
            cloud_hypervisor_guest: resources
                .get(&Wave6ResourceKind::CloudHypervisorGuest)
                .cloned()
                .ok_or(Wave6BoundaryError::ResourceSelection)?,
        })
    }

    /// Return all resources keyed by their acceptance kind.
    pub fn by_kind(&self) -> BTreeMap<Wave6ResourceKind, Wave6Resource> {
        BTreeMap::from([
            (Wave6ResourceKind::Volume, self.volume.clone()),
            (Wave6ResourceKind::Network, self.network.clone()),
            (Wave6ResourceKind::DeviceTpm, self.device_tpm.clone()),
            (
                Wave6ResourceKind::CloudHypervisorGuest,
                self.cloud_hypervisor_guest.clone(),
            ),
        ])
    }
}

/// Dependency state supplied to a provider reconcile boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wave6Dependencies {
    /// Whether Volume layout and backing are ready.
    pub volume_ready: bool,
    /// Whether Network realization is ready.
    pub network_ready: bool,
    /// Whether the Device TPM endpoint is ready.
    pub device_tpm_ready: bool,
    /// Whether the Guest process is ready.
    pub guest_ready: bool,
    /// Whether attachment realization is ready.
    pub attachment_ready: bool,
}

impl Wave6Dependencies {
    pub const fn network_waiting_for_volume() -> Self {
        Self {
            volume_ready: false,
            network_ready: false,
            device_tpm_ready: true,
            guest_ready: false,
            attachment_ready: false,
        }
    }

    pub const fn guest_waiting_for_network() -> Self {
        Self {
            volume_ready: true,
            network_ready: false,
            device_tpm_ready: true,
            guest_ready: false,
            attachment_ready: false,
        }
    }

    pub const fn network_ready_for_guest() -> Self {
        Self {
            volume_ready: true,
            network_ready: true,
            device_tpm_ready: true,
            guest_ready: true,
            attachment_ready: true,
        }
    }

    pub const fn guest_ready_for_adoption() -> Self {
        Self {
            volume_ready: true,
            network_ready: true,
            device_tpm_ready: true,
            guest_ready: true,
            attachment_ready: true,
        }
    }
}

/// Provider reconcile result used by the production dependency sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wave6ReconcileResult {
    /// The provider correctly stopped before touching an unavailable child.
    Waiting,
    /// The provider reached its ready boundary.
    Ready,
}

/// Stable failure classes exposed by the operator acceptance boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wave6BoundaryError {
    /// The public API did not expose exactly one resource of a required type.
    ResourceSelection,
    /// A resource did not carry a valid provider route.
    ProviderRoute,
    /// A provider returned an unexpected dependency or lifecycle phase.
    Lifecycle,
    /// A provider failed to retain the Device TPM state identity.
    DeviceStateNotRetained,
    /// A provider effect failed.
    Effect,
}

impl core::fmt::Display for Wave6BoundaryError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::ResourceSelection => "wave6-resource-selection-failed",
            Self::ProviderRoute => "wave6-provider-route-invalid",
            Self::Lifecycle => "wave6-provider-lifecycle-failed",
            Self::DeviceStateNotRetained => "wave6-device-state-not-retained",
            Self::Effect => "wave6-provider-effect-failed",
        })
    }
}

impl std::error::Error for Wave6BoundaryError {}

/// Provider-owned effects used by the production acceptance sequence.
///
/// Implementations must call the real Provider controllers/effect ports.  A
/// call-recording fake cannot prove this contract because dependency waits,
/// process adoption, and retained TPM state are all externally observable.
#[async_trait]
pub trait Wave6ProviderBoundary: Send + Sync {
    /// Reconcile the local Volume.
    async fn reconcile_volume(
        &self,
        resource: &Wave6Resource,
    ) -> Result<Wave6ReconcileResult, Wave6BoundaryError>;

    /// Reconcile the host Network with the supplied dependency projection.
    async fn reconcile_network(
        &self,
        resource: &Wave6Resource,
        dependencies: Wave6Dependencies,
    ) -> Result<Wave6ReconcileResult, Wave6BoundaryError>;

    /// Reconcile the Device TPM.
    async fn reconcile_device_tpm(
        &self,
        resource: &Wave6Resource,
    ) -> Result<Wave6ReconcileResult, Wave6BoundaryError>;

    /// Reconcile the Cloud Hypervisor Guest with the supplied dependency
    /// projection.
    async fn reconcile_cloud_hypervisor_guest(
        &self,
        resource: &Wave6Resource,
        dependencies: Wave6Dependencies,
    ) -> Result<Wave6ReconcileResult, Wave6BoundaryError>;

    /// Reconstruct Provider state and adopt already-running effects after a
    /// daemon restart.
    async fn adopt_after_restart(
        &self,
        resources: &Wave6ResourceSet,
    ) -> Result<(), Wave6BoundaryError>;

    /// Remove the Guest before its Network and Volume dependencies.
    async fn remove_cloud_hypervisor_guest(
        &self,
        resource: &Wave6Resource,
    ) -> Result<(), Wave6BoundaryError>;

    /// Remove the Network after Guest and attachment teardown.
    async fn remove_network(&self, resource: &Wave6Resource) -> Result<(), Wave6BoundaryError>;

    /// Finalize Device TPM process state while retaining its state Volume.
    async fn remove_device_tpm(&self, resource: &Wave6Resource)
    -> Result<bool, Wave6BoundaryError>;

    /// Remove the Volume last.
    async fn remove_volume(&self, resource: &Wave6Resource) -> Result<(), Wave6BoundaryError>;
}

/// Result of the full operator acceptance sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wave6AcceptanceReport {
    /// The exact four rows admitted from the public Resource API.
    pub resources: Wave6ResourceSet,
    /// Whether all four resources reached Ready.
    pub ready: bool,
    /// Whether all four running effects were adopted after restart.
    pub adopted_after_restart: bool,
    /// Whether dependency-safe removal completed.
    pub removed: bool,
    /// Whether Device TPM state survived finalization.
    pub device_state_retained: bool,
}

/// Select exactly one resource of every Wave 6 kind through the public API.
pub async fn select_wave6_resources(
    client: &ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>,
) -> Result<Wave6ResourceSet, Wave6BoundaryError> {
    let mut resources = BTreeMap::new();
    for kind in [
        Wave6ResourceKind::Volume,
        Wave6ResourceKind::Network,
        Wave6ResourceKind::DeviceTpm,
        Wave6ResourceKind::CloudHypervisorGuest,
    ] {
        Wave6ResourceSet::insert(
            &mut resources,
            kind,
            select_authenticated_resource(client, kind).await?,
        );
    }
    Wave6ResourceSet::from_resources(resources)
}

/// Select exactly one resource through an already authenticated Resource API
/// client. The caller supplies only the closed acceptance kind; identity and
/// provider routing come from the committed response.
pub async fn select_authenticated_resource(
    client: &ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>,
    kind: Wave6ResourceKind,
) -> Result<Wave6Resource, Wave6BoundaryError> {
    let mut request = wire::ListRequest::new();
    let mut meta = wire::RequestMeta::new();
    meta.operation_id = kind.operation_id().to_owned();
    meta.correlation_id = meta.operation_id.clone();
    request.meta = MessageField::some(meta);
    request.resource_types.push(kind.resource_type().to_owned());
    let mut projection = wire::Projection::new();
    projection.kind = EnumOrUnknown::new(wire::ProjectionKind::PROJECTION_KIND_FULL);
    request.projection = MessageField::some(projection);
    let response = client.list(request).await;
    if response.error.is_some() || response.resources.len() != 1 {
        return Err(Wave6BoundaryError::ResourceSelection);
    }
    let envelope = response
        .resources
        .into_iter()
        .next()
        .ok_or(Wave6BoundaryError::ResourceSelection)?;
    let identity = envelope
        .identity
        .0
        .ok_or(Wave6BoundaryError::ResourceSelection)?;
    let resource_type = identity.resource_type;
    if resource_type != kind.resource_type() {
        return Err(Wave6BoundaryError::ResourceSelection);
    }
    let resource_ref = ResourceRef::parse(&format!("{resource_type}/{}", identity.name))
        .map_err(|_| Wave6BoundaryError::ResourceSelection)?;
    let uid = ResourceUid::parse(
        identity
            .uid
            .ok_or(Wave6BoundaryError::ResourceSelection)?,
    )
    .map_err(|_| Wave6BoundaryError::ResourceSelection)?;
    let generation = ResourceGeneration::new(
        identity
            .generation
            .ok_or(Wave6BoundaryError::ResourceSelection)?,
    )
    .map_err(|_| Wave6BoundaryError::ResourceSelection)?;
    let provider_ref = provider_ref_from_canonical(&envelope.canonical_json)?;
    Ok(Wave6Resource {
        resource_ref,
        uid,
        generation,
        provider_ref,
        canonical_json: envelope.canonical_json,
    })
}

fn provider_ref_from_canonical(canonical_json: &[u8]) -> Result<ResourceRef, Wave6BoundaryError> {
    let value: Value =
        serde_json::from_slice(canonical_json).map_err(|_| Wave6BoundaryError::ProviderRoute)?;
    let provider = value
        .get("spec")
        .and_then(|spec| spec.get("providerRef"))
        .and_then(Value::as_str)
        .ok_or(Wave6BoundaryError::ProviderRoute)?;
    let provider_ref =
        ResourceRef::parse(provider).map_err(|_| Wave6BoundaryError::ProviderRoute)?;
    if provider_ref.resource_type().as_str() != "Provider" {
        return Err(Wave6BoundaryError::ProviderRoute);
    }
    Ok(provider_ref)
}
