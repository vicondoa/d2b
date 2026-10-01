//! The provider-owned implementation of the `NetworkBinding` family's driver
//! effects (U6): the family serves its effects from this crate instead of a
//! daemon-built port.
//!
//! One value is the driver's typed seam, [`NetworkBindingDriverEffects`]: the
//! factory builds it from the same declared facets the composition root
//! supplies, so the driver holds no daemon state type and no externally built
//! port appears at any construction site (R2).
//!
//! The family declares no hosted zone-plane service. Its only cross-provider
//! surface is the driver's own effect port, which the daemon drives through the
//! registered driver factory; a `ServiceDecl` with no host behind it would be
//! a surface nothing can reach, so none is declared.

use std::sync::Arc;

use crate::driver::{
    FabricDrain, FabricMembership, FabricMembershipState, FabricRelease,
    NetworkBindingDriverEffects,
};
use crate::facets::NetworkBindingEffectFacets;

/// The `NetworkBinding` family's effects, built from one zone's daemon-supplied
/// facets.
///
/// Every effect the driver needs - what the fabric holds, the join, the drain,
/// and the release - is served from here; the crate itself holds no host state.
pub struct NetworkBindingEffectsService {
    observe: Arc<dyn crate::facets::FabricObserveSource>,
    join: Arc<dyn crate::facets::FabricJoinSource>,
    drain: Arc<dyn crate::facets::FabricDrainSource>,
    release: Arc<dyn crate::facets::FabricReleaseSource>,
}

impl NetworkBindingEffectsService {
    /// Build the effects from one zone's daemon-supplied facet set (R2): every
    /// daemon-structural read and write rides a facet, never a daemon handle.
    pub fn new(facets: NetworkBindingEffectFacets) -> Self {
        Self {
            observe: facets.observe,
            join: facets.join,
            drain: facets.drain,
            release: facets.release,
        }
    }
}

#[async_trait::async_trait]
impl NetworkBindingDriverEffects for NetworkBindingEffectsService {
    async fn observe_membership(
        &self,
        membership: &FabricMembership,
    ) -> Result<FabricMembershipState, String> {
        self.observe.observe(membership).await
    }

    async fn join_membership(
        &self,
        membership: &FabricMembership,
    ) -> Result<FabricMembershipState, String> {
        self.join.join(membership).await
    }

    async fn drain_membership(
        &self,
        membership: &FabricMembership,
    ) -> Result<FabricDrain, String> {
        self.drain.drain(membership).await
    }

    async fn leave_membership(
        &self,
        membership: &FabricMembership,
    ) -> Result<FabricRelease, String> {
        self.release.release(membership).await
    }
}

#[cfg(test)]
mod tests {
    use d2b_contracts_resource::v3::{
        NetworkPresentation, ResourceGeneration, ResourceRef, ResourceUid, ZoneId,
    };
    use d2b_resource_runtime::identity::ResourceKey;

    use super::*;
    use crate::driver::{MembershipIdentity, NETWORK_BINDING_PROVIDER_REF};


    /// The shared call log every scripted facet in these tests appends to.
    #[derive(Clone, Default)]
    struct SharedScript(Arc<tokio::sync::Mutex<Vec<String>>>);

    impl SharedScript {
        async fn record(&self, entry: &str) {
            self.0.lock().await.push(entry.to_owned());
        }

        async fn entries(&self) -> Vec<String> {
            self.0.lock().await.clone()
        }
    }

    #[derive(Clone)]
    struct Membership(Arc<FabricMembership>);

    impl std::ops::Deref for Membership {
        type Target = FabricMembership;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    fn membership() -> Membership {
        let identity = MembershipIdentity {
            row: ResourceKey::new("work", "NetworkBinding", "membership"),
            row_uid: uid(1),
            row_generation: ResourceGeneration::new(1).expect("generation"),
            zone_uid: uid(9),
            network_ref: reference("Network/lan"),
            network_uid: uid(2),
            network_generation: ResourceGeneration::new(4).expect("generation"),
            target_ref: reference("Guest/guest-a"),
            consumer_uid: uid(3),
            presentation: NetworkPresentation::SharedFabric,
        };
        Membership(Arc::new(
            FabricMembership::derive(identity).expect("the identity derives"),
        ))
    }

    fn uid(byte: u8) -> ResourceUid {
        ResourceUid::from_bytes(&[byte; 16]).expect("canonical uid")
    }

    fn reference(value: &str) -> ResourceRef {
        ResourceRef::parse(value).expect("registered resource reference")
    }

    struct Observe(SharedScript);
    struct Join(SharedScript);
    struct Drain(SharedScript);
    struct Release(SharedScript);

    #[async_trait::async_trait]
    impl crate::facets::FabricObserveSource for Observe {
        async fn observe(
            &self,
            membership: &FabricMembership,
        ) -> Result<FabricMembershipState, String> {
            assert_eq!(membership.network_generation.get(), 4);
            self.0.record("observe").await;
            Ok(FabricMembershipState::absent())
        }
    }

    #[async_trait::async_trait]
    impl crate::facets::FabricJoinSource for Join {
        async fn join(
            &self,
            _membership: &FabricMembership,
        ) -> Result<FabricMembershipState, String> {
            self.0.record("join").await;
            Ok(FabricMembershipState {
                joined: true,
                fabric_generation: Some(ResourceGeneration::new(4).expect("generation")),
                interface_ready: true,
            })
        }
    }

    #[async_trait::async_trait]
    impl crate::facets::FabricDrainSource for Drain {
        async fn drain(&self, _membership: &FabricMembership) -> Result<FabricDrain, String> {
            self.0.record("drain").await;
            Ok(FabricDrain {
                fenced: true,
                drained: true,
            })
        }
    }

    #[async_trait::async_trait]
    impl crate::facets::FabricReleaseSource for Release {
        async fn release(
            &self,
            _membership: &FabricMembership,
        ) -> Result<FabricRelease, String> {
            self.0.record("release").await;
            Ok(FabricRelease::fabric_released())
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn every_effect_rides_its_declared_facet() {
        let script = SharedScript::default();
        let zone = ZoneId::parse("work").expect("zone");
        assert_eq!(zone.as_str(), "work");
        assert_eq!(NETWORK_BINDING_PROVIDER_REF, "Provider/network-local");
        let service = NetworkBindingEffectsService::new(NetworkBindingEffectFacets {
            observe: Arc::new(Observe(script.clone())),
            join: Arc::new(Join(script.clone())),
            drain: Arc::new(Drain(script.clone())),
            release: Arc::new(Release(script.clone())),
        });
        let membership = membership();

        assert!(!service.observe_membership(&membership).await.expect("observe").joined);
        let joined = service.join_membership(&membership).await.expect("join");
        assert!(joined.is_held_under(ResourceGeneration::new(4).expect("generation")));
        assert!(service
            .drain_membership(&membership)
            .await
            .expect("drain")
            .is_complete());
        assert!(
            !service
                .leave_membership(&membership)
                .await
                .expect("release")
                .fabric_retained
        );

        assert_eq!(
            script.entries().await,
            vec!["observe", "join", "drain", "release"]
        );
    }
}
