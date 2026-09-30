//! Cross-Zone semantic sharing under the resource graph (U29, KTD14; R7,
//! R15-R16, R35-R41; AE11, AE30).
//!
//! A `ResourceImport` is a lease over the owner Zone's admitted semantic
//! service, not a copy of what that service backs. The four cases below drive
//! the decisions that keep it that way and show that each refusal is a
//! refusal, not a downstream no-op:
//!
//! 1. **AE11.** A consumer request that names a remote `Device`, `Volume`, or
//!    `Credential` directly is refused by the family catalog, before the
//!    export's policy or lease is consulted. An over-ceiling capability is
//!    refused by name too, rather than silently dropped from the admitted set.
//! 2. **AE30.** An imported USB service cannot mint a local physical-device
//!    binding: the USB projection schema carries no backing field, the
//!    imported backing set is empty, and a request that asks for a local
//!    device attachment is refused.
//! 3. **R36.** Revoking an export blocks new use immediately and drives
//!    outstanding lease use to its released state before the export may
//!    retire, using the same pre-drain ordering U8 added to the binding
//!    lifecycle.
//! 4. **AE11.** Neither re-exporting an import nor relaying it over a
//!    ZoneLink can launder the source Zone's authority.
//!
//! Everything here is contract-level: no provider implementation, no
//! transport, and no production entry point is exercised. U34 wires these
//! decisions into the plane; this suite is what it wires them to.

use d2b_contracts_provider::v3::provider::BindingTargetType;
use d2b_contracts_provider::v3::semantic_services::{
    ImportUseRefusal, ImportUseRequest, LocalPhysicalEffect, SemanticContractError, SemanticFamily,
    SemanticPairContract, catalog,
};
use d2b_contracts_resource::v3::SchemaFingerprint;
use d2b_contracts_resource::v3::execution_policy::{BoundedText, BoundedToken};
use d2b_contracts_resource::v3::resource::ResourceEnvelope;
use d2b_contracts_resource::v3::{ResourceName, ResourceRef, ResourceTypeName, ZoneId};
use d2b_contracts_zone_session::v3::component_session::{OperationClass, OperationId};
use d2b_contracts_zone_session::v3::zone_routing::ZoneLinkRouteAdmissionRequest;
use d2b_contracts_zone_session::v3::{
    ConsumerZonePolicy, ExportArbitration, ExportLeaseState, ExportLeaseSummary, ExportRevocation,
    ExportRevocationStage, ExportVisibility, ImportDisconnectPolicy, ImportLeaseClaim,
    ResourceExportContractError, ResourceExportSpec, ResourceImportContractError,
    ResourceImportSpec, RevocationPolicy, ShareQuota, ShareFairness,
};
use d2b_provider_resource_export::EXPORT_SUBJECT_TYPES;
use d2b_provider_resource_import::IMPORT_FORBIDDEN_LOCAL_TYPES;
use d2b_provider_zone_link::zone_links::{
    ZoneLinkCursor, ZoneLinkError, ZoneLinkKeyPolicy, ZoneLinkLimits, ZoneLinkRecord,
};
use d2b_provider_zone_link::zonelink::{
    ZoneLinkAdoption, ZoneLinkAdoptionError, ZoneLinkController, ZoneLinkControllerGeneration,
    ZoneLinkCursorRecord, ZoneLinkOwnerProof, ZoneLinkShareScope,
};
use d2b_resource_types::WellKnownType;
use d2b_contracts_resource::v3::identity::BindingDigest;

const CONSUMER_ZONE: &str = "guest-lab";
const USB_SERVICE: &str = "usb.d2bus.org.UsbService";
const USB_BINDING: &str = "usb.d2bus.org.UsbBinding";

fn pair() -> &'static SemanticPairContract {
    catalog()
        .iter()
        .find(|pair| pair.family() == SemanticFamily::Usb)
        .expect("the USB family is in the frozen semantic catalog")
}

fn service_type() -> ResourceTypeName {
    pair().service().resource_type().clone()
}

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("a bounded capability token")
}

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("a resource reference")
}

fn fingerprint(digit: char) -> SchemaFingerprint {
    SchemaFingerprint::parse(format!("sha256:{}", digit.to_string().repeat(64)))
        .expect("a domain-separated digest is a schema fingerprint")
}

fn export() -> ResourceExportSpec {
    ResourceExportSpec::new(
        reference(&format!("{USB_SERVICE}/dock")),
        service_type(),
        fingerprint('a'),
        fingerprint('b'),
        vec![token("list"), token("attach")],
        ExportArbitration::Exclusive,
        ShareQuota::new(Some(4), Some(64), ShareFairness::Weighted, Some(30_000))
            .expect("the quota is inside its frozen bounds"),
        ConsumerZonePolicy::new(vec![zone(CONSUMER_ZONE)], vec![token("list"), token("attach")])
            .expect("the consumer policy is canonical"),
        ExportVisibility::NamedZones,
        RevocationPolicy::new(5_000, true).expect("the grace period is inside its bounds"),
    )
    .expect("an export of a locally owned semantic Service is admitted")
}

fn import() -> ResourceImportSpec {
    ResourceImportSpec::new(
        reference("ZoneLink/lab-link"),
        BoundedText::parse("studio/dock".to_owned())
            .expect("the opaque export key is bounded"),
        service_type(),
        fingerprint('a'),
        fingerprint('b'),
        ResourceName::parse("dock").expect("a projection name"),
        vec![token("list")],
        ShareQuota::new(Some(4), Some(64), ShareFairness::Weighted, Some(30_000))
            .expect("the quota is inside its frozen bounds"),
        ImportDisconnectPolicy::Degrade,
    )
    .expect("an import of a qualified semantic Service is admitted")
}

fn zone(value: &str) -> ZoneId {
    ZoneId::parse(value).expect("a bounded zone id")
}

fn lease(capabilities: &[&str], state: ExportLeaseState) -> ImportLeaseClaim {
    ImportLeaseClaim::new(
        zone(CONSUMER_ZONE),
        capabilities.iter().copied().map(token).collect(),
        state,
    )
    .expect("a bounded, duplicate-free lease observation")
}

fn request(
    capabilities: &[&str],
    backing_refs: &[&str],
    local_effects: &[LocalPhysicalEffect],
) -> ImportUseRequest {
    ImportUseRequest::new(
        reference(&format!("{USB_BINDING}/kiosk")),
        reference(&format!("{USB_SERVICE}/dock")),
        BindingTargetType::Guest,
        capabilities.iter().copied().map(token).collect(),
        backing_refs.iter().copied().map(reference).collect(),
        local_effects.to_vec(),
    )
    .expect("a bounded, duplicate-free import-use request")
}

fn import_owner() -> ResourceRef {
    reference("ResourceImport/dock")
}

/// The projection row the importing Zone materialized for this export.
fn projection_owner() -> ResourceRef {
    import_owner()
}

/// AE11, part one: a direct backing-resource reference is refused by name.
///
/// The refusal comes out of the family catalog, before the export policy, the
/// quota, or the lease is consulted, and it names the attempt. A request that
/// also named a remote `Device`, `Volume`, or `Credential` would otherwise be
/// indistinguishable downstream from one that did not.
#[test]
fn a_direct_remote_backing_reference_is_refused_rather_than_ignored() {
    for backing in ["Device/usb-1", "Volume/media", "Credential/key"] {
        let refused = pair().admit_import_use(
            Some(&projection_owner()),
            &request(&["list"], &[backing], &[]),
        );
        let refusal = refused.expect_err("a backing reference is never admitted");
        assert_eq!(
            refusal,
            ImportUseRefusal::BackingReferenceForbidden,
            "{backing} must be refused, not narrowed away"
        );
        assert_eq!(refusal.as_str(), "semantic-import-backing-reference-forbidden");
    }

    // The same request without the backing reference is admitted, so the
    // refusal above is the backing reference and nothing else.
    assert!(
        pair()
            .admit_import_use(Some(&projection_owner()), &request(&["list"], &[], &[]))
            .is_ok()
    );
}

/// AE11, part two: only the exporting Zone's own Service is an export subject.
///
/// Every primitive source in the plane's closed vocabulary is refused as an
/// export target, and the provider crates' own subject vocabulary is empty, so
/// this conversion cannot name one even by accident.
#[test]
fn a_primitive_resource_is_not_an_export_subject() {
    let usb = pair();
    for resource_type in IMPORT_FORBIDDEN_LOCAL_TYPES {
        let name = resource_type.to_resource_type_name();
        assert_eq!(
            usb.admit_export_target(&reference(&format!("{name}/source"))),
            Err(SemanticContractError::WrongResourceType),
            "{name} must never be an export subject"
        );
        assert!(
            !EXPORT_SUBJECT_TYPES.contains(&resource_type),
            "{name} must not be reachable as an export subject"
        );
    }

    // The family-owned Service is the one admitted subject.
    assert!(usb.admit_export_target(&reference(&format!("{USB_SERVICE}/dock"))).is_ok());
}

/// AE11, part three: an over-ceiling or over-lease import is refused.
///
/// The importing Zone may not use more than the exporting Zone's consumer
/// policy, capability ceiling, and lease allow. Each of those is refused by
/// its own reason rather than quietly trimmed, so a request for more than it
/// was granted never looks admitted.
#[test]
fn a_capability_outside_the_export_ceiling_policy_or_lease_is_refused() {
    let export = export();
    let import = import();

    // Outside the exporting Zone's consumer policy.
    let stranger = ImportLeaseClaim::new(
        zone("unlisted-lab"),
        vec![token("list")],
        ExportLeaseState::Active,
    )
    .expect("a bounded lease observation");
    assert_eq!(
        import.admit_consumer_use(
            pair(),
            &export,
            &stranger,
            Some(&projection_owner()),
            &request(&["list"], &[], &[]),
        ),
        Err(ResourceImportContractError::ConsumerZoneNotAdmitted)
    );

    // Outside the export's advertised operations, and outside what this
    // import declared it wanted.
    assert_eq!(
        import.admit_consumer_use(
            pair(),
            &export,
            &lease(&["list", "attach"], ExportLeaseState::Active),
            Some(&projection_owner()),
            &request(&["attach"], &[], &[]),
        ),
        Err(ResourceImportContractError::CapabilityNotLeased)
    );

    // Inside the export's ceiling but outside the lease the owner Zone
    // granted this consumer.
    assert_eq!(
        import.admit_consumer_use(
            pair(),
            &export,
            &lease(&["list"], ExportLeaseState::Active),
            Some(&projection_owner()),
            &request(&["list", "attach"], &[], &[]),
        ),
        Err(ResourceImportContractError::CapabilityNotLeased)
    );

    // The granted use is admitted and carries exactly what the lease granted.
    let admitted = import
        .admit_consumer_use(
            pair(),
            &export,
            &lease(&["list"], ExportLeaseState::Active),
            Some(&projection_owner()),
            &request(&["list"], &[], &[]),
        )
        .expect("a leased, in-policy use is admitted");
    assert_eq!(admitted.family(), SemanticFamily::Usb);
    assert!(admitted.admits_capability(&token("list")));
    assert!(!admitted.admits_capability(&token("attach")));
}

/// AE30: an imported USB service cannot mint a local physical-device binding.
///
/// Three independent facts hold together here. The USB projection schema
/// carries no backing-reference field, the backing set an imported projection
/// may name is empty, and a consumer request that asks for a local physical
/// effect is refused. None of them is a check the importing Zone can skip by
/// naming things differently.
#[test]
fn an_imported_usb_service_cannot_mint_a_local_physical_device_binding() {
    let usb = pair();
    let projection = usb.projection();

    assert!(
        projection.backing_field_names().next().is_some(),
        "the USB family still declares its same-Zone backing Device reference"
    );
    assert_eq!(
        projection.projection_backing_field(),
        None,
        "the USB projection-mode prohibition must survive the conversion"
    );
    for family in catalog() {
        assert!(
            family
                .projection()
                .imported_backing_ref_types()
                .is_empty(),
            "the {:?} family gave an imported projection a backing reference type",
            family.family()
        );
    }
    assert!(
        projection.imported_backing_ref_types().is_empty(),
        "an imported projection has no backing reference type to name"
    );
    assert!(
        projection
            .allowed_backing_ref_types()
            .contains(&ResourceTypeName::parse("Device").expect("a Device is a ResourceType")),
        "the owner Zone still declares its own backing Device"
    );

    for effect in LocalPhysicalEffect::ALL {
        assert_eq!(
            usb.admit_import_use(
                Some(&projection_owner()),
                &request(&["list"], &[], &[effect]),
            )
            .unwrap_err(),
            ImportUseRefusal::LocalPhysicalEffectForbidden,
        );
    }

    // A locally owned Service of the same type is not an import: admitting it
    // here would let a consumer reach a local resource through the cross-Zone
    // vocabulary, and it is refused.
    assert_eq!(
        usb.admit_import_use(None, &request(&["list"], &[], &[]))
            .unwrap_err(),
        ImportUseRefusal::NotAnImportedProjection
    );
}

/// R36: revoking an export blocks new use at once and drains outstanding lease
/// use before the export may retire.
///
/// The revocation begins fenced, so "new use" stops being possible before
/// anything else happens; the advertisement is only withdrawn once no lease is
/// still active or draining; and the export row retires only after that.
#[test]
fn revoking_an_export_fences_new_use_and_drains_lease_use_first() {
    let export = export();
    let mut revocation = ExportRevocation::begin(&export);
    assert!(revocation.blocks_new_use());
    assert_eq!(revocation.stage(), ExportRevocationStage::PreDrain);

    let active = ExportLeaseSummary::new(
        zone(CONSUMER_ZONE),
        1,
        ExportLeaseState::Active,
        digest(0x11),
    );
    let draining = ExportLeaseSummary::new(
        zone("other-lab"),
        1,
        ExportLeaseState::Revoking,
        digest(0x22),
    );
    assert_eq!(
        revocation.observe_leases(std::slice::from_ref(&active)),
        ExportRevocationStage::PreDrain,
        "an active lease keeps the export open"
    );
    assert_eq!(revocation.outstanding_leases(), 1);
    assert_eq!(
        revocation.observe_leases(&[active.clone(), draining]),
        ExportRevocationStage::PreDrain,
        "a draining lease still holds the export open"
    );
    assert_eq!(revocation.outstanding_leases(), 2);
    assert!(!revocation.is_complete());

    let released = ExportLeaseSummary::new(
        zone(CONSUMER_ZONE),
        1,
        ExportLeaseState::Revoked,
        digest(0x11),
    );
    let never_active = ExportLeaseSummary::new(
        zone("other-lab"),
        0,
        ExportLeaseState::Pending,
        digest(0x22),
    );
    assert_eq!(
        revocation.observe_leases(&[released.clone(), never_active.clone()]),
        ExportRevocationStage::AdvertisementWithdrawn,
        "a released or never-active lease does not hold the export open"
    );
    assert_eq!(revocation.outstanding_leases(), 0);
    assert_eq!(
        revocation.observe_leases(&[released, never_active]),
        ExportRevocationStage::Complete
    );
    assert!(revocation.is_complete());

    // Forcing is a controller decision the policy has to have asked for.
    assert!(!revocation.force_permitted(4_999));
    assert!(revocation.force_permitted(5_000));
}

/// R36, continued: a lease that is draining or revoked admits no new use, so
/// a consumer that reconnects after a revocation cannot pick up where it left
/// off.
#[test]
fn a_revoked_or_draining_lease_admits_no_new_use() {
    for state in [ExportLeaseState::Revoking, ExportLeaseState::Revoked] {
        assert!(
            !lease(&["list"], state).admits_new_use(),
            "{state:?} must refuse new use"
        );
        assert_eq!(
            import().admit_consumer_use(
                pair(),
                &export(),
                &lease(&["list"], state),
                Some(&projection_owner()),
                &request(&["list"], &[], &[]),
            ),
            Err(ResourceImportContractError::LeaseNotUsable)
        );
    }
}

/// AE11: re-exporting an import is refused against the stored row.
///
/// An import-owned projection is a lease over a remote Service. The exporting
/// Zone owns that Service; the consumer does not, so the consumer cannot
/// advertise it onward. The check is against the stored row's owner, not
/// against the type name the author typed.
#[test]
fn an_import_owned_projection_is_never_re_exportable() {
    let export = export();
    let subject = stored_row(Some("ResourceImport/dock"));
    assert_eq!(
        export.admits_export_subject_origin(&subject),
        Err(ResourceExportContractError::ImportOwnedOriginRejected)
    );
    assert_eq!(
        ResourceExportContractError::ImportOwnedOriginRejected.code(),
        "resource-export-import-owned-origin-rejected"
    );

    // A locally owned Service of the same type is a valid subject.
    assert!(
        export
            .admits_export_subject_origin(&stored_row(None))
            .is_ok()
    );
}

/// AE11: relaying a share cannot launder it either.
///
/// A link fixed as a consumer carries only the use it leased. It cannot relay
/// the share to a third Zone, re-advertise the export, attach to a remote
/// resource, or hand the share over in a support bundle - and a reconnect
/// does not change which side of the share the link is on.
#[test]
fn a_consumer_link_cannot_relay_or_re_advertise_the_share() {
    let proof = owner_proof(1);
    let mut controller = ZoneLinkController::restore_in_share_scope(
        ZoneLinkLimits::default(),
        ZoneLinkKeyPolicy::default(),
        ZoneLinkRecord::unenrolled(ZoneLinkControllerGeneration::parse("1").expect("a bounded controller generation")),
        proof.clone(),
        Some(ZoneLinkShareScope::Consumer),
    );
    assert!(
        controller
            .adopt_cursor([ZoneLinkCursorRecord::in_share_scope(
                proof.clone(),
                ZoneLinkShareScope::Consumer,
                ZoneLinkCursor::default(),
            )])
            .is_adopted()
    );

    // `Attach` is not even constructible as a route request: the committed
    // route contract refuses it before any link state is consulted.
    assert!(ZoneLinkRouteAdmissionRequest::new(
        OperationId::new(vec![0x43; 16]).expect("an operation id"),
        OperationClass::Attach,
    )
    .is_err());

    for verb in [
        OperationClass::Relay,
        OperationClass::AuditExport,
        OperationClass::SupportBundle,
    ] {
        assert_eq!(
            controller.issue_route_admission(route_request(verb), Ok),
            Err(ZoneLinkError::ShareScopeRefusesVerb),
            "{verb:?} must be refused on a consumer link"
        );
    }

    // Using the leased share is what the link is for: the scope is not the
    // obstacle for an invocation. (This link is still unenrolled, so the
    // handler refuses it later, for a reason of its own.)
    let consumer_invoke = controller
        .issue_route_admission(route_request(OperationClass::Invoke), |request| {
            Ok(request)
        })
        .expect_err("an unenrolled link issues no route admission");
    assert_ne!(consumer_invoke, ZoneLinkError::ShareScopeRefusesVerb);

    // The same relay verb on an owner-scoped link is not refused by the scope
    // either, which is what makes the refusals above a statement about the
    // scope rather than about the link's session state.
    let mut owner = ZoneLinkController::restore_in_share_scope(
        ZoneLinkLimits::default(),
        ZoneLinkKeyPolicy::default(),
        ZoneLinkRecord::unenrolled(
            ZoneLinkControllerGeneration::parse("1").expect("a bounded controller generation"),
        ),
        proof.clone(),
        Some(ZoneLinkShareScope::Owner),
    );
    owner
        .adopt_cursor([ZoneLinkCursorRecord::in_share_scope(
            proof.clone(),
            ZoneLinkShareScope::Owner,
            ZoneLinkCursor::default(),
        )]);
    let owner_relay = owner
        .issue_route_admission(route_request(OperationClass::Relay), Ok)
        .expect_err("an unenrolled link issues no route admission");
    assert_ne!(owner_relay, ZoneLinkError::ShareScopeRefusesVerb);
    let owner_attach_probe = ZoneLinkRouteAdmissionRequest::new(
        OperationId::new(vec![0x44; 16]).expect("an operation id"),
        OperationClass::Connect,
    )
    .expect("connect is a committable route verb");
    assert_ne!(
        owner
            .issue_route_admission(owner_attach_probe, Ok)
            .expect_err("an unenrolled link issues no route admission"),
        ZoneLinkError::ShareScopeRefusesVerb
    );

    // A reconnect re-establishes the transport, never the authority.
    assert_eq!(
        ZoneLinkShareScope::Consumer.resumed_after_reconnect(),
        ZoneLinkShareScope::Consumer
    );

    // A restarted cursor cannot reclassify the link's side of the share
    // either: an observation carrying the other scope is quarantined.
    assert_eq!(
        controller.adopt_cursor([ZoneLinkCursorRecord::in_share_scope(
            proof,
            ZoneLinkShareScope::Owner,
            ZoneLinkCursor::default(),
        )]),
        ZoneLinkAdoption::Quarantined(ZoneLinkAdoptionError::ShareScopeMismatch)
    );
}

/// Every primitive source stays outside what an import may materialize, and
/// the plane's closed type vocabulary is still the one that list was derived
/// from, so the refusal cannot go stale against a rename.
#[test]
fn the_import_refusal_list_tracks_the_closed_type_vocabulary() {
    for resource_type in IMPORT_FORBIDDEN_LOCAL_TYPES {
        assert!(
            WellKnownType::ALL.contains(&resource_type),
            "{resource_type:?} left the closed vocabulary without a replacement"
        );
        assert!(
            d2b_provider_resource_export::EXPORT_SUBJECT_TYPES
                .iter()
                .all(|subject| subject != &resource_type),
            "{resource_type:?} became reachable as an export subject"
        );
    }
}

fn digest(byte: u8) -> BindingDigest {
    BindingDigest::parse(format!("sha256:{}", format!("{byte:02x}").repeat(32)))
        .expect("a domain-separated digest is a binding digest")
}

fn owner_proof(generation: u64) -> ZoneLinkOwnerProof {
    ZoneLinkOwnerProof::new(generation, fingerprint('c')).expect("an owner proof")
}

fn route_request(verb: OperationClass) -> ZoneLinkRouteAdmissionRequest {
    ZoneLinkRouteAdmissionRequest::new(
        OperationId::new(vec![0x42; 16]).expect("an operation id"),
        verb,
    )
    .expect("attach is not a committable route verb")
}

/// One stored row, optionally owned by an import.
fn stored_row(owner_ref: Option<&str>) -> ResourceEnvelope {
    let owner_ref = owner_ref
        .map(|owner| format!("\"{owner}\""))
        .unwrap_or_else(|| "null".to_owned());
    let json = r#"{
        "apiVersion": "resources.d2bus.org/v3",
        "type": "__TYPE__",
        "metadata": {
            "name": "dock",
            "zone": "studio",
            "uid": "123e4567-e89b-42d3-a456-426614174000",
            "generation": 1,
            "revision": 1,
            "ownerRef": __OWNER__,
            "finalizers": [],
            "deletionRequestedAt": null,
            "createdAt": "2026-07-22T00:00:00.000Z",
            "updatedAt": "2026-07-22T00:00:00.000Z",
            "managedBy": "controller",
            "configurationGeneration": null,
            "controllerGeneration": null,
            "providerGeneration": null
        },
        "spec": {},
        "status": {
            "completedAt": null,
            "conditions": [],
            "lastReconciledAt": null,
            "observedGeneration": 0,
            "outcome": null,
            "phase": "Pending",
            "resource": {},
            "startedAt": null,
            "update": {
                "dependencies": {"count": 0, "refs": []},
                "disruption": "None",
                "lastAssessedAt": null,
                "observedGeneration": 0,
                "operationId": null,
                "owned": {"count": 0, "refs": []},
                "preserveState": true,
                "reasons": [],
                "state": "Unknown",
                "targetGeneration": 1
            }
        }
    }"#
    .replace("__TYPE__", USB_SERVICE)
    .replace("__OWNER__", &owner_ref);
    ResourceEnvelope::from_json(json.as_bytes()).expect("a valid stored resource envelope")
}
