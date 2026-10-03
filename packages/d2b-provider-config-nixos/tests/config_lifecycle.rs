use base64::{Engine as _, engine::general_purpose::STANDARD};
use d2b_contracts_resource::v3::{
    AdmissionStage, BoundedToken, BindingArbitration, BindingAuthorization, BindingKey,
    BindingKind, BindingRealizationFacet, BindingSlot, DesiredDigest, DesiredRevision,
    FreshnessTuple, RefusalReason, ResourceRef, ResourceUid, SourceAdmission, StoreIncarnation,
    VolumeBindingRequest, ZoneId, volume::AttachmentAccess,
};
use d2b_provider_config_nixos::{
    CONFIG_WORKING_COPY_FACETS, ConfigApproveRequest, ConfigAttachment, ConfigCaller,
    ConfigDiffRequest, ConfigPublication, ConfigRejectRequest, ConfigService, ConfigServiceBackend,
    ConfigStageRequest, ConfigStagingStore, ConfigStatusRequest, ConfigSyncRequest,
    ConfigSyncResponse, GuestConfigDocument, GuestConfigReader, GuestSessionEvidence,
    MAX_CONFIG_BYTES, config_working_copy_support,
};

#[test]
fn guest_read_requires_current_matching_session() {
    let guest = ResourceRef::parse("Guest/work").expect("guest ref");
    let request = ConfigSyncRequest::new(guest.clone()).expect("request");
    let evidence = GuestSessionEvidence::new(guest, "boot-commitment", 1).expect("evidence");
    let result = ConfigService
        .read_guest_config(ConfigCaller::Guest, &request, &evidence, b"{}")
        .expect("read");
    assert_eq!(result.identifier, "guest-config");
    assert_eq!(result.bytes, 2);
    assert!(result.document().is_ok());
}

// ---------------------------------------------------------------------------
// Scenario 1: an invalid config cannot publish a self-grant or bypass an
// unsupported provider facet.
// ---------------------------------------------------------------------------

fn source_uid() -> ResourceUid {
    ResourceUid::from_bytes(&[0x31; 16]).expect("canonical source uid")
}

fn consumer_uid() -> ResourceUid {
    ResourceUid::from_bytes(&[0x32; 16]).expect("canonical consumer uid")
}

fn slot() -> BindingSlot {
    BindingSlot::parse("guest-config").expect("bounded slot")
}

fn view() -> BoundedToken {
    BoundedToken::parse("guest-config").expect("bounded view token")
}

fn volume() -> ResourceRef {
    ResourceRef::parse("Volume/work-config").expect("canonical Volume")
}

fn guest() -> ResourceRef {
    ResourceRef::parse("Guest/work").expect("canonical Guest")
}

/// The canonical declaration configuration publishes for one Guest.
fn attachment() -> ConfigAttachment {
    ConfigAttachment::new(
        volume(),
        guest(),
        slot(),
        view(),
        AttachmentAccess::ReadOnly,
        "/run/d2b/guest/guest-config.nix",
    )
    .expect("the typed declaration is constructible")
}

fn binding_key() -> BindingKey {
    BindingKey::new(
        zone(),
        BindingKind::Volume,
        volume(),
        source_uid(),
        guest(),
        consumer_uid(),
        slot(),
    )
    .expect("the declaration is an admitted consumer for its kind")
}

fn freshness() -> FreshnessTuple {
    FreshnessTuple::new(
        zone(),
        StoreIncarnation::parse("config-store").expect("bounded incarnation"),
        volume(),
        source_uid(),
        DesiredRevision::INITIAL,
        DesiredDigest::of(b"{\"view\":\"guest-config\"}"),
    )
}

fn source_admission(rights: Vec<d2b_contracts_resource::v3::RequestedRights>) -> SourceAdmission {
    SourceAdmission::new(binding_key(), rights, BindingArbitration::Shared)
        .expect("the source decision is scoped to the exact relationship")
}

fn granted() -> BindingAuthorization {
    BindingAuthorization::granted()
}

fn absent() -> BindingAuthorization {
    BindingAuthorization::absent()
}

fn observe() -> d2b_contracts_resource::v3::RequestedRights {
    d2b_contracts_resource::v3::RequestedRights::Observe
}

/// An invalid config cannot authorize its own publication: without the Role
/// evaluation's grant, the typed contract refuses at the authorizing stage
/// and nothing is admitted.
#[test]
fn a_config_cannot_publish_without_the_admission_grant() {
    let refusal = ConfigService
        .publish(
            &attachment(),
            &ConfigPublication {
                zone: &zone(),
                source_uid: &source_uid(),
                consumer_uid: &consumer_uid(),
                authorization: &absent(),
                source: &source_admission(vec![observe()]),
                dependencies: &[freshness()],
            },
        )
        .expect_err("a config that presents no grant cannot publish");
    assert_eq!(refusal.stage(), AdmissionStage::Authorize);
    assert_eq!(refusal.reason(), RefusalReason::IdentityNotAuthorized);
}

/// A source that does not admit the requested rights refuses at the admitting
/// stage, so an operator cannot widen the attachment by editing the document.
#[test]
fn a_config_cannot_publish_beyond_what_the_source_admits() {
    let refusal = ConfigService
        .publish(
            &attachment(),
            &ConfigPublication {
                zone: &zone(),
                source_uid: &source_uid(),
                consumer_uid: &consumer_uid(),
                authorization: &granted(),
                // The source admits a writer; the request asks to observe.
                source: &source_admission(vec![
                    d2b_contracts_resource::v3::RequestedRights::Mutate,
                ]),
                dependencies: &[freshness()],
            },
        )
        .expect_err("a right the source does not admit is refused");
    assert_eq!(refusal.stage(), AdmissionStage::Admit);
    assert_eq!(refusal.reason(), RefusalReason::SourcePolicyRefused);
}

/// A backend that declares no filesystem presentation cannot enforce the
/// configuration working copy, so the publication is refused at the prepare
/// stage rather than delivered over a weaker presentation.
#[test]
fn an_unsupported_provider_facet_refuses_publication() {
    let empty = d2b_contracts_resource::v3::BindingRealizationSupport::new(Vec::new())
        .expect("an empty support set is a valid declaration of nothing");
    let error = d2b_contracts_resource::v3::admit_binding_request(
        &binding_key(),
        observe(),
        &CONFIG_WORKING_COPY_FACETS,
        &granted(),
        &source_admission(vec![observe()]),
        &empty,
        &[freshness()],
    )
    .expect_err("a backend that realizes nothing cannot deliver the working copy");
    assert_eq!(error.stage(), AdmissionStage::Prepare);
    assert_eq!(error.reason(), RefusalReason::MandatoryFacetUnsupported);
}

/// The configuration Provider's declared support is fixed, so a caller
/// cannot widen it to make a facet this Provider never realizes look
/// admitted.
#[test]
fn the_configuration_support_set_is_not_caller_widened() {
    let support = config_working_copy_support();
    for facet in CONFIG_WORKING_COPY_FACETS {
        assert!(support.realizes(facet), "{facet:?} is the realized facet");
    }
    for unsupported in [
        BindingRealizationFacet::EndpointDescriptor,
        BindingRealizationFacet::SharedFabric,
        BindingRealizationFacet::CredentialDelivery,
        BindingRealizationFacet::ConsumerDeviceSlot,
    ] {
        assert!(
            !support.realizes(unsupported),
            "configuration delivery never realizes {unsupported:?}"
        );
    }
    assert_eq!(support.facets(), CONFIG_WORKING_COPY_FACETS.as_slice());
    assert_eq!(support.facets().len(), 1);
}

/// The declaration names exact typed references, never a host source path or
/// a numerical principal.
#[test]
fn the_declaration_states_exact_typed_references() {
    let declared = attachment();
    let request: &VolumeBindingRequest = declared.request();
    assert_eq!(request.source_ref(), &volume());
    assert_eq!(request.consumer_ref(), &guest());
    assert_eq!(request.kind(), BindingKind::Volume);
    assert_eq!(request.slot(), &slot());
    assert_eq!(declared.requested_rights(), observe());
    assert_eq!(declared.required_facets(), CONFIG_WORKING_COPY_FACETS);
    // The rendered request names the source and consumer by reference and
    // carries the presentation's destination inside the consumer. No host
    // source path is authored: the fields are exactly the closed request set.
    let rendered = serde_json::to_value(request).expect("the request serializes");
    let object = rendered.as_object().expect("the request is an object");
    let mut fields = object.keys().map(String::as_str).collect::<Vec<_>>();
    fields.sort_unstable();
    assert_eq!(
        fields,
        vec![
            "access",
            "consumerRef",
            "presentation",
            "slot",
            "sourceRef",
            "view",
        ],
        "the declared field set is closed: {object:?}"
    );
    assert_eq!(object["sourceRef"], "Volume/work-config");
    assert_eq!(object["consumerRef"], "Guest/work");
    assert_eq!(
        object["presentation"]["destination"],
        "/run/d2b/guest/guest-config.nix",
        "the destination is inside the consumer, not a host source path"
    );
    for forbidden in ["sourcePath", "hostPath", "path", "uid", "gid", "principal"] {
        assert!(
            !object.contains_key(forbidden),
            "the request authored a {forbidden}: {object:?}"
        );
    }
}

/// A relationship the source decision is scoped to cannot be substituted for
/// a different one: the admission is refused rather than re-pointed.
#[test]
fn a_config_cannot_publish_through_a_foreign_source_decision() {
    let foreign_key = BindingKey::new(
        zone(),
        BindingKind::Volume,
        volume(),
        source_uid(),
        ResourceRef::parse("Guest/other").expect("canonical Guest"),
        consumer_uid(),
        slot(),
    )
    .expect("the foreign pair is structurally valid");
    let foreign = SourceAdmission::new(foreign_key, vec![observe()], BindingArbitration::Shared)
        .expect("the foreign decision is constructible");
    let refusal = ConfigService
        .publish(
            &attachment(),
            &ConfigPublication {
                zone: &zone(),
                source_uid: &source_uid(),
                consumer_uid: &consumer_uid(),
                authorization: &granted(),
                source: &foreign,
                dependencies: &[freshness()],
            },
        )
        .expect_err("a decision scoped to another relationship is refused");
    assert_eq!(refusal.stage(), AdmissionStage::Admit);
    assert_eq!(refusal.reason(), RefusalReason::SourcePolicyRefused);
}

// ---------------------------------------------------------------------------
// Scenario 3: the diagnostic names the resource and the enforcing stage and
// leaks no private path or secret.
// ---------------------------------------------------------------------------

/// A refusal names the relationship and the enforcing stage, and carries
/// neither the document, the destination, nor a session identity.
#[test]
fn a_config_refusal_names_the_relationship_and_the_stage() {
    let declared = attachment();
    let refusal = ConfigService
        .publish(
            &declared,
            &ConfigPublication {
            zone: &zone(),
            source_uid: &source_uid(),
            consumer_uid: &consumer_uid(),
            authorization: &absent(),
            source: &source_admission(vec![observe()]),
            dependencies: &[freshness()],
            },
        )
        .expect_err("no grant means no publication");
    assert_eq!(
        refusal.attachment().source_ref(),
        &volume(),
        "the refusal names the exact source"
    );
    assert_eq!(
        refusal.attachment().consumer_ref(),
        &guest(),
        "the refusal names the exact consumer"
    );
    assert_eq!(refusal.stage(), AdmissionStage::Authorize);
    let rendered = refusal.to_string();
    assert!(rendered.contains("Volume/work-config"), "{rendered}");
    assert!(rendered.contains("Guest/work"), "{rendered}");
    assert!(rendered.contains("Authorize"), "{rendered}");
}

/// The refusal never carries the staged document, its digest, the
/// destination, or a private path.
#[test]
fn a_config_refusal_leaks_no_document_or_private_path() {
    let document = GuestConfigDocument::new(
        b"services.private.enable = true;\npassword = \"hunter2\";\n".to_vec(),
    )
    .expect("document");
    let digest = document.sha256();
    let refusal = ConfigService
        .publish(
            &attachment(),
            &ConfigPublication {
                zone: &zone(),
                source_uid: &source_uid(),
                consumer_uid: &consumer_uid(),
                authorization: &absent(),
                source: &source_admission(vec![observe()]),
                dependencies: &[freshness()],
            },
        )
        .expect_err("no grant means no publication");
    let rendered = format!("{refusal:?} {refusal}");
    for forbidden in [
        "hunter2",
        "password",
        "services.private",
        digest.as_str(),
        "/run/d2b/guest/guest-config.nix",
        "guest-config.nix",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "the refusal leaked {forbidden}: {rendered}"
        );
    }
}

/// The whole lifecycle still works: a well-formed config with a grant, a
/// matching source decision, and the declared support publishes.
#[test]
fn a_well_formed_config_publishes_through_admitted_activation() {
    let admission = ConfigService
        .publish(
            &attachment(),
            &ConfigPublication {
                zone: &zone(),
                source_uid: &source_uid(),
                consumer_uid: &consumer_uid(),
                authorization: &granted(),
                source: &source_admission(vec![observe()]),
                dependencies: &[freshness()],
            },
        )
        .expect("a well-formed, granted config publishes");
    assert_eq!(admission.key(), &binding_key());
    assert_eq!(admission.rights(), observe());
    assert!(
        admission.is_current(&[freshness()]),
        "the admission is current against the evidence it was evaluated against"
    );
    assert!(
        !admission.is_current(&[]),
        "a moved dependency does not leave the admission current"
    );
}

#[test]
fn sync_response_integrity_mismatches_fail_closed() {
    let guest = ResourceRef::parse("Guest/work").expect("guest ref");
    let document =
        GuestConfigDocument::new(b"services.foo.enable = true;\n".to_vec()).expect("document");
    let valid = ConfigSyncResponse {
        guest_ref: guest.clone(),
        identifier: "guest-config".to_owned(),
        content_base64: STANDARD.encode(document.bytes()),
        bytes: document.len(),
        sha256: document.sha256(),
    };
    assert!(valid.document().is_ok());

    let mut forged_digest = valid.clone();
    forged_digest.sha256 =
        "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_owned();
    assert_eq!(
        forged_digest
            .document()
            .expect_err("forged digest must fail")
            .code(),
        "config-document-encoding-failed"
    );

    let mut forged_bytes = valid.clone();
    forged_bytes.bytes += 1;
    assert_eq!(
        forged_bytes
            .document()
            .expect_err("wrong byte count must fail")
            .code(),
        "config-document-encoding-failed"
    );

    let mut over_bound = valid;
    over_bound.content_base64 = "A".repeat(MAX_CONFIG_BYTES.div_ceil(3) * 4 + 1);
    assert_eq!(
        over_bound
            .document()
            .expect_err("over-bound payload must fail")
            .code(),
        "config-request-invalid"
    );
}

fn zone() -> ZoneId {
    ZoneId::parse("work").expect("zone")
}

#[test]
fn stale_session_and_non_guest_callers_fail_closed() {
    let guest = ResourceRef::parse("Guest/work").expect("guest ref");
    let request = ConfigSyncRequest::new(guest.clone()).expect("request");
    let evidence = GuestSessionEvidence::new(guest, "boot-commitment", 1)
        .expect("evidence")
        .stale();
    assert_eq!(
        ConfigService
            .read_guest_config(ConfigCaller::Guest, &request, &evidence, b"{}")
            .expect_err("stale read")
            .code(),
        "config-session-stale"
    );
    assert_eq!(
        ConfigService
            .read_guest_config(
                ConfigCaller::Admin,
                &request,
                &GuestSessionEvidence::new(
                    ResourceRef::parse("Guest/work").expect("guest ref"),
                    "boot-commitment",
                    1
                )
                .expect("evidence"),
                b"{}"
            )
            .expect_err("admin read")
            .code(),
        "config-session-stale"
    );
}

#[test]
fn host_staging_lifecycle_is_typed_and_consumes_approved_content() {
    let guest = ResourceRef::parse("Guest/work").expect("guest ref");
    let document =
        GuestConfigDocument::new(b"services.foo.enable = true;\n".to_vec()).expect("document");
    let stage = ConfigStageRequest::new(guest.clone(), &document).expect("stage request");
    let mut store = ConfigStagingStore::default();

    let staged = store
        .stage(ConfigCaller::Admin, &zone(), &stage)
        .expect("stage content");
    assert_eq!(staged.bytes, document.len());
    assert_eq!(staged.sha256, document.sha256());

    let status = store
        .status(
            ConfigCaller::Admin,
            &zone(),
            &ConfigStatusRequest::new(guest.clone()).expect("status request"),
        )
        .expect("status");
    assert!(status.pending);
    assert_eq!(status.bytes, Some(document.len()));

    let diff = store
        .diff(
            ConfigCaller::Admin,
            &zone(),
            &ConfigDiffRequest::new(
                guest.clone(),
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            )
            .expect("diff request"),
        )
        .expect("diff");
    assert!(diff.differs);

    let approved = store
        .approve(
            ConfigCaller::Admin,
            &zone(),
            &ConfigApproveRequest::new(guest.clone(), "host-config").expect("approve request"),
        )
        .expect("approve");
    assert_eq!(approved.bytes, document.len());
    assert_eq!(approved.sha256, document.sha256());
    assert!(
        !store
            .status(
                ConfigCaller::Admin,
                &zone(),
                &ConfigStatusRequest::new(guest.clone()).expect("status request"),
            )
            .expect("status after approve")
            .pending
    );
}

#[test]
fn approval_retry_is_idempotent_after_downstream_publish_failure() {
    let guest = ResourceRef::parse("Guest/work").expect("guest ref");
    let document = GuestConfigDocument::new(b"true\n".to_vec()).expect("document");
    let mut store = ConfigStagingStore::default();
    store
        .stage(
            ConfigCaller::Admin,
            &zone(),
            &ConfigStageRequest::new(guest.clone(), &document).expect("stage request"),
        )
        .expect("stage");

    let request = ConfigApproveRequest::new(guest.clone(), "host-config").expect("approve request");
    let first = store
        .approve(ConfigCaller::Admin, &zone(), &request)
        .expect("first approval");
    // The host publish may fail after this receipt is recorded. Retrying the
    // same approval must not require the consumed staging bytes again.
    let retry = store
        .approve(ConfigCaller::Admin, &zone(), &request)
        .expect("retry approval");
    assert_eq!(retry, first);
    assert_eq!(
        store
            .approve(
                ConfigCaller::Admin,
                &zone(),
                &ConfigApproveRequest::new(guest.clone(), "other-target")
                    .expect("second destination request"),
            )
            .expect_err("different destination must not reuse approval")
            .code(),
        "config-approval-conflict"
    );
    assert!(
        store
            .reject(
                ConfigCaller::Admin,
                &zone(),
                &ConfigRejectRequest::new(guest).expect("reject request"),
            )
            .expect("reject approval receipt")
            .removed
    );
}

#[test]
fn staging_isolated_by_zone_for_same_guest_name() {
    let guest = ResourceRef::parse("Guest/work").expect("guest ref");
    let work_zone = zone();
    let personal_zone = ZoneId::parse("personal").expect("zone");
    let work_document = GuestConfigDocument::new(b"work = true\n".to_vec()).expect("document");
    let personal_document =
        GuestConfigDocument::new(b"personal = true\n".to_vec()).expect("document");
    let mut store = ConfigStagingStore::default();

    store
        .stage(
            ConfigCaller::Admin,
            &work_zone,
            &ConfigStageRequest::new(guest.clone(), &work_document).expect("stage request"),
        )
        .expect("work stage");
    store
        .stage(
            ConfigCaller::Admin,
            &personal_zone,
            &ConfigStageRequest::new(guest.clone(), &personal_document).expect("stage request"),
        )
        .expect("personal stage");

    assert_eq!(
        store
            .status(
                ConfigCaller::Admin,
                &work_zone,
                &ConfigStatusRequest::new(guest.clone()).expect("status request"),
            )
            .expect("work status")
            .sha256,
        Some(work_document.sha256())
    );
    assert_eq!(
        store
            .status(
                ConfigCaller::Admin,
                &personal_zone,
                &ConfigStatusRequest::new(guest).expect("status request"),
            )
            .expect("personal status")
            .sha256,
        Some(personal_document.sha256())
    );
}

#[test]
fn staging_rejects_paths_invalid_views_and_unauthorized_callers() {
    let guest = ResourceRef::parse("Guest/work").expect("guest ref");
    let document = GuestConfigDocument::new(b"true\n".to_vec()).expect("document");
    let mut store = ConfigStagingStore::default();
    let stage = ConfigStageRequest::new(guest.clone(), &document).expect("stage request");
    assert_eq!(
        store
            .stage(ConfigCaller::Guest, &zone(), &stage)
            .expect_err("guest must be denied")
            .code(),
        "config-unauthorized"
    );
    assert!(ConfigDiffRequest::new(guest.clone(), "/etc/host.nix").is_err());
    assert!(ConfigApproveRequest::new(guest.clone(), "/etc/host.nix").is_err());
    assert!(
        !store
            .reject(
                ConfigCaller::Admin,
                &zone(),
                &ConfigRejectRequest::new(guest).expect("reject request"),
            )
            .expect("reject empty store")
            .removed
    );
}

#[cfg(unix)]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn guest_reader_rejects_hardlinked_config_files() {
    let root = std::env::current_dir()
        .expect("test working directory")
        .join(".scratch")
        .join(format!("config-reader-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("scratch directory");
    let source = root.join("source.nix");
    let hardlink = root.join("guest-config.nix");
    std::fs::write(&source, b"true\n").expect("source");
    std::fs::hard_link(&source, &hardlink).expect("hardlink");

    let reader = GuestConfigReader::new(
        ResourceRef::parse("Guest/work").expect("guest ref"),
        "boot-commitment",
        1,
        &hardlink,
    )
    .expect("reader");
    let request = ConfigSyncRequest::new(ResourceRef::parse("Guest/work").expect("guest ref"))
        .expect("request");
    let error = reader
        .dispatch(
            d2b_provider_config_nixos::ConfigOperation::ReadGuestConfig,
            serde_json::to_value(request).expect("request JSON"),
        )
        .expect_err("hardlinked files must fail closed");
    assert_eq!(error.code(), "config-request-invalid");
    std::fs::remove_dir_all(root).expect("cleanup");
}
