//! The Process family through the plane's registry.
//!
//! Both member types register from the family's own declarations, the
//! descriptor carries the decoder the manager wires per type, and the factory
//! builds a driver for either type once the declaration path is used.

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_resource::v3::process::{EphemeralProcessSpec, ProcessSpec};
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, ZoneId};
use d2b_process_conformance::AdoptionCandidate;
use d2b_provider_process::{
    ExecutionMode, ProcessDriverArgs, ProcessEffectFacets, ProcessFamilySpec,
    ProcessProviderRuntime, ProcessResourceContext, ProcessResourceIdentity, ProviderAdoption,
    ProviderLaunch, ProviderLiveness, process_family_descriptors,
};
use d2b_resource_runtime::identity::ResourceKey;
use d2b_resource_runtime::provider::{DriverRegistration, ProviderDirectory};

/// A runtime facet that refuses every effect: this test proves the
/// declaration and registration path, never a launch. The reconciliation
/// surface is unreachable in these tests, so the bundle-facing reads
/// refuse loudly rather than inventing trusted data.
struct RefusingRuntime;

#[async_trait::async_trait]
impl ProcessProviderRuntime for RefusingRuntime {
    fn bundle(&self) -> &d2b_core::bundle_resolver::BundleResolver {
        unreachable!("the registration tests never reconcile a row")
    }

    fn socket_runtime_dir(&self) -> &std::path::Path {
        unreachable!("the registration tests never reconcile a row")
    }

    fn guest_setup_descriptor_digest(
        &self,
        _zone: &ZoneId,
        _guest_ref: &ResourceRef,
    ) -> Option<d2b_contracts_resource::v3::SchemaFingerprint> {
        None
    }

    async fn resolve_device_worker_launch(
        &self,
        _ctx: &mut d2b_resource_runtime::context::ResourceContext,
        _identity: &ProcessResourceIdentity,
        _spec: &ProcessFamilySpec,
    ) -> Result<Option<d2b_provider_process::DeviceWorkerLaunch>, &'static str> {
        Err("refused")
    }

    async fn launch_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &ProcessSpec,
        _timeout: Duration,
    ) -> Result<ProviderLaunch, String> {
        Err("refused".to_owned())
    }

    async fn launch_ephemeral_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &EphemeralProcessSpec,
        _timeout: Duration,
    ) -> Result<ProviderLaunch, String> {
        Err("refused".to_owned())
    }

    async fn adopt_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &ProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        Err("refused".to_owned())
    }

    async fn probe_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &ProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        Err("refused".to_owned())
    }

    async fn adopt_ephemeral_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &EphemeralProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        Err("refused".to_owned())
    }

    async fn probe_ephemeral_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &EphemeralProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        Err("refused".to_owned())
    }

    async fn stop_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &ProcessSpec,
        _term_timeout: Duration,
        _kill_timeout: Duration,
    ) -> Result<bool, String> {
        Err("refused".to_owned())
    }

    async fn stop_ephemeral_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &EphemeralProcessSpec,
        _term_timeout: Duration,
        _kill_timeout: Duration,
    ) -> Result<bool, String> {
        Err("refused".to_owned())
    }

    async fn stop_stale_resource(
        &self,
        _provider_ref: &ResourceRef,
        _candidate: &AdoptionCandidate,
    ) -> Result<(), String> {
        Err("refused".to_owned())
    }

    async fn finalize_resource(
        &self,
        _context: ProcessResourceContext<'_>,
    ) -> Result<(), String> {
        Err("refused".to_owned())
    }

    fn has_active_resource_in_zone(
        &self,
        _zone: &ZoneId,
        _zone_uid: Option<&ResourceUid>,
        _resource_ref: &ResourceRef,
    ) -> bool {
        false
    }
}

fn descriptors() -> [d2b_resource_types::DriverDescriptor; 2] {
    process_family_descriptors(ProcessDriverArgs {
        zone: ZoneId::parse("work").expect("zone"),
        facets: ProcessEffectFacets {
            runtime: Arc::new(RefusingRuntime),
            committed: None,
            guest_owners: None,
        },
        zone_uid: None,
        policy_revision: None,
        provider_assignment_generation: None,
        controller_generation: d2b_contracts_resource::v3::ControllerGeneration::new(1)
            .expect("controller generation"),
        guest_execution: None,
        mode: ExecutionMode::Host,
    })
}

/// One `Process` row's stored envelope, exactly as the spec store holds it.
fn process_spec_bytes() -> Vec<u8> {
    br#"{"providerRef":"Provider/system-minijail","executionRef":"Host/host-system","processClass":"worker","template":"reaction","drainTimeout":"250ms"}"#.to_vec()
}

/// Both member types register through the family's declarations: one
/// descriptor per type, both served by the family's decoder and factory.
#[test]
fn the_family_registers_both_member_types_from_its_declarations() {
    let mut providers = ProviderDirectory::new();
    for descriptor in descriptors() {
        providers
            .register_driver(&descriptor)
            .expect("the family registers");
    }

    let registered: Vec<String> = providers
        .registered_types()
        .into_iter()
        .map(|type_name| type_name.as_str().to_owned())
        .collect();
    assert_eq!(registered, vec!["EphemeralProcess", "Process"]);

    // The mask carries the presence obligation: both types are admitted by the
    // built-in and startup sources and cannot arrive late, so a plane that
    // opens without them fails closed by the registry's own contract.
    let descriptors = descriptors();
    for descriptor in &descriptors {
        assert!(
            descriptor.allowed_sources.requires_plane_registration(),
            "{} is required before the plane opens",
            descriptor.resource_type.to_resource_type_name().as_str()
        );
        assert!(
            !descriptor.exportable,
            "a process is never an export subject"
        );
    }

    providers.mark_plane_open();
}

/// The decoder a descriptor carries is the family's own: it decodes a stored
/// row envelope and refuses one that is not a resource spec.
#[test]
fn the_declared_decoder_decodes_the_family_rows() {
    for descriptor in descriptors() {
        let decoder = DriverRegistration::decoder(&descriptor);
        decoder
            .decode(&process_spec_bytes())
            .expect("a stored Process row envelope decodes");
        assert!(
            decoder.decode(b"{not-json").is_err(),
            "an unreadable envelope is refused"
        );
    }
}

/// The registry serves the family's factory per type, and a second
/// registration for the same type is refused rather than clobbering the first.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn the_registry_serves_one_factory_per_member_type() {
    let descriptors = descriptors();
    let mut providers = ProviderDirectory::new();
    providers
        .register_driver(&descriptors[0])
        .expect("the first registration succeeds");
    assert!(
        providers.register_driver(&descriptors[0]).is_err(),
        "one driver per resource type"
    );

    let key = ResourceKey::new("work", "Process", "worker");
    providers
        .create_driver(&key)
        .await
        .expect("the registered factory builds the driver");
    assert!(
        providers
            .create_driver(&ResourceKey::new("work", "Volume", "data"))
            .await
            .is_err(),
        "an unregistered type has no driver"
    );
}

// ---------------------------------------------------------------------------
// The one resolved plan (U12)
// ---------------------------------------------------------------------------

/// The family's own registry path does not change what a Process and an
/// EphemeralProcess are: both resolve one plan through one policy path, both
/// prepare their bindings against the committed consumer identity before it
/// runs, and neither has a kind-specific branch the other lacks (AE20, AE28).
#[test]
fn both_member_types_share_one_resolved_plan_path() {
    d2b_process_conformance::suite::assert_one_policy_path_for_both_lifetimes();
    d2b_process_conformance::suite::assert_preparation_completes_before_the_consumer_runs();
}

/// A restart or adoption matches the executable, resource, Provider, policy,
/// and binding evidence rather than cached readiness, and a failed launch
/// releases only its own prepared effects (R41, R42).
#[test]
fn launch_evidence_and_failure_release_are_bounded_by_the_plan() {
    d2b_process_conformance::suite::assert_adoption_matches_every_launch_evidence_fact();
    d2b_process_conformance::suite::assert_failed_launch_releases_only_its_own_effects();
}

/// A supplied launch argument cannot replace a binding-selected source: the
/// resolved plan's destinations and sources come from the broker's own
/// accepted graph, and the screen refuses rather than silently drops.
#[test]
fn supplied_arguments_cannot_redirect_a_source() {
    d2b_process_conformance::suite::assert_supplied_arguments_cannot_redirect_a_source();
}

// ---------------------------------------------------------------------------
// The launch binding gate (U4, KTD6, R18)
// ---------------------------------------------------------------------------

/// The fixtures for the launch gate.
///
/// Every expectation here is a function of COMMITTED facts: an endpoint row, a
/// derived canonical relationship row, the authorization and dependency facts
/// the derivation was read at, and the opaque realization-incarnation token
/// the endpoint published. Nothing is host-shaped, so nothing in this module
/// can leak a socket, an inode, or a host error.
mod binding_gate {
    use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
    use d2b_process_conformance::BindingPreparation;
    use d2b_provider_process::effects::{
        BindingAuthorityLease, BindingDeliveryEvidence, BindingEvidenceFault, BindingGateError,
        ExpectedBindingRow, ObservedBinding, ProcessBindingPreparation,
        resolve_process_binding_preparation,
    };

    const ZONE: &str = "work";
    const CONSUMER: &str = "Process/display-host-proxy-abc";
    const ENDPOINT: &str = "Endpoint/display-compositor-abc";
    const BINDING: &str = "EndpointBinding/endpoint-binding-abc";
    const INCARNATION: &str = "incarnation-1111111111111111";
    const SLOT: &str = "endpoint-slot-222222222222";

    fn uid() -> ResourceUid {
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("consumer uid")
    }

    fn reference(value: &str) -> ResourceRef {
        ResourceRef::parse(value).expect("resource reference")
    }

    /// One expected relationship, derived from publication intent.
    fn expected() -> ExpectedBindingRow {
        ExpectedBindingRow::new(
            reference(BINDING),
            reference(ENDPOINT),
            2,
            3,
            reference(CONSUMER),
            SLOT.to_owned(),
            "auth-digest-a".to_owned(),
            "dependency-revision-1".to_owned(),
            INCARNATION.to_owned(),
            BindingPreparation::Prepared,
        )
        .expect("a well-formed expectation")
    }

    fn delivered(incarnation: &str, generation: u64) -> ObservedBinding {
        ObservedBinding::new(
            reference(BINDING),
            "binding-uid-1".to_owned(),
            3,
            reference(CONSUMER),
            SLOT.to_owned(),
            "auth-digest-a".to_owned(),
            "dependency-revision-1".to_owned(),
            2,
            true,
            Some(INCARNATION.to_owned()),
            Ok(BindingDeliveryEvidence::Delivered {
                generation,
                incarnation: incarnation.to_owned(),
            }),
        )
    }

    fn resolve(expected: &[ExpectedBindingRow], observed: &[ObservedBinding]) -> ProcessBindingPreparation {
        resolve_process_binding_preparation(uid(), expected, observed)
    }

    /// Scenario 4 / AE7, 10: an endpoint that publishes nothing yields an empty
    /// expected set, and an empty expected set is `NotRequired`. This is the
    /// answer for every existing non-display Process, so their preparation
    /// behavior is unchanged.
    #[test]
    fn an_empty_publication_set_is_not_required() {
        assert_eq!(
            resolve(&[], &[delivered(INCARNATION, 3)]),
            ProcessBindingPreparation::NotRequired
        );
    }

    /// Scenario 1 / AE7: an endpoint that is `Ready` while its relationship is
    /// `Undelivered` leaves the launch PENDING. Generic endpoint readiness is
    /// not delivery, and no effect may be issued.
    #[test]
    fn endpoint_ready_with_undelivered_binding_stays_pending() {
        let observed = ObservedBinding::new(
            reference(BINDING),
            "binding-uid-1".to_owned(),
            3,
            reference(CONSUMER),
            SLOT.to_owned(),
            "auth-digest-a".to_owned(),
            "dependency-revision-1".to_owned(),
            2,
            true,
            Some(INCARNATION.to_owned()),
            Ok(BindingDeliveryEvidence::Undelivered),
        );
        assert_eq!(
            resolve(&[expected()], &[observed]),
            ProcessBindingPreparation::Pending,
            "a ready endpoint over an undelivered relationship does not admit a launch"
        );
    }

    /// Scenario 2: `EndpointReplaced` blocks until a fresh `Delivered`
    /// projection appears for the same incarnation. The replacement is not
    /// read as the consumer's own delivery.
    #[test]
    fn a_replaced_endpoint_blocks_until_fresh_delivery() {
        let replaced = ObservedBinding::new(
            reference(BINDING),
            "binding-uid-1".to_owned(),
            3,
            reference(CONSUMER),
            SLOT.to_owned(),
            "auth-digest-a".to_owned(),
            "dependency-revision-1".to_owned(),
            2,
            true,
            Some(INCARNATION.to_owned()),
            Ok(BindingDeliveryEvidence::EndpointReplaced),
        );
        assert_eq!(
            resolve(&[expected()], &[replaced]),
            ProcessBindingPreparation::Pending
        );
        // Only a FRESH `Delivered` at the expected incarnation opens the gate.
        let fresh = resolve(&[expected()], &[delivered(INCARNATION, 3)]);
        assert!(
            matches!(fresh, ProcessBindingPreparation::Ready(_)),
            "a fresh Delivered at the expected incarnation admits the launch, got {fresh:?}"
        );
    }

    /// Scenario 6: a draining relationship is a fence. It never reads as a
    /// delivery, so the launch defers and issues no effect.
    #[test]
    fn a_draining_relationship_defers_the_launch() {
        let draining = ObservedBinding::new(
            reference(BINDING),
            "binding-uid-1".to_owned(),
            3,
            reference(CONSUMER),
            SLOT.to_owned(),
            "auth-digest-a".to_owned(),
            "dependency-revision-1".to_owned(),
            2,
            true,
            Some(INCARNATION.to_owned()),
            Ok(BindingDeliveryEvidence::Draining),
        );
        assert_eq!(
            resolve(&[expected()], &[draining]),
            ProcessBindingPreparation::Pending
        );
    }

    /// Scenario 4: an expected row the manager cannot answer for DEFERS. A
    /// row this launch requires and cannot see is not delivered.
    #[test]
    fn a_missing_expected_row_defers() {
        assert_eq!(
            resolve(&[expected()], &[]),
            ProcessBindingPreparation::Pending
        );
    }

    /// Scenario 3: delivery over the wrong relationship generation, consumer,
    /// authorization digest, dependency revision, or canonical slot is FOREIGN
    /// evidence and is refused rather than deferred.
    ///
    /// The relationship's store-assigned uid is deliberately NOT in this list:
    /// it is assigned by the manager, so an expectation cannot know it before
    /// the row is read, and inventing one would make every expectation a
    /// guess. It is fenced where it can be - at lease revalidation, which
    /// compares the uid that was observed when the lease was sealed (see
    /// `a_revoked_lease_refuses_the_effect`).
    #[test]
    fn delivery_over_the_wrong_authority_facts_is_refused() {
        let mut rejected = 0_usize;
        let variants: Vec<(&str, ObservedBinding)> = vec![
            (
                "relationship generation",
                ObservedBinding::new(
                    reference(BINDING),
                    "binding-uid-1".to_owned(),
                    9,
                    reference(CONSUMER),
                    SLOT.to_owned(),
                    "auth-digest-a".to_owned(),
                    "dependency-revision-1".to_owned(),
                    2,
                    true,
                    Some(INCARNATION.to_owned()),
                    Ok(BindingDeliveryEvidence::Delivered { generation: 3, incarnation: INCARNATION.to_owned() }),
                ),
            ),
            (
                "consumer",
                ObservedBinding::new(
                    reference(BINDING),
                    "binding-uid-1".to_owned(),
                    3,
                    reference("Process/someone-else"),
                    SLOT.to_owned(),
                    "auth-digest-a".to_owned(),
                    "dependency-revision-1".to_owned(),
                    2,
                    true,
                    Some(INCARNATION.to_owned()),
                    Ok(BindingDeliveryEvidence::Delivered { generation: 3, incarnation: INCARNATION.to_owned() }),
                ),
            ),
            (
                "authorization digest",
                ObservedBinding::new(
                    reference(BINDING),
                    "binding-uid-1".to_owned(),
                    3,
                    reference(CONSUMER),
                    SLOT.to_owned(),
                    "auth-digest-REVOKED".to_owned(),
                    "dependency-revision-1".to_owned(),
                    2,
                    true,
                    Some(INCARNATION.to_owned()),
                    Ok(BindingDeliveryEvidence::Delivered { generation: 3, incarnation: INCARNATION.to_owned() }),
                ),
            ),
            (
                "dependency revision",
                ObservedBinding::new(
                    reference(BINDING),
                    "binding-uid-1".to_owned(),
                    3,
                    reference(CONSUMER),
                    SLOT.to_owned(),
                    "auth-digest-a".to_owned(),
                    "dependency-revision-7".to_owned(),
                    2,
                    true,
                    Some(INCARNATION.to_owned()),
                    Ok(BindingDeliveryEvidence::Delivered { generation: 3, incarnation: INCARNATION.to_owned() }),
                ),
            ),
            (
                "canonical slot",
                ObservedBinding::new(
                    reference(BINDING),
                    "binding-uid-1".to_owned(),
                    3,
                    reference(CONSUMER),
                    "endpoint-slot-OTHER".to_owned(),
                    "auth-digest-a".to_owned(),
                    "dependency-revision-1".to_owned(),
                    2,
                    true,
                    Some(INCARNATION.to_owned()),
                    Ok(BindingDeliveryEvidence::Delivered { generation: 3, incarnation: INCARNATION.to_owned() }),
                ),
            ),
        ];
        for (label, observed) in variants {
            assert_eq!(
                resolve(&[expected()], &[observed]),
                ProcessBindingPreparation::Refused(BindingGateError::Foreign),
                "delivery over the wrong {label} is foreign evidence"
            );
            rejected += 1;
        }
        assert_eq!(rejected, 5);
    }

    /// Scenario 3 / KTD8: delivery naming a DIFFERENT realization incarnation
    /// is evidence about another realization, not a stale reading of this one.
    #[test]
    fn delivery_at_another_incarnation_is_refused() {
        assert_eq!(
            resolve(&[expected()], &[delivered("incarnation-OTHER", 3)]),
            ProcessBindingPreparation::Refused(BindingGateError::Foreign)
        );
    }

    /// Scenario 7 / AE13: an authorization-only change - a withdrawn
    /// `RoleBinding`, a consumer owner change, or a provider reassignment -
    /// moves the authorization digest while the ENDPOINT row generation stays
    /// exactly where it was. The gate sees it without an endpoint generation
    /// bump and refuses the launch.
    #[test]
    fn an_authorization_only_change_needs_no_endpoint_generation_bump() {
        let revoked = ObservedBinding::new(
            reference(BINDING),
            "binding-uid-1".to_owned(),
            3,
            reference(CONSUMER),
            SLOT.to_owned(),
            "auth-digest-REVOKED".to_owned(),
            "dependency-revision-1".to_owned(),
            2,
            true,
            Some(INCARNATION.to_owned()),
            Ok(BindingDeliveryEvidence::Delivered { generation: 3, incarnation: INCARNATION.to_owned() }),
        );
        // The endpoint is still `Ready` at the SAME generation and the SAME
        // incarnation; only the authorization moved.
        assert_eq!(
            resolve(&[expected()], &[revoked]),
            ProcessBindingPreparation::Refused(BindingGateError::Foreign)
        );
    }

    /// Scenario 8 / KTD6: authority revoked between preparation and the effect
    /// fails lease revalidation, so the effect is never issued. This is the
    /// race the plan names: preparation concluded, then the world moved.
    #[test]
    fn a_revoked_lease_refuses_the_effect() {
        let lease = match resolve(&[expected()], &[delivered(INCARNATION, 3)]) {
            ProcessBindingPreparation::Ready(lease) => lease,
            other => panic!("expected a sealed lease, got {other:?}"),
        };
        assert!(
            lease.revalidate(&[delivered(INCARNATION, 3)]).is_ok(),
            "an unchanged world revalidates"
        );

        // The relationship row was re-issued under a new identity.
        let reissued = ObservedBinding::new(
            reference(BINDING),
            "binding-uid-2".to_owned(),
            4,
            reference(CONSUMER),
            SLOT.to_owned(),
            "auth-digest-a".to_owned(),
            "dependency-revision-1".to_owned(),
            2,
            true,
            Some(INCARNATION.to_owned()),
            Ok(BindingDeliveryEvidence::Delivered { generation: 4, incarnation: INCARNATION.to_owned() }),
        );
        assert_eq!(
            lease.revalidate(&[reissued]),
            Err(BindingGateError::LeaseRevoked),
            "a re-issued relationship revokes the lease before the effect"
        );

        // Delivery was withdrawn between preparation and the effect.
        let withdrawn = ObservedBinding::new(
            reference(BINDING),
            "binding-uid-1".to_owned(),
            3,
            reference(CONSUMER),
            SLOT.to_owned(),
            "auth-digest-a".to_owned(),
            "dependency-revision-1".to_owned(),
            2,
            true,
            Some(INCARNATION.to_owned()),
            Ok(BindingDeliveryEvidence::Undelivered),
        );
        assert_eq!(lease.revalidate(&[withdrawn]), Err(BindingGateError::LeaseRevoked));

        // The relationship row is gone entirely (scenario 5: a restart must
        // not trust cached delivery).
        assert_eq!(lease.revalidate(&[]), Err(BindingGateError::LeaseRevoked));
    }

    /// Scenario 8 / AE14: the endpoint RE-REALIZED between the seal and the
    /// effect. The relationship row is untouched and its delivery projection
    /// still names the sealed incarnation, so every relationship-side fact is
    /// identical - but the endpoint itself now publishes a different
    /// realization, and the observation the gate reads is internally
    /// consistent about that new one. Only the sealed lease compares the
    /// endpoint's OWN realization against the one it sealed, which is what
    /// stops a launch over a grant the endpoint no longer holds.
    #[test]
    fn a_re_realized_endpoint_revokes_the_lease() {
        let lease = match resolve(&[expected()], &[delivered(INCARNATION, 3)]) {
            ProcessBindingPreparation::Ready(lease) => lease,
            other => panic!("expected a sealed lease, got {other:?}"),
        };
        let re_realized = ObservedBinding::new(
            reference(BINDING),
            "binding-uid-1".to_owned(),
            3,
            reference(CONSUMER),
            SLOT.to_owned(),
            "auth-digest-a".to_owned(),
            "dependency-revision-1".to_owned(),
            2,
            true,
            Some("incarnation-OTHER-REALIZATION".to_owned()),
            // The relationship still publishes a delivery at the SEALED
            // incarnation: the endpoint moved, the row did not.
            Ok(BindingDeliveryEvidence::Delivered {
                generation: 3,
                incarnation: INCARNATION.to_owned(),
            }),
        );
        assert_eq!(
            lease.revalidate(&[re_realized]),
            Err(BindingGateError::LeaseRevoked),
            "an endpoint that re-realized revokes the lease before the effect"
        );
    }

    /// Scenario 8 / AE14: the endpoint row generation moved between the seal
    /// and the effect, so the expectation this lease carries was derived from
    /// a row that no longer stands - whatever the realization and the
    /// delivery projection still say about it.
    #[test]
    fn a_moved_endpoint_row_generation_revokes_the_lease() {
        let lease = match resolve(&[expected()], &[delivered(INCARNATION, 3)]) {
            ProcessBindingPreparation::Ready(lease) => lease,
            other => panic!("expected a sealed lease, got {other:?}"),
        };
        let re_derived = ObservedBinding::new(
            reference(BINDING),
            "binding-uid-1".to_owned(),
            3,
            reference(CONSUMER),
            SLOT.to_owned(),
            "auth-digest-a".to_owned(),
            "dependency-revision-1".to_owned(),
            7,
            true,
            Some(INCARNATION.to_owned()),
            Ok(BindingDeliveryEvidence::Delivered {
                generation: 3,
                incarnation: INCARNATION.to_owned(),
            }),
        );
        assert_eq!(
            lease.revalidate(&[re_derived]),
            Err(BindingGateError::LeaseRevoked),
            "a moved endpoint row generation revokes the lease before the effect"
        );
    }

    /// Scenario 8: sealing refuses an observation set that does not line up
    /// with the expectation, so a lease can never carry a mismatch into the
    /// effect.
    #[test]
    fn sealing_refuses_a_mismatched_observation_set() {
        assert_eq!(
            BindingAuthorityLease::seal(uid(), &[expected()], &[]),
            Err(BindingGateError::Foreign)
        );
    }

    /// Scenario 4 / AE7: an evidence projection that was published and cannot
    /// be read is REFUSED, not deferred. Deferring would retry the same
    /// unreadable evidence forever; reading it as delivered would launch over
    /// evidence nothing proved.
    #[test]
    fn an_unreadable_projection_is_refused_not_deferred() {
        let unreadable = ObservedBinding::new(
            reference(BINDING),
            "binding-uid-1".to_owned(),
            3,
            reference(CONSUMER),
            SLOT.to_owned(),
            "auth-digest-a".to_owned(),
            "dependency-revision-1".to_owned(),
            2,
            true,
            Some(INCARNATION.to_owned()),
            Err(BindingEvidenceFault::Unreadable),
        );
        assert_eq!(
            resolve(&[expected()], &[unreadable]),
            ProcessBindingPreparation::Refused(BindingGateError::EvidenceUnreadable)
        );
    }

    /// An absent projection is the ordinary not-yet and DEFERS, which is the
    /// one place the two faults must differ.
    #[test]
    fn an_absent_projection_defers() {
        let absent = ObservedBinding::new(
            reference(BINDING),
            "binding-uid-1".to_owned(),
            3,
            reference(CONSUMER),
            SLOT.to_owned(),
            "auth-digest-a".to_owned(),
            "dependency-revision-1".to_owned(),
            2,
            true,
            Some(INCARNATION.to_owned()),
            Err(BindingEvidenceFault::Absent),
        );
        assert_eq!(
            resolve(&[expected()], &[absent]),
            ProcessBindingPreparation::Pending
        );
    }

    /// Evidence for a relationship this launch does not expect is another
    /// launch's, and is refused before the expected set is compared.
    #[test]
    fn evidence_for_an_unexpected_relationship_is_refused() {
        let foreign = ObservedBinding::new(
            reference("EndpointBinding/endpoint-binding-not-mine"),
            "binding-uid-9".to_owned(),
            3,
            reference(CONSUMER),
            SLOT.to_owned(),
            "auth-digest-a".to_owned(),
            "dependency-revision-1".to_owned(),
            2,
            true,
            Some(INCARNATION.to_owned()),
            Ok(BindingDeliveryEvidence::Delivered { generation: 3, incarnation: INCARNATION.to_owned() }),
        );
        assert_eq!(
            resolve(&[expected()], &[foreign]),
            ProcessBindingPreparation::Refused(BindingGateError::Foreign)
        );
    }

    /// The zone is only ever named through the fixture's own constants; this
    /// keeps the fixture honest about the boundary it derives from.
    #[test]
    fn the_fixture_is_bound_to_one_zone() {
        assert_eq!(ZONE, "work");
    }
}

// ---------------------------------------------------------------------------
// The launch binding gate, wired into the driver (U4, KTD6, R18, R21)
// ---------------------------------------------------------------------------

/// The driver-level fixtures for the wired launch gate.
///
/// The manager double publishes EXACTLY the layers the endpoint family
/// publishes - the `/endpoint/bindings` publication set and the `/binding`
/// delivery evidence - so the driver reads committed evidence rather than a
/// test-shaped shortcut. Every fact here is a function of committed rows: an
/// endpoint row, a canonical relationship row, and the opaque realization
/// token the endpoint published. Nothing is host-shaped, so no socket name, no
/// path and no `(dev, ino)` pair can enter one of these fixtures.
mod gated_launch {
    use std::sync::Arc;
    use std::time::Duration;

    use d2b_contracts_resource::v3::execution_policy::BoundedToken;
    use d2b_contracts_resource::v3::{
        BindingArbitration, BindingRealizationFacet, BindingSourceDecision, ControllerGeneration,
        EndpointAttachmentKind, EndpointBindingSpec, RequestedRights, ResourceRef, ResourceUid,
        SchemaFingerprint, ZoneId,
    };
    use d2b_process_conformance::ProcessIdentityDigest;
    use d2b_provider_process::{
        ExecutionMode, ProcessDriverArgs, ProcessEffectFacets, ProcessFamilySpec,
        ProcessProviderRuntime, ProcessResourceContext, ProcessResourceIdentity, ProviderAdoption,
        ProviderLaunch, ProviderLiveness, process_family_descriptors,
    };
    use d2b_provider_toolkit::testing::fakes::RecordingRequeue;
    use d2b_resource_runtime::context::{
        ChildEnsure, EffectCompleted, ManagerEndpoint, ResourceContext, WatchCondition, WatchId,
        WatchRegistration,
    };
    use d2b_resource_runtime::driver::{DynResourceDriver, ReconcileOutcome};
    use d2b_resource_runtime::error::ResourceError;
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};
    use d2b_resource_runtime::manager::ResourceView;
    use d2b_resource_runtime::provider::{DriverRegistration, ProviderDirectory};
    use d2b_resource_runtime::resource::ResourceStatus;
    use d2b_resource_runtime::spec_store::EnsureOutcome;

    const OWNER_UID: [u8; 16] = [0x51; 16];
    const ENDPOINT_NAME: &str = "compositor";
    const BINDING_NAME: &str = "endpoint-binding-abc";
    const CONSUMER: &str = "Process/worker";
    const SLOT: &str = "endpoint-slot-222222222222";
    const INCARNATION: &str = "incarnation-1111111111111111";
    const AUTHORIZATION: &str = "auth-digest-a";
    const REVOKED: &str = "auth-digest-REVOKED";
    const DEPENDENCY: &str = "dependency-revision-1";

    /// The runtime one gated pass runs over: adoption reports nothing live, the
    /// launch succeeds, and the probe sees the process alive. Every other
    /// effect refuses loudly rather than inventing trusted data.
    struct GatedRuntime;

    #[async_trait::async_trait]
    impl ProcessProviderRuntime for GatedRuntime {
        fn bundle(&self) -> &d2b_core::bundle_resolver::BundleResolver {
            unreachable!("the gated fixtures never resolve a launch ticket")
        }

        fn socket_runtime_dir(&self) -> &std::path::Path {
            unreachable!("the gated fixtures never resolve a launch ticket")
        }

        fn guest_setup_descriptor_digest(
            &self,
            _zone: &ZoneId,
            _guest_ref: &ResourceRef,
        ) -> Option<SchemaFingerprint> {
            None
        }

        async fn resolve_device_worker_launch(
            &self,
            _ctx: &mut ResourceContext,
            _identity: &ProcessResourceIdentity,
            _spec: &ProcessFamilySpec,
        ) -> Result<Option<d2b_provider_process::DeviceWorkerLaunch>, &'static str> {
            Err("refused")
        }

        async fn launch_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &d2b_contracts_resource::v3::process::ProcessSpec,
            _timeout: Duration,
        ) -> Result<ProviderLaunch, String> {
            Ok(ProviderLaunch {
                identity: ProcessIdentityDigest::from_bytes([0x71; 32]),
            })
        }

        async fn launch_ephemeral_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &d2b_contracts_resource::v3::process::EphemeralProcessSpec,
            _timeout: Duration,
        ) -> Result<ProviderLaunch, String> {
            Err("refused".to_owned())
        }

        async fn adopt_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &d2b_contracts_resource::v3::process::ProcessSpec,
        ) -> Result<ProviderAdoption, String> {
            Ok(ProviderAdoption::Absent)
        }

        async fn probe_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &d2b_contracts_resource::v3::process::ProcessSpec,
        ) -> Result<ProviderLiveness, String> {
            Ok(ProviderLiveness::Alive)
        }

        async fn adopt_ephemeral_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &d2b_contracts_resource::v3::process::EphemeralProcessSpec,
        ) -> Result<ProviderAdoption, String> {
            Err("refused".to_owned())
        }

        async fn probe_ephemeral_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &d2b_contracts_resource::v3::process::EphemeralProcessSpec,
        ) -> Result<ProviderLiveness, String> {
            Err("refused".to_owned())
        }

        async fn stop_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &d2b_contracts_resource::v3::process::ProcessSpec,
            _term_timeout: Duration,
            _kill_timeout: Duration,
        ) -> Result<bool, String> {
            Err("refused".to_owned())
        }

        async fn stop_ephemeral_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &d2b_contracts_resource::v3::process::EphemeralProcessSpec,
            _term_timeout: Duration,
            _kill_timeout: Duration,
        ) -> Result<bool, String> {
            Err("refused".to_owned())
        }

        async fn stop_stale_resource(
            &self,
            _provider_ref: &ResourceRef,
            _candidate: &d2b_process_conformance::AdoptionCandidate,
        ) -> Result<(), String> {
            Err("refused".to_owned())
        }

        async fn finalize_resource(
            &self,
            _context: ProcessResourceContext<'_>,
        ) -> Result<(), String> {
            Err("refused".to_owned())
        }

        fn has_active_resource_in_zone(
            &self,
            _zone: &ZoneId,
            _zone_uid: Option<&ResourceUid>,
            _resource_ref: &ResourceRef,
        ) -> bool {
            false
        }
    }

    fn gated_descriptors() -> [d2b_resource_types::DriverDescriptor; 2] {
        process_family_descriptors(ProcessDriverArgs {
            zone: ZoneId::parse("work").expect("zone"),
            facets: ProcessEffectFacets {
                runtime: Arc::new(GatedRuntime),
                committed: None,
                guest_owners: None,
            },
            zone_uid: Some(
                ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("zone uid"),
            ),
            policy_revision: None,
            provider_assignment_generation: None,
            controller_generation: ControllerGeneration::new(1).expect("controller generation"),
            guest_execution: None,
            mode: ExecutionMode::Host,
        })
    }

    fn endpoint_key() -> ResourceKey {
        ResourceKey::new("work", "Endpoint", ENDPOINT_NAME)
    }

    fn binding_key() -> ResourceKey {
        ResourceKey::new("work", "EndpointBinding", BINDING_NAME)
    }

    fn endpoint_ref() -> String {
        format!("Endpoint/{ENDPOINT_NAME}")
    }

    /// The canonical `EndpointBinding` row one delivery commits, exactly as
    /// the source's own derivation writes it.
    fn binding_spec_bytes(consumer: &str) -> Vec<u8> {
        let spec = EndpointBindingSpec::new(
            ResourceRef::parse(&endpoint_ref()).expect("endpoint ref"),
            ResourceRef::parse(consumer).expect("consumer ref"),
            EndpointAttachmentKind::Connect,
            BoundedToken::parse(SLOT).expect("slot token"),
            BindingSourceDecision::new(
                vec![RequestedRights::Consume],
                BindingArbitration::Shared,
                vec![BindingRealizationFacet::EndpointDescriptor],
            )
            .expect("source decision"),
        )
        .expect("binding spec");
        serde_json::to_vec(&spec).expect("a committed binding spec serializes")
    }

    /// The `/endpoint` layer the endpoint driver publishes at one row
    /// generation and one realization, including the `/endpoint/bindings`
    /// publication set this gate derives from.
    fn endpoint_view_at(
        generation: u64,
        incarnation: &str,
        authorization: &str,
        consumer: &str,
    ) -> ResourceView {
        view(
            endpoint_key(),
            [0x62; 16],
            generation,
            Some(serde_json::json!({
                "endpoint": {
                    "readiness": "realized",
                    "generation": generation,
                    "incarnation": incarnation,
                    "bindings": [{
                        "name": BINDING_NAME,
                        "endpoint": endpoint_ref(),
                        "consumer": consumer,
                        "slot": SLOT,
                        "authorizationDigest": authorization,
                        "dependencyRevision": DEPENDENCY,
                    }],
                },
            })),
            Vec::new(),
        )
    }

    /// The endpoint as this fixture's consumer is gated on it: generation 2,
    /// the one realization this module names.
    fn endpoint_view(authorization: &str, consumer: &str) -> ResourceView {
        endpoint_view_at(2, INCARNATION, authorization, consumer)
    }

    /// One published relationship view carrying exactly one of the delivery
    /// states the `EndpointBinding` contract publishes.
    fn binding_view(generation: u64, binding: serde_json::Value) -> ResourceView {
        view(
            binding_key(),
            [0x61; 16],
            generation,
            Some(serde_json::json!({ "binding": binding })),
            binding_spec_bytes(CONSUMER),
        )
    }

    fn delivered() -> serde_json::Value {
        serde_json::json!({ "state": "delivered", "generation": 3, "incarnation": INCARNATION })
    }

    fn draining() -> serde_json::Value {
        serde_json::json!({ "state": "draining" })
    }

    fn view(
        key: ResourceKey,
        uid: [u8; 16],
        generation: u64,
        status_projection: Option<serde_json::Value>,
        spec: Vec<u8>,
    ) -> ResourceView {
        ResourceView {
            key: key.clone(),
            uid,
            generation,
            deleting: false,
            provenance: ResourceProvenance::Resource,
            spec,
            metadata: Vec::new(),
            owner_key: None,
            status: Some(ResourceStatus::Ready),
            status_generation: Some(generation),
            status_projection,
        }
    }

    /// The committed endpoint row its owner holds.
    fn endpoint_row() -> StoredDesiredResource {
        StoredDesiredResource {
            key: endpoint_key(),
            uid: [0x62; 16],
            generation: 2,
            owner_uid: Some(OWNER_UID),
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: Vec::new(),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    /// The owner-scoped manager one gated pass reads: the committed rows its
    /// owner holds, the views those rows published, and every watch the driver
    /// registered.
    struct BindingManager {
        rows: tokio::sync::Mutex<Vec<StoredDesiredResource>>,
        views: tokio::sync::Mutex<Vec<ResourceView>>,
        /// Views served once each, in order, ahead of the committed ones. One
        /// scripted read is how a test moves the world between two reads
        /// inside a single reconcile pass.
        scripted: tokio::sync::Mutex<Vec<ResourceView>>,
        watches: tokio::sync::Mutex<Vec<(ResourceKey, WatchCondition)>>,
    }

    impl BindingManager {
        fn new(rows: Vec<StoredDesiredResource>, views: Vec<ResourceView>) -> Arc<Self> {
            Arc::new(Self {
                rows: tokio::sync::Mutex::new(rows),
                views: tokio::sync::Mutex::new(views),
                scripted: tokio::sync::Mutex::new(Vec::new()),
                watches: tokio::sync::Mutex::new(Vec::new()),
            })
        }

        /// Serve `views` once each, in order, before the committed ones.
        async fn script(self: &Arc<Self>, views: Vec<ResourceView>) {
            let mut scripted = self.scripted.lock().await;
            scripted.clear();
            scripted.extend(views);
        }

        /// Republish one row's live view, exactly as that row's own actor
        /// would when its evidence layer changes underneath a `Ready` phase.
        async fn republish(&self, view: ResourceView) {
            let mut views = self.views.lock().await;
            views.retain(|published| published.key != view.key);
            views.push(view);
        }

        async fn watches(&self) -> Vec<(ResourceKey, WatchCondition)> {
            self.watches.lock().await.clone()
        }
    }

    #[async_trait::async_trait]
    impl ManagerEndpoint for BindingManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            _child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            Err(ResourceError::ManagerRejected {
                reason: "unexpected ensure_child".into(),
            })
        }

        async fn get(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Ok(self
                .rows
                .lock()
                .await
                .iter()
                .find(|row| row.key == *key)
                .cloned())
        }

        async fn view(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
            let scripted = {
                let mut scripted = self.scripted.lock().await;
                match scripted.first() {
                    Some(next) if next.key == *key => Some(scripted.remove(0)),
                    _ => None,
                }
            };
            if scripted.is_some() {
                return Ok(scripted);
            }
            Ok(self
                .views
                .lock()
                .await
                .iter()
                .find(|view| view.key == *key)
                .cloned())
        }

        async fn delete(&self, _key: &ResourceKey) -> Result<(), ResourceError> {
            Err(ResourceError::ManagerRejected {
                reason: "unexpected delete".into(),
            })
        }

        async fn list_owned(
            &self,
            owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Ok(self
                .rows
                .lock()
                .await
                .iter()
                .filter(|row| row.owner_uid == Some(owner_uid))
                .cloned()
                .collect())
        }

        async fn list_zone_type(
            &self,
            zone: &str,
            type_name: &str,
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Ok(self
                .rows
                .lock()
                .await
                .iter()
                .filter(|row| row.key.zone == zone && row.key.type_name == type_name)
                .cloned()
                .collect())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            let mut watches = self.watches.lock().await;
            watches.push((registration.target, registration.condition));
            Ok(WatchId(watches.len() as u64))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    /// The `Process` row under test: owned by the session that also owns the
    /// endpoint, so the driver reaches the publication intent it is gated on
    /// through the owner-scoped sibling listing it already has.
    fn process_row(owner: Option<[u8; 16]>) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", "Process", "worker"),
            uid: [0x42; 16],
            generation: 3,
            owner_uid: owner,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: br#"{"providerRef":"Provider/system-minijail","executionRef":"Host/host-system","processClass":"worker","template":"reaction","drainTimeout":"250ms"}"#.to_vec(),
            // The authored owner reference the row was ingested with: an
            // owned child of a session the manager does not itself own
            // resolves its owner reference here, and the launch identity
            // refuses an owner uid with no owner reference to bind.
            metadata: if owner.is_some() {
                br#"{"annotations":{},"labels":{},"ownerRef":"display-wayland.d2bus.org.WaylandSession/display-wayland"}"#
                    .to_vec()
            } else {
                Vec::new()
            },
            created_at: 0,
        }
    }

    struct Harness {
        driver: Box<dyn DynResourceDriver>,
        ctx: ResourceContext,

        effects: tokio::sync::mpsc::UnboundedReceiver<EffectCompleted>,
        requeue: RecordingRequeue,
    }

    async fn harness(row: StoredDesiredResource, manager: Arc<BindingManager>) -> Harness {
        let descriptors = gated_descriptors();
        let key = row.key.clone();
        let mut providers = ProviderDirectory::new();
        providers
            .register_driver(&descriptors[0])
            .expect("the family registers");
        let driver = providers
            .create_driver(&key)
            .await
            .expect("the registered factory builds the driver");
        let (effects_tx, effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let requeue = RecordingRequeue::default();
        let ctx = ResourceContext::new(
            row,
            DriverRegistration::decoder(&descriptors[0]),
            manager.clone(),
            Arc::new(requeue.clone()),
            effects_tx,
            notify_tx,
        );
        Harness {
            driver,
            ctx,
            effects: effects_rx,
            requeue,
        }
    }

    /// A manager serving one published, delivered relationship for this
    /// consumer: the shape an ordinary bound Process starts from.
    fn delivered_manager() -> Arc<BindingManager> {
        BindingManager::new(
            vec![endpoint_row()],
            vec![endpoint_view(AUTHORIZATION, CONSUMER), binding_view(3, delivered())],
        )
    }

    /// Let a spawned effect task reach its send before the assertions read the
    /// mailbox.
    async fn settle() {
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    /// Scenario 6: a relationship the endpoint still publishes while its own
    /// actor reports `Draining` is a fence. The launch issues no effect, never
    /// publishes a readiness claim, and requeues.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_draining_relationship_defers_the_launch_effect() {
        let manager = BindingManager::new(
            vec![endpoint_row()],
            vec![endpoint_view(AUTHORIZATION, CONSUMER), binding_view(3, draining())],
        );
        let mut h = harness(process_row(Some(OWNER_UID)), manager).await;

        assert_eq!(
            h.driver
                .reconcile(&mut h.ctx)
                .await
                .expect("the pass reports an outcome"),
            ReconcileOutcome::RetryScheduled,
            "a draining relationship is not a delivery"
        );
        settle().await;
        assert!(h.effects.try_recv().is_err(), "no launch effect was issued");
        assert!(
            h.ctx.take_status_projection().is_none(),
            "no readiness claim rides a deferred launch"
        );
        assert_eq!(
            h.requeue.scheduled().len(),
            1,
            "the pass schedules exactly one retryable requeue"
        );
    }

    /// Scenario 7 / AE13: an authorization-only change - here a consumer owner
    /// change, so the committed relationship names someone else - is refused
    /// with the gate's closed slug and issues no effect. The endpoint row
    /// generation never moved, so a gate that compared only endpoint
    /// generations would have launched straight over it.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn an_authorization_only_change_with_no_endpoint_bump_is_refused() {
        let manager = BindingManager::new(
            vec![endpoint_row()],
            vec![
                endpoint_view(AUTHORIZATION, CONSUMER),
                view(
                    binding_key(),
                    [0x61; 16],
                    3,
                    Some(serde_json::json!({ "binding": delivered() })),
                    binding_spec_bytes("Process/someone-else"),
                ),
            ],
        );
        let mut h = harness(process_row(Some(OWNER_UID)), manager).await;

        let failure = h
            .driver
            .reconcile(&mut h.ctx)
            .await
            .expect_err("the pass refuses");
        assert!(
            failure
                .wire_layer()
                .to_string()
                .contains("process-binding-gate-foreign-evidence"),
            "the refusal carries the closed gate slug, got {}",
            failure.wire_layer()
        );
        settle().await;
        assert!(h.effects.try_recv().is_err(), "no launch effect was issued");
        assert!(
            h.requeue.scheduled().is_empty(),
            "a refused launch never enters a requeue loop"
        );
    }

    /// Scenario 8 / KTD6: the authority moved between preparation and the
    /// effect - the endpoint re-derived its authorization digest at the SAME
    /// generation - so the sealed lease no longer holds and the launch effect
    /// is never issued.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_lease_whose_authority_moved_refuses_the_effect() {
        let manager = delivered_manager();
        // Preparation seals the lease against the first read; the next read
        // inside the same pass sees the withdrawn authorization.
        manager
            .script(vec![
                endpoint_view(AUTHORIZATION, CONSUMER),
                endpoint_view(REVOKED, CONSUMER),
            ])
            .await;
        let mut h = harness(process_row(Some(OWNER_UID)), manager).await;

        assert_eq!(
            h.driver
                .reconcile(&mut h.ctx)
                .await
                .expect("the pass reports an outcome"),
            ReconcileOutcome::RetryScheduled,
            "a revoked lease issues no effect"
        );
        settle().await;
        assert!(h.effects.try_recv().is_err(), "no launch effect was issued");
        assert!(
            !h.requeue.scheduled().is_empty(),
            "the pass re-reads the evidence instead of launching"
        );
    }

    /// Scenario 8 / AE14: the endpoint RE-REALIZED between the seal and the
    /// effect. Preparation sealed the lease against the first read; the next
    /// read inside the same pass sees a different realization published for
    /// the same relationship row, which is a self-consistent observation of
    /// the new one. Only the sealed lease compares the endpoint's own
    /// realization against the one it sealed, so the launch effect is never
    /// issued over a grant the endpoint no longer holds.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_re_realized_endpoint_between_seal_and_effect_refuses_the_launch() {
        let manager = delivered_manager();
        manager
            .script(vec![
                endpoint_view(AUTHORIZATION, CONSUMER),
                endpoint_view_at(2, "incarnation-OTHER-REALIZATION", AUTHORIZATION, CONSUMER),
            ])
            .await;
        let mut h = harness(process_row(Some(OWNER_UID)), manager).await;

        assert_eq!(
            h.driver
                .reconcile(&mut h.ctx)
                .await
                .expect("the pass reports an outcome"),
            ReconcileOutcome::RetryScheduled,
            "an endpoint that re-realized issues no effect"
        );
        settle().await;
        assert!(h.effects.try_recv().is_err(), "no launch effect was issued");
        assert!(
            !h.requeue.scheduled().is_empty(),
            "the pass re-reads the evidence instead of launching"
        );
    }

    /// Scenario 8 / AE14: the endpoint row generation moved between the seal
    /// and the effect, so the expectation the lease carries was derived from
    /// a row that no longer stands. The realization and the delivery
    /// projection are unchanged, which is what makes this the one movement
    /// the relationship-side facts cannot see.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_moved_endpoint_row_generation_between_seal_and_effect_refuses_the_launch() {
        let manager = delivered_manager();
        manager
            .script(vec![
                endpoint_view(AUTHORIZATION, CONSUMER),
                endpoint_view_at(7, INCARNATION, AUTHORIZATION, CONSUMER),
            ])
            .await;
        let mut h = harness(process_row(Some(OWNER_UID)), manager).await;

        assert_eq!(
            h.driver
                .reconcile(&mut h.ctx)
                .await
                .expect("the pass reports an outcome"),
            ReconcileOutcome::RetryScheduled,
            "a moved endpoint row generation issues no effect"
        );
        settle().await;
        assert!(h.effects.try_recv().is_err(), "no launch effect was issued");
        assert!(
            !h.requeue.scheduled().is_empty(),
            "the pass re-reads the evidence instead of launching"
        );
    }

    /// A required relationship the manager has not shown is the ordinary
    /// not-yet: the launch defers, requeues, and issues nothing.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_missing_relationship_row_defers_the_launch() {
        let manager = BindingManager::new(
            vec![endpoint_row()],
            vec![endpoint_view(AUTHORIZATION, CONSUMER)],
        );
        let mut h = harness(process_row(Some(OWNER_UID)), manager).await;

        assert_eq!(
            h.driver
                .reconcile(&mut h.ctx)
                .await
                .expect("the pass reports an outcome"),
            ReconcileOutcome::RetryScheduled
        );
        settle().await;
        assert!(h.effects.try_recv().is_err(), "no launch effect was issued");
    }

    /// A delivery projection that was published and cannot be read is REFUSED,
    /// not deferred: retrying the same unreadable evidence cannot change it.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn an_unreadable_delivery_projection_is_refused() {
        let manager = BindingManager::new(
            vec![endpoint_row()],
            vec![
                endpoint_view(AUTHORIZATION, CONSUMER),
                // Published, `delivered`, and missing the row generation and
                // the incarnation a delivery has to name.
                binding_view(3, serde_json::json!({ "state": "delivered" })),
            ],
        );
        let mut h = harness(process_row(Some(OWNER_UID)), manager).await;

        let failure = h
            .driver
            .reconcile(&mut h.ctx)
            .await
            .expect_err("the pass refuses");
        assert!(
            failure
                .wire_layer()
                .to_string()
                .contains("process-binding-gate-evidence-unreadable"),
            "the refusal carries the closed gate slug, got {}",
            failure.wire_layer()
        );
        settle().await;
        assert!(h.effects.try_recv().is_err(), "no launch effect was issued");
    }

    /// Scenario 9 / R21: a delivery downgrade AFTER the row was delivered keeps
    /// the relationship on `Ready`, so only the projection change can wake this
    /// row. The driver registers that watch on the exact dependency rows it
    /// read, and the pass the watch triggers refuses to launch over evidence
    /// that downgraded.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_projection_downgrade_wakes_the_row_and_blocks_the_launch() {
        let manager = delivered_manager();
        let mut h = harness(process_row(Some(OWNER_UID)), manager.clone()).await;

        assert!(
            matches!(
                h.driver
                    .reconcile(&mut h.ctx)
                    .await
                    .expect("the pass reports an outcome"),
                ReconcileOutcome::InProgress { .. }
            ),
            "a delivered relationship admits the launch"
        );
        settle().await;
        assert!(h.effects.try_recv().is_ok(), "the launch effect ran");

        let watches = manager.watches().await;
        for target in [endpoint_key(), binding_key()] {
            assert!(
                watches.iter().any(|(key, condition)| *key == target
                    && *condition == WatchCondition::ProjectionChanged),
                "a projection change on {target:?} wakes this row; watched: {watches:?}"
            );
        }

        // The relationship's own actor republishes the SAME row generation as
        // `draining`: the readiness phase never moves, the evidence layer does.
        manager.republish(binding_view(3, draining())).await;
        assert_eq!(
            h.driver
                .reconcile(&mut h.ctx)
                .await
                .expect("the pass reports an outcome"),
            ReconcileOutcome::RetryScheduled,
            "the downgrade the watch delivered blocks the next launch"
        );
        settle().await;
        assert!(h.effects.try_recv().is_err(), "no second launch effect ran");
    }

    /// Scenario 10 / AE7, the regression: a Process that requires no
    /// `EndpointBinding` behaves exactly as it did before the gate existed. The
    /// empty expected set is a real answer, not a fallback.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_process_with_no_required_binding_launches_exactly_as_before() {
        let manager = BindingManager::new(Vec::new(), Vec::new());
        let mut h = harness(process_row(Some(OWNER_UID)), manager).await;

        assert!(
            matches!(
                h.driver
                    .reconcile(&mut h.ctx)
                    .await
                    .expect("the pass reports an outcome"),
                ReconcileOutcome::InProgress { .. }
            ),
            "an ungated row still launches"
        );
        settle().await;
        assert!(
            matches!(h.effects.try_recv(), Ok(EffectCompleted { .. })),
            "the launch effect ran to completion"
        );
        assert!(h.requeue.scheduled().is_empty());
    }

    /// A root Process has no owner and therefore no siblings: it mints no
    /// expectation, reads nothing, and launches exactly as the row above.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_root_process_requires_no_binding() {
        let manager = BindingManager::new(Vec::new(), Vec::new());
        let mut h = harness(process_row(None), manager).await;

        assert!(
            matches!(
                h.driver
                    .reconcile(&mut h.ctx)
                    .await
                    .expect("the pass reports an outcome"),
                ReconcileOutcome::InProgress { .. }
            ),
            "a root row still launches"
        );
        settle().await;
        assert!(h.effects.try_recv().is_ok(), "the launch effect ran");
    }
}
