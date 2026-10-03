//! Test-support recording doubles for the `Device` driver effect port.
//!
//! Gated behind the `test-support` Cargo feature (or `cfg(test)`) so
//! production consumers never pull this in; `d2bd`'s plane tests read the
//! recording doubles through this module.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    BindingAuthorization, BindingLifecycleState, BindingSlot, DeviceAttachmentMode,
    DeviceBindingRequest, DeviceBindingSpec, DesiredDigest, DesiredRevision, FreshnessTuple,
    ResourceRef, StoreIncarnation,
};

use d2b_provider_toolkit::{
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectPhase,
    SharedProviderEffectRequest, SharedProviderFinalize,
};

use crate::binding::{DeviceBindingEvidence, DeviceInventory, DeviceInventoryEntry, DevicePresence};
use crate::driver::{
    DeviceComponent, DeviceResourceState, component_for_provider, declared_device_functions,
};
use crate::facets::{
    DeviceBindingAuthoritySource, DeviceEffectFacets, DeviceInventorySource, DeviceRuntime,
};

/// The store incarnation every recorded authority fence is written against.
const RECORDED_STORE: &str = "store-one";

/// Recording [`DeviceInventorySource`] double: resolves the declared
/// vocabulary of a Device row to present capabilities with distinct
/// authority keys, so a test can drive binding admission without a host
/// inventory.
#[derive(Default)]
pub struct RecordingInventory;

impl RecordingInventory {
    /// The inventory this double resolves for one Provider reference, using
    /// the same path the effect itself takes.
    ///
    /// Exposing it lets a test compare a driver's published authority against
    /// the one this double mints, rather than against a literal the test and
    /// the double could drift apart on.
    pub fn resolved_for(
        &self,
        provider_ref: &str,
    ) -> Result<DeviceInventory, SharedProviderEffectError> {
        let component = component_for_provider(provider_ref)
            .ok_or(SharedProviderEffectError::InvalidResource)?;
        let spec = recorded_spec(component);
        DeviceInventory::new(
            declared_device_functions(component, &spec)
                .into_iter()
                .enumerate()
                .map(|(index, function)| {
                    DeviceInventoryEntry::new(
                        function,
                        d2b_contracts_resource::v3::DeviceAuthorityKey::from_core(
                            [index as u8 + 1; 32],
                        ),
                        d2b_contracts_resource::v3::DeviceAuthorityArbitration::Exclusive,
                        DevicePresence::Present,
                    )
                })
                .collect(),
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)
    }
}

#[async_trait]
impl DeviceInventorySource for RecordingInventory {
    async fn device_inventory(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<DeviceInventory, SharedProviderEffectError> {
        let provider_ref = request
            .spec
            .get("providerRef")
            .and_then(serde_json::Value::as_str)
            .ok_or(SharedProviderEffectError::InvalidResource)?;
        let component =
            component_for_provider(provider_ref).ok_or(SharedProviderEffectError::InvalidResource)?;
        let spec = recorded_spec(component);
        let entries = declared_device_functions(component, &spec)
            .into_iter()
            .enumerate()
            .map(|(index, function)| {
                DeviceInventoryEntry::new(
                    function,
                    d2b_contracts_resource::v3::DeviceAuthorityKey::from_core([index as u8 + 1; 32]),
                    d2b_contracts_resource::v3::DeviceAuthorityArbitration::Exclusive,
                    DevicePresence::Present,
                )
            })
            .collect();
        DeviceInventory::new(entries)
            .map_err(|_| SharedProviderEffectError::InvalidResource)
    }
}

/// The recorded minimal Device spec for one family.
pub fn recorded_spec(component: DeviceComponent) -> d2b_contracts_resource::v3::DeviceSpec {
    use d2b_contracts_resource::v3::{
        DeviceArbitration, DeviceClass, DeviceSpec, InventorySelector, InventorySpec,
        execution_policy::BoundedToken,
    };
    let label = BoundedToken::parse("recorded").expect("bounded token");
    let selector = match component {
        DeviceComponent::Tpm => Some(InventorySelector::Tpm { label, index: 0 }),
        DeviceComponent::Usbip => Some(InventorySelector::Usb {
            label,
            vendor_id: None,
            product_id: None,
            serial: None,
        }),
        DeviceComponent::SecurityKey => Some(InventorySelector::Hidraw {
            label,
            vendor_id: None,
            product_id: None,
            serial: None,
        }),
        DeviceComponent::Gpu => Some(InventorySelector::Drm { label, pci_slot: None }),
    };
    DeviceSpec::new(
        DeviceClass::Physical,
        DeviceArbitration::Exclusive,
        1,
        InventorySpec::new(selector),
    )
    .expect("the recorded Device spec is always valid")
}

/// A [`DeviceInventorySource`] over one explicitly built inventory.
///
/// The recording double above answers with the family's whole declared
/// vocabulary present. This one serves exactly the observation a test hands
/// it, so a test can observe a capability the host no longer backs without
/// restating the host device-node matrix.
#[derive(Clone)]
pub struct FixedInventory {
    inventory: DeviceInventory,
}

impl FixedInventory {
    /// Serve this inventory for every row.
    pub fn new(inventory: DeviceInventory) -> Self {
        Self { inventory }
    }
}

#[async_trait]
impl DeviceInventorySource for FixedInventory {
    async fn device_inventory(
        &self,
        _request: &SharedProviderEffectRequest<'_>,
    ) -> Result<DeviceInventory, SharedProviderEffectError> {
        Ok(self.inventory.clone())
    }
}

/// Recording [`DeviceBindingAuthoritySource`] double: reconstructs the
/// canonical request from the committed row's own spec, grants it against a
/// fence over that same row, and reports the lifecycle the test hands it.
///
/// This is a stand-in for the authority journal, not a grant the family mints
/// for itself: the request is read back out of the committed bytes rather than
/// composed here, so a driver that admitted something the row does not say
/// still fails. The lifecycle is the recording double's one knob, which is what
/// lets a test observe the difference between a proven and an unproven
/// relationship without restating the host device-node matrix.
#[derive(Clone, Copy)]
pub struct RecordedAuthority {
    lifecycle: BindingLifecycleState,
}

impl RecordedAuthority {
    /// Report this lifecycle for every committed relationship.
    pub const fn new(lifecycle: BindingLifecycleState) -> Self {
        Self { lifecycle }
    }

    /// Report the one lifecycle whose presence is proven: the relationship's
    /// effect is standing.
    pub const fn active() -> Self {
        Self::new(BindingLifecycleState::Active)
    }
}

#[async_trait]
impl DeviceBindingAuthoritySource for RecordedAuthority {
    async fn binding_evidence(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<DeviceBindingEvidence, SharedProviderEffectError> {
        let Ok(binding) = serde_json::from_value::<DeviceBindingSpec>(request.spec.clone()) else {
            return Err(SharedProviderEffectError::InvalidResource);
        };
        let canonical = DeviceBindingRequest::new(
            binding.device_ref().clone(),
            binding.execution_ref().clone(),
            BindingSlot::parse(binding.slot().as_str())
                .map_err(|_| SharedProviderEffectError::InvalidResource)?,
            binding.function().clone(),
            *binding.claim(),
            DeviceAttachmentMode::Descriptor,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        // The fence is over the committed row itself, which is what the journal
        // would hold: the relationship, at its store-assigned identity, in this
        // store incarnation.
        let subject_name = format!("{}/{}", request.target.type_name, request.target.name);
        let subject = ResourceRef::parse(&subject_name)
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let fence = FreshnessTuple::new(
            request.zone.clone(),
            StoreIncarnation::parse(RECORDED_STORE)
                .map_err(|_| SharedProviderEffectError::InvalidResource)?,
            subject.clone(),
            request.uid.clone(),
            DesiredRevision::INITIAL,
            DesiredDigest::of(subject.to_canonical_string().as_bytes()),
        );
        Ok(DeviceBindingEvidence::new(
            canonical,
            BindingAuthorization::granted(),
            vec![fence],
            self.lifecycle,
        ))
    }
}

/// Recording [`DeviceRuntime`] double: answers Pending/Complete for every
/// effect call and records the driven components, so `d2bd`'s plane tests
/// can build a facet set without a daemon.
///
/// Both recorders are async locks (`tokio::sync::Mutex`, awaited): every
/// reader and writer here is an async effect method or an async test, so no
/// synchronous accessor forces a blocking lock.
#[derive(Default)]
pub struct RecordingRuntime {
    /// Components reconciled, in call order.
    pub reconciled: tokio::sync::Mutex<Vec<DeviceComponent>>,
    /// Components finalized, in call order.
    pub finalized: tokio::sync::Mutex<Vec<DeviceComponent>>,
}

#[async_trait]
impl DeviceRuntime for RecordingRuntime {
    async fn reconcile_device(
        &self,
        component: DeviceComponent,
        _request: &SharedProviderEffectRequest<'_>,
        _state: &DeviceResourceState,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        self.reconciled.lock().await.push(component);
        Ok(SharedProviderEffectOutcome::phase(
            SharedProviderEffectPhase::Pending,
        ))
    }

    async fn finalize_device(
        &self,
        component: DeviceComponent,
        _request: &SharedProviderEffectRequest<'_>,
        _state: &DeviceResourceState,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.finalized.lock().await.push(component);
        Ok(SharedProviderFinalize::Complete)
    }
}

/// Build a Device facet set from a recording runtime double.
pub fn recording_facets(runtime: Arc<RecordingRuntime>) -> DeviceEffectFacets {
    device_facets(runtime, Arc::new(RecordingInventory), Arc::new(RecordedAuthority::active()))
}

/// Build a Device facet set whose inventory is the caller's own observation.
pub fn fixed_facets(
    runtime: Arc<RecordingRuntime>,
    inventory: DeviceInventory,
) -> DeviceEffectFacets {
    device_facets(
        runtime,
        Arc::new(FixedInventory::new(inventory)),
        Arc::new(RecordedAuthority::active()),
    )
}

/// Build a Device facet set over the caller's own inventory and authority
/// observations.
///
/// This is the seam a test drives presence through: the inventory facet answers
/// which devices the host backs, and the authority facet answers whether the
/// relationship's effect is proven. Both are read through the same production
/// construction the composition root uses.
pub fn device_facets(
    runtime: Arc<RecordingRuntime>,
    inventory: Arc<dyn DeviceInventorySource>,
    authority: Arc<dyn DeviceBindingAuthoritySource>,
) -> DeviceEffectFacets {
    DeviceEffectFacets {
        runtime,
        inventory,
        authority,
    }
}
