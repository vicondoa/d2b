//! Daemon-side Endpoint family effects.
//!
//! The Endpoint family crate owns the driver and its effect port; this module
//! implements that port over the preserved endpoint realization: the host
//! socket effect for the binding-owned virtiofsd socket, the guest VMM
//! evidence for the guest-runtime control endpoints, the producer-row
//! evidence for the Device TPM worker sockets, and the per-provider purpose
//! derivations the family classifies against. The derivations read the
//! declaring providers' own vocabularies (the Cloud Hypervisor provider's
//! child roles and the Device TPM Provider's declared purposes), so the
//! closed admission set cannot drift from the children those providers
//! commit.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::endpoint::EndpointClass;
use d2b_provider_endpoint::{
    EndpointDriverEffects, EndpointPurposeVocabulary, GuestControlProducer,
};

/// The producer the Cloud Hypervisor provider's own child-role vocabulary
/// declares for one guest-runtime control purpose, or `None` for any other
/// purpose. Derived from the provider's role list, so the closed family
/// cannot drift from the children a guest's provider controller commits.
pub(crate) fn guest_control_producer(purpose: &str) -> Option<GuestControlProducer> {
    use d2b_provider_guest_cloud_hypervisor::ChildRole;
    for (role, producer) in [
        (ChildRole::ChApiEndpoint, GuestControlProducer::VmmProcess),
        (ChildRole::GuestControlEndpoint, GuestControlProducer::Guest),
    ] {
        if role.purpose() == Some(purpose) {
            return Some(producer);
        }
    }
    None
}

/// Whether one purpose belongs to the guest-runtime control family.
pub(crate) fn guest_control_purpose(purpose: &str) -> bool {
    guest_control_producer(purpose).is_some()
}

/// The Endpoint class the Device TPM Provider declares for one of its
/// device-worker purposes, or `None` for any other purpose.
///
/// Derived from the provider's own constants (`TPM_SERVER_ENDPOINT_PURPOSE`,
/// `TPM_CONTROL_ENDPOINT_PURPOSE`), so the closed realization set cannot
/// drift from the rows the provider declares in its projection.
pub(crate) fn device_worker_endpoint_class(purpose: &str) -> Option<EndpointClass> {
    match purpose {
        d2b_provider_device_tpm::TPM_SERVER_ENDPOINT_PURPOSE => Some(EndpointClass::Device),
        d2b_provider_device_tpm::TPM_CONTROL_ENDPOINT_PURPOSE => Some(EndpointClass::Control),
        _ => None,
    }
}

/// Whether one purpose belongs to the device-worker family.
pub(crate) fn device_worker_purpose(purpose: &str) -> bool {
    device_worker_endpoint_class(purpose).is_some()
}

/// Boxed future returned by the production presence probe: resolving the
/// socket target is store-backed (the registry loads derived-child rows from
/// the authority on a miss), so the port cannot be a sync closure.
pub(crate) type SocketPresenceFuture<'a> = Pin<Box<dyn Future<Output = bool> + Send + 'a>>;

/// Boxed socket-presence probe closure (one producer's socket state).
pub(crate) type SocketPresenceEffect = Arc<
    dyn for<'a> Fn(&'a ResourceRef, &'a str) -> SocketPresenceFuture<'a> + Send + Sync,
>;

/// A boxed async socket effect (ensure or remove).
#[async_trait::async_trait]
pub(crate) trait AsyncSocketEffect: Send + Sync {
    async fn run(&self, producer_ref: &ResourceRef, purpose: &str) -> Result<(), String>;
}

/// Production effects over the preserved endpoint realization: the daemon's
/// effect executors plus the purpose derivations the family port answers
/// with.
pub(crate) struct ProductionEndpointDriverEffects {
    present: SocketPresenceEffect,
    ensure: Arc<dyn AsyncSocketEffect + Send + Sync>,
    remove: Arc<dyn AsyncSocketEffect + Send + Sync>,
}

impl ProductionEndpointDriverEffects {
    pub(crate) fn new(
        present: SocketPresenceEffect,
        ensure: Arc<dyn AsyncSocketEffect + Send + Sync>,
        remove: Arc<dyn AsyncSocketEffect + Send + Sync>,
    ) -> Self {
        Self { present, ensure, remove }
    }
}

impl EndpointPurposeVocabulary for ProductionEndpointDriverEffects {
    fn guest_control_producer(&self, purpose: &str) -> Option<GuestControlProducer> {
        guest_control_producer(purpose)
    }

    fn device_worker_endpoint_class(&self, purpose: &str) -> Option<EndpointClass> {
        device_worker_endpoint_class(purpose)
    }
}

#[async_trait::async_trait]
impl EndpointDriverEffects for ProductionEndpointDriverEffects {
    async fn socket_present(&self, producer_ref: &ResourceRef, purpose: &str) -> bool {
        (self.present)(producer_ref, purpose).await
    }

    async fn ensure_socket(
        &self,
        producer_ref: &ResourceRef,
        purpose: &str,
    ) -> Result<(), String> {
        self.ensure.run(producer_ref, purpose).await
    }

    async fn remove_socket(
        &self,
        producer_ref: &ResourceRef,
        purpose: &str,
    ) -> Result<(), String> {
        self.remove.run(producer_ref, purpose).await
    }
}

#[cfg(test)]
mod tests {
    use d2b_contracts_resource::v3::endpoint::EndpointClass;
    use d2b_provider_endpoint::GuestControlProducer;

    use super::{device_worker_endpoint_class, device_worker_purpose, guest_control_purpose};

    /// The derived families are exactly the declaring providers' own
    /// vocabularies: the Cloud Hypervisor child-role purposes and the Device
    /// TPM Provider's declared worker-socket purposes.
    #[test]
    fn the_derived_families_follow_the_declaring_providers() {
        assert_eq!(
            super::guest_control_producer("ch-api"),
            Some(GuestControlProducer::VmmProcess)
        );
        assert_eq!(
            super::guest_control_producer("guest-control"),
            Some(GuestControlProducer::Guest)
        );
        assert_eq!(
            GuestControlProducer::VmmProcess.resource_type(),
            d2b_provider_guest_cloud_hypervisor::ChildRole::VmmProcess.resource_type(),
            "the family's VMM producer is the Process the provider commits"
        );
        assert!(guest_control_purpose("ch-api"));
        assert!(guest_control_purpose("guest-control"));
        assert!(!guest_control_purpose("virtiofsd"));
        assert!(!guest_control_purpose("aca-sandbox-agent"));

        assert_eq!(
            device_worker_endpoint_class(d2b_provider_device_tpm::TPM_SERVER_ENDPOINT_PURPOSE),
            Some(EndpointClass::Device)
        );
        assert_eq!(
            device_worker_endpoint_class(d2b_provider_device_tpm::TPM_CONTROL_ENDPOINT_PURPOSE),
            Some(EndpointClass::Control)
        );
        assert!(device_worker_purpose("swtpm-tpm-socket"));
        assert!(device_worker_purpose("swtpm-control-socket"));
        assert!(!device_worker_purpose("virtiofsd"));
        assert!(!device_worker_purpose("ch-api"));
    }
}
