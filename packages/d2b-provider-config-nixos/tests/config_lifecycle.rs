use base64::{Engine as _, engine::general_purpose::STANDARD};
use d2b_contracts_resource::v3::{ResourceRef, ZoneId};
use d2b_provider_config_nixos::{
    ConfigApproveRequest, ConfigCaller, ConfigDiffRequest, ConfigRejectRequest, ConfigService,
    ConfigServiceBackend, ConfigStageRequest, ConfigStagingStore, ConfigStatusRequest,
    ConfigSyncRequest, ConfigSyncResponse, GuestConfigDocument, GuestConfigReader,
    GuestSessionEvidence, MAX_CONFIG_BYTES,
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
